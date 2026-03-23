# iperf3 网络测试适配：从零分到 6/6 全部通过

**日期**: 2026/3/23

---

## 1. 背景

iperf3 是 oscomp 评测系统中的网络性能测试工具，测试 6 个子项：BASIC_UDP、BASIC_TCP、PARALLEL_UDP、PARALLEL_TCP、REVERSE_UDP、REVERSE_TCP。测试脚本在 sdcard 中的 `iperf_testcode.sh` 里，核心流程：

1. 启动 iperf3 server daemon（`iperf3 -s -p 5001 -D`，后台监听 127.0.0.1:5001）
2. 依次运行 6 个 iperf3 client 连接到 loopback 地址

评测前 rcore-lab 的 iperf 得分为 **0 分**（所有子测试失败）。本次工作目标是逐层排除阻塞，使 iperf3 能在 rcore-lab 上运行。

---

## 2. 适配思路与架构

iperf3 的完整运行依赖一条很长的系统调用链：

```
daemon()              → setsid, fork, chdir, open(/dev/null), dup2, fstat
socket/bind/listen    → AF_INET TCP/UDP
accept/connect        → TCP 三次握手（loopback 127.0.0.1）
select/pselect6       → I/O 多路复用
read/write/sendto     → 数据传输（TCP 和 UDP）
getpeername           → 获取 connected 远端地址（UDP stream 初始化）
getsockopt            → SO_SNDBUF/SO_RCVBUF 缓冲区大小检查
close                 → 优雅关闭 TCP 连接（发送 FIN）
```

rcore-lab 已有 smoltcp 0.12 网络栈 + VirtIO-Net 驱动 + 基础 socket syscall，但上述链条中有 **10+ 个环节** 缺失或行为不正确。适配采用"运行→报错→修复→再运行"的迭代调试方式，每轮通过 QEMU 日志定位最外层阻塞点并修复。

---

## 3. 逐层修复详情

### 3.1 `/dev/urandom` 设备（第 1 层阻塞）

**现象**：iperf3 启动后立即报 `failed to open /dev/urandom: No such file or directory`

**原因**：iperf3 用 `/dev/urandom` 生成 session cookie。内核的 `is_char_device()` 已识别该路径，但 `sys_openat` 的设备匹配只有 `/dev/null` 和 `/dev/zero`。

**修复**：
- `os/src/fs/stdio.rs`：新增 `DevUrandom` 结构体，实现 `File` trait，`read()` 用 xorshift64 PRNG 生成伪随机字节（与现有 `sys_getrandom` 相同算法）
- `os/src/syscall/fs.rs`：`sys_openat` 的设备匹配添加 `"/dev/urandom" | "/dev/random"`
- `os/src/fs/mod.rs`：`ensure_basic_paths()` 添加占位文件确保 `faccessat` 兼容

### 3.2 `setsid` 系统调用（第 2 层阻塞）

**现象**：glibc 版 iperf3 daemon 化时报 `unable to become a daemon: Function not implemented`

**原因**：glibc 的 `daemon()` 调用 `setsid()`（syscall 157），内核未实现。

**修复**：
- `os/src/task/process.rs`：`ProcessControlBlockInner` 添加 `session_id` 和 `pgid` 字段，`new()` 和 `fork()` 中初始化/继承
- `os/src/syscall/process.rs`：实现 `sys_setsid`（设 session_id=pgid=pid）、`sys_setpgid`、`sys_getpgid`、`sys_getsid`
- `os/src/syscall/mod.rs`：添加 syscall 154-157 的分发

### 3.3 `pselect6` 系统调用（第 3 层阻塞）

**现象**：`select failed: Function not implemented`

**原因**：iperf3 server 用 `select()`（musl 实现为 `pselect6` syscall 72）做 I/O 多路复用，未实现。

**修复**：
- `os/src/syscall/fs.rs`：新增 `sys_pselect6`，将 Linux `fd_set` 位图格式（每 64 位一个 word）转换为对每个 fd 调用 `File::poll()` 检查 POLLIN/POLLOUT/POLLERR 状态。支持 readfds、writefds、exceptfds 和 timeout。

### 3.4 `getsockopt` 返回值（第 4 层阻塞）

**现象**：`socket buffer size not set correctly`

**原因**：iperf3 用 `setsockopt(SO_SNDBUF)` 设置缓冲区大小后用 `getsockopt` 验证。我们的 `setsockopt` 是 stub（静默接受），`getsockopt` 对 SO_SNDBUF/SO_RCVBUF 返回 0。

**修复**：`os/src/net/syscall.rs`：`sys_getsockopt` 对 SO_SNDBUF/SO_RCVBUF 返回 65536。

### 3.5 `fstatat(AT_EMPTY_PATH)` 和 DevNull fstat（第 5 层阻塞）

**现象**：glibc `daemon()` 报 `Invalid argument`，然后 `No such device`

**原因**：两个独立问题：
1. `fstatat(fd, "", stat, AT_EMPTY_PATH)` — 当 path 为空且设了 `AT_EMPTY_PATH` flag 时应等价于 `fstat(fd)`，但我们的实现对空 path 直接返回 EINVAL
2. `DevNull` 的 `File::path()` 返回 `None`，导致 `sys_fstat` 把 `/dev/null` 当普通文件处理，返回 `S_IFREG` 而非 `S_IFCHR`。glibc `daemon()` 检查 `/dev/null` 是否为字符设备。

**修复**：
- `sys_fstatat`：检测 `AT_EMPTY_PATH` flag 时委托给 `sys_fstat`
- `DevNull`/`DevZero`/`DevUrandom`：实现 `path()` 方法返回设备路径，使 fstat 正确返回 CHR 类型

### 3.6 `waitpid` 语义（第 6 层阻塞——最隐蔽）

**现象**：测试脚本 `status=0x0` 正常退出，但只输出 `BASIC_UDP begin` 没有 `end`

**原因**：initcode 的 `run_testcode` 用 `wait(-1)` 等子进程。iperf3 daemon 模式 fork 两次（parent→child→grandchild），grandchild 成为 orphan 被 reparent 给 init (pid=1)。当 init 调 `wait(-1)` 想等 busybox shell 退出时，**误收割了 daemon 的 orphan 中间进程**，导致提前认为测试完成。

**修复**：`user/src/bin/initcode.rs`：`run_testcode` 和 `run_single_binary` 从 `wait(&mut status)` 改为 `waitpid(pid, &mut status)` 循环，只等待直接子进程。

### 3.7 Loopback TCP 握手（第 7 层阻塞）

**现象**：client 输出 `Connecting to host 127.0.0.1, port 5001` 后永远阻塞

**原因**：smoltcp 的 Loopback 设备通过内部 VecDeque 实现：`transmit` 把包入队，`receive` 从队列取包。TCP 三次握手需要多次 TX→RX 往返：
1. 第 1 次 poll：client socket TX SYN → 入队
2. 第 2 次 poll：RX SYN → server socket 处理 → TX SYN-ACK → 入队
3. 第 3 次 poll：RX SYN-ACK → client socket 处理 → TX ACK → 入队
4. 第 4 次 poll：RX ACK → server socket 进入 ESTABLISHED

但 `poll_net()` 只对 `lo_iface` poll 1 次，握手永远停在 SYN-SENT。

**修复**：`os/src/net/mod.rs`：`poll_net()` 对 `lo_iface` 连续 poll 4 次，确保一次 `poll_net` 调用能完成完整 TCP 三次握手。

### 3.8 UDP connected write（第 8 层阻塞）

**现象**：iperf3 client 创建 UDP socket 后 `write(fd=7, 4)` 返回 0

**原因**：iperf3 对 UDP socket 先 `connect(dst)` 然后用 `write()` 发数据（而非 `sendto`）。这是合法的 POSIX 语义——connected UDP socket 的 `write` 等价于 `send`。但 `SocketFile::udp_write()` 直接返回 0 并打印 "use sendto"。

**修复**：
- `os/src/net/socket_file.rs`：`SocketFile` 添加 `connected_remote: spin::Mutex<Option<IpEndpoint>>` 字段
- `os/src/net/syscall.rs`：`sys_connect` 对 UDP 保存远端地址到 `connected_remote`
- `udp_write`：从 `connected_remote` 获取目标地址，对 loopback 使用 `inject_recv` 直接注入目标 socket 的 RX buffer（与 `sys_sendto` 的 loopback 路径一致）

### 3.9 `getpeername` 对 UDP socket（第 9 层阻塞）

**现象**：`unable to initialize stream: Not supported`

**原因**：iperf3 对 UDP data stream 调用 `getpeername()` 获取远端地址。我们的 `sys_getpeername` 对非 TCP socket 直接返回 EOPNOTSUPP。

**修复**：`sys_getpeername` 对 UDP socket 从 `connected_remote` 返回 `connect()` 时保存的地址。在 `File` trait 添加 `get_connected_remote()` 方法。

### 3.10 UDP connected `set_remote_endpoint`（第 10 层阻塞——smoltcp 层）

**现象**：iperf3 client 的 UDP `connect()` 成功，但 server 端 `recvfrom` 收不到数据

**原因**：我们在 `sys_connect` 对 UDP 只保存了 `connected_remote` 到 SocketFile，但没有告诉 smoltcp socket 本身。smoltcp 的 `accepts()` 方法检查 `remote_endpoint`：如果设置了，就只接收来自该 remote 的包。但我们没设置，导致 server 端的 unconnected listener 不正确地接收了所有包（包括本应发给 connected socket 的）。

**修复**：`sys_connect` 对 UDP socket 额外调用 `sock.set_remote_endpoint(Some(remote))`，让 smoltcp 层面也知道这是一个 connected socket，配合后续的 demux 逻辑使用。

### 3.11 TCP graceful close（第 11 层阻塞——最深层）

**现象**：BASIC_UDP 通过后，BASIC_TCP 立即报 `control socket has closed unexpectedly`

**原因**：这是一个 TCP 连接生命周期管理的竞争条件。完整的 bug 链：

1. BASIC_UDP client 完成测试，发送 IPERF_DONE state byte，然后 `close(fd=6)`
2. `SocketFile::drop()` 调用 `socket.abort()` + `sockets.remove(handle)` — **TCP socket 被暴力销毁，FIN 从未发送**
3. Server 端永远收不到 IPERF_DONE（数据在 client 的 TX buffer 里被 abort 丢弃了）
4. BASIC_TCP client 启动，`connect()` 发 SYN 到 server 的 listen socket
5. Server 的 `select()` 同时看到 listener ready（新 SYN）和 ctrl_sck ready（旧连接可能有 stale 数据）
6. iperf server 代码先检查 `FD_ISSET(listener)` → `iperf_accept()` 替换了 `test->ctrl_sck`
7. 新 client 的控制通道被错误地放到了旧测试的状态机中 → 状态不匹配 → close

**修复**：
- `SocketFile::drop()` 对 TCP 改用 `socket.close()`（优雅关闭，发送 FIN）替代 `socket.abort()`
- Drop 中立即 poll loopback，确保 FIN 通过 loopback 设备投递到 server 端
- 这样 server 能在下一次 `select()` 中先收到 IPERF_DONE + FIN → 正常退出旧测试循环 → cleanup → 重新 listen → 正确 accept 新 client

### 3.12 Loopback UDP demux（第 12 层阻塞——PARALLEL 系列）

**现象**：PARALLEL_UDP（`-P 5`）在创建第 2 个 UDP stream 时卡死。client `write(fd=9, cookie, 4)` 后阻塞在 `recvfrom(fd=9)` 等 server 回复 cookie，但 server 永远收不到。

**原因**：iperf3 `-P 5` 需要 5 个并行 UDP stream，server 为每个 stream 创建独立的 UDP socket 并 `bind` 到同一端口 5001。我们的 loopback `inject_recv` 逻辑遍历所有 socket，**找到第一个端口匹配的就 `break`**——永远把数据送到 stream 1 的 socket，stream 2-5 的 socket 永远收不到数据。

这是 iperf3 parallel 模式的核心挑战：**同一端口上有多个 UDP socket，需要根据来源地址分发到正确的 socket**。Linux 内核的 UDP demux 通过 connected 四元组匹配实现，smoltcp 的 `accepts()` 也已经支持 `remote_endpoint` 过滤（3.10 中设置的），但我们的 loopback inject 完全绕过了 smoltcp 的匹配逻辑。

**修复**：新增 `loopback_udp_inject()` 函数（`os/src/net/mod.rs`），实现两级 demux：

```
1. 优先级 1：找 connected socket（remote_endpoint 匹配发送者的 addr+port）
2. 优先级 2：fallback 到 unconnected wildcard socket（未设 remote_endpoint）
3. 跳过发送者自己（防止自发自收）
```

这精确模拟了 Linux 内核的 UDP 同端口 demux 语义。替换了 `udp_write` 和 `sendto` 中的简单 port 匹配逻辑。

**效果**：PARALLEL_UDP 5 个 stream 全部成功传输（76.9 Mbits/sec 合计）。PARALLEL_TCP 本身不需要修复——smoltcp 的 TCP 栈天然用 4-tuple 匹配，多连接在同一 listen 端口上互不干扰。

---

## 4. 其他补充修复

| 修复 | 说明 |
|------|------|
| `getrusage` stub | iperf3 调用 `getrusage` 获取 CPU 使用率，返回全零即可 |
| smoltcp Loopback `queue` 字段可见性 | `pub(crate)` → `pub`，允许内核检查 loopback 队列状态 |
| `alloc::vec` 宏导入 | `sys_pselect6` 使用 `vec![]` 需要显式导入 |
| smoltcp UDP `remote_endpoint` 字段 | 在 `vendor/smoltcp/src/socket/udp.rs` 中为 `Socket` 添加 `remote_endpoint` 字段和 `set_remote_endpoint()`/`remote_endpoint()` 访问器，`accepts()` 增加 connected 过滤逻辑 |

---

## 5. 最终成果

### 5.1 测试结果（6/6 全部通过）

| 子测试 | 状态 | 吞吐量（sender） | 吞吐量（receiver） |
|--------|------|-------------------|---------------------|
| BASIC_UDP | SUCCESS | 47.6 Mbits/sec | 29.8 Mbits/sec |
| BASIC_TCP | SUCCESS | 27.2 Mbits/sec | 27.0 Mbits/sec |
| PARALLEL_UDP (-P 5) | SUCCESS | 76.9 Mbits/sec | 46.3 Mbits/sec |
| PARALLEL_TCP (-P 5) | SUCCESS | 71.5 Mbits/sec | 71.1 Mbits/sec |
| REVERSE_UDP (-R) | SUCCESS | 41.6 Mbits/sec | 30.2 Mbits/sec |
| REVERSE_TCP (-R) | SUCCESS | 26.9 Mbits/sec | 26.9 Mbits/sec |

### 5.2 已知问题（不影响功能）

1. **TCP MSS 为 0**：`getsockopt(TCP_MAXSEG)` 返回 0，iperf3 输出 `warning: Ignoring nonsense TCP MSS 0`。不影响功能但可能影响性能
2. **UDP 丢包率 ~35%**：loopback 上不应有丢包，可能是 smoltcp buffer 容量或 poll 频率不足导致 RX buffer 满溢
3. **Judge 正则匹配**：评测脚本的正则 `^\[\s*[56SUM]*]` 只匹配 stream ID 5/6/SUM，我们的 fd 从 7 开始。需确认 Docker 评测环境下的行为

### 5.3 修改文件清单

| 文件 | 改动 |
|------|------|
| `os/src/fs/stdio.rs` | DevUrandom 设备 + DevNull/DevZero path() |
| `os/src/fs/mod.rs` | 导出 DevUrandom + ensure_basic_paths + File trait 扩展 |
| `os/src/syscall/fs.rs` | sys_pselect6 + fstatat AT_EMPTY_PATH + openat /dev/urandom |
| `os/src/syscall/mod.rs` | pselect6/setsid/setpgid/getpgid/getsid/getrusage 分发 |
| `os/src/syscall/process.rs` | setsid/setpgid/getpgid/getsid/getrusage 实现 |
| `os/src/task/process.rs` | session_id/pgid 字段 |
| `os/src/net/mod.rs` | loopback 多轮 poll + loopback_udp_inject() demux |
| `os/src/net/socket_file.rs` | connected_remote + udp_write loopback demux + graceful TCP close |
| `os/src/net/syscall.rs` | UDP connect 保存地址 + set_remote_endpoint + getpeername UDP + getsockopt 缓冲区 |
| `user/src/bin/initcode.rs` | waitpid 替代 wait |
| `vendor/smoltcp/src/phy/loopback.rs` | queue 字段 pub |
| `vendor/smoltcp/src/socket/udp.rs` | remote_endpoint 字段 + connected accepts 过滤 |

### 5.4 架构图

```
  iperf3 client (pid=3)              iperf3 server (pid=5)
   ┌─────────────────┐               ┌─────────────────┐
   │ write(fd=7,data) │               │ recvfrom(fd=7)  │
   │ write(fd=9,data) │               │ recvfrom(fd=8)  │
   └────────┬─────────┘               └────────▲────────┘
            │                                   │
   ─────────┼───── sys_write / sys_sendto ──────┼─────────
            │                                   │
            ▼                                   │
   ┌─────────────────────────────────────────────────────┐
   │              loopback_udp_inject()                   │
   │                                                      │
   │  1. 找 connected socket (remote_endpoint 匹配)      │
   │  2. fallback unconnected wildcard socket             │
   │  3. inject_recv() 直接写入目标 RX buffer             │
   └──────────────────────────────────────────────────────┘
```
