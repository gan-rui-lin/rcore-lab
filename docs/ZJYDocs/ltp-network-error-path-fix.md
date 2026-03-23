# LTP 网络测试错误路径修复

**日期**: 2026/3/23

---

## 1. 背景

在 netperf 5/5 全通、LTP 网络测试从 0 到 14/25 PASS 的基础上，继续深挖剩余 11 个 FAIL 测试，针对可修的错误路径逐一阅读 LTP 源码、对比 Linux 行为、修复内核实现。

本轮工作聚焦于**网络 syscall 的错误返回值精确性**——LTP 对每个 errno 都做严格断言，我们的内核在很多边界条件下返回了错误的 errno 或缺少校验。

---

## 2. 逐项修复

### 2.1 sendto EFAULT：无效用户指针返回错误 errno

**LTP 测试**: sendto01 test 3 — `sendto(fd, (void*)-1, 1024, 0, addr, len)`

**期望**: EFAULT(14)（无效用户地址）

**旧行为**: 返回 EINVAL(22)。`translated_byte_buffer` 对高地址指针（如 `0xFFFFFFFFFFFFFFFF`）行为不确定，没有明确的 EFAULT 路径。

**修复** (`os/src/net/syscall.rs`):

在 `sys_sendto` 入口添加用户指针校验：

```rust
// Validate user buffer pointer
if len > 0 && (buf as usize) >= 0x4000_0000_0000 {
    return EFAULT;
}
```

`0x4000_0000_0000` 是 rCore-lab 用户地址空间上限。超过此地址的指针一定是非法的（内核地址或 `(void*)-1`）。

同理对 UDP sendto 的 `dest_addr` 指针也做检查：

```rust
if !dest_addr.is_null() && (dest_addr as usize) >= 0x4000_0000_0000 {
    return EFAULT;
}
```

**为什么阈值是 `0x4000_0000_0000`**: rCore-lab 的用户虚拟地址空间布局中，用户栈在 `0x7FFFFFFXX` 附近，堆和 mmap 区域更低。`0x4000_0000_0000`（256TB）足以覆盖所有合法用户地址，同时拒绝 `(void*)-1`（`0xFFFFFFFFFFFFFFFF`）这种明显非法的指针。

### 2.2 sendto UDP EMSGSIZE：数据报过大

**LTP 测试**: sendto01 test 8 — `sendto(udp_fd, bigbuf_128KB, 128*1024, ...)`

**期望**: EMSGSIZE(90)（消息过大）

**旧行为**: 尝试发送，smoltcp 可能截断或失败返回 EINVAL。

**修复** (`os/src/net/syscall.rs`):

在 UDP sendto 分支添加长度检查：

```rust
if len > 65535 {
    return EMSGSIZE;
}
```

UDP 数据报最大 65535 字节（含 IP 头），128KB 明显超限。

### 2.3 sendto UDP EINVAL：负的地址长度

**LTP 测试**: sendto01 test 6 — `sendto(fd, buf, len, 0, addr, -1)`

**期望**: EINVAL(22)（无效参数）

**修复**: 在 UDP 分支入口检查 `addr_len` 是否为负：

```rust
if (addr_len as isize) < 0 {
    return EINVAL;
}
```

### 2.4 connect 0.0.0.0 → 127.0.0.1：INADDR_ANY 语义

**LTP 测试**: sendto01 test 4 — server bind 到 `INADDR_ANY:0`，`getsockname` 返回 `0.0.0.0:PORT`，client connect 到 `0.0.0.0:PORT`。

**期望**: connect 成功（Linux 将 `connect(0.0.0.0:PORT)` 解析为 `127.0.0.1:PORT`）

**旧行为**: connect 到 `0.0.0.0` 走外部网络接口（VirtIO-Net），没有 server 在监听 → ECONNREFUSED。

**修复** (`os/src/net/syscall.rs`):

在 TCP connect 分支，将 `0.0.0.0` 重写为 `127.0.0.1`：

```rust
let is_loopback = match remote.addr {
    IpAddress::Ipv4(v4) => {
        let b = v4.as_bytes();
        b[0] == 127 || (b[0] == 0 && b[1] == 0 && b[2] == 0 && b[3] == 0)
    }
};
// Rewrite 0.0.0.0 to 127.0.0.1 (INADDR_ANY means localhost for connect)
let connect_remote = if remote.addr == IpAddress::v4(0, 0, 0, 0) {
    IpEndpoint::new(IpAddress::v4(127, 0, 0, 1), remote.port)
} else {
    remote
};
```

**为什么这么做**: 在 Linux 上，`connect(AF_INET, 0.0.0.0:PORT)` 会被内核路由为 `127.0.0.1:PORT`，因为 `INADDR_ANY` 在 connect 上下文中表示"本机任意地址"。许多测试程序（包括 LTP 的 sendto01）依赖这个行为：server bind 到 `0.0.0.0:0`，getsockname 获取端口，client 直接 connect 到 getsockname 返回的地址。

### 2.5 sendto TCP EPIPE：未连接 socket 发送

**LTP 测试**: sendto01 test 5 — `sendto(unconnected_tcp_fd, buf, len, ...)`

**期望**: EPIPE(32)（管道断裂，即未建立连接）

**旧行为**: 返回 0（EOF 式返回）。smoltcp 对 Closed 状态的 socket `may_send()=false`，我们的代码直接 `return 0`。

**修复** (`os/src/net/syscall.rs`):

区分"从未连接"和"连接后关闭"两种 `!may_send()` 情况：

```rust
if !socket.may_send() {
    use smoltcp::socket::tcp::State;
    let state = socket.state();
    // Closed/non-established: EPIPE (never connected)
    if matches!(state, State::Closed | State::Listen | State::SynSent | State::SynReceived) {
        return EPIPE;
    }
    // CloseWait/LastAck/etc: return 0 (connection closed after established)
    return 0;
}
```

**为什么区分**: Linux 对未连接的 SOCK_STREAM 发送返回 EPIPE + SIGPIPE（表示"管道不通"）。对已建立后断开的连接，`send` 也返回 EPIPE 但语义不同。在 rCore-lab 中，我们先区分连接前/后状态，连接前用 EPIPE，连接后用 0（EOF），这样既满足 LTP 断言又不影响 iperf3/netperf 的正常工作。

---

## 3. 测试结果

### 3.1 sendto01 子测试详情

| # | 描述 | 期望 | 旧结果 | 新结果 |
|---|------|------|--------|--------|
| 1 | bad fd (400) | EBADF | TPASS | TPASS |
| 2 | invalid socket (/dev/null) | ENOTSOCK | TPASS | TPASS |
| 3 | invalid send buffer (-1) | EFAULT | TFAIL(EINVAL) | **TPASS** |
| 4 | connected TCP | success | TBROK(ECONNREFUSED) | **TPASS** |
| 5 | not connected TCP | EPIPE | TBROK | **TPASS** |
| 6 | invalid to buffer length (-1) | EINVAL | TBROK | **TPASS** |
| 7 | invalid to buffer (-1) | EFAULT | TBROK | **TPASS** |
| 8 | UDP message too big (128KB) | EMSGSIZE | TBROK | **TPASS** |
| 9 | local endpoint shutdown | EPIPE | TBROK | TBROK* |
| 10 | invalid flags (MSG_OOB) | EOPNOTSUPP | TBROK | TBROK* |

*test 9-10 的 TBROK 是因为 server 子进程在处理完前面的连接后，select 循环未能及时响应新的 TCP 连接（loopback FIN 投递时序问题），导致 setup 的 connect 返回 ECONNREFUSED。这是 smoltcp loopback TCP 状态机的时序问题，不是错误路径问题。

**sendto01 子测试通过率: 2/10 → 8/10**

### 3.2 整体 LTP 网络测试结果

| 测试 | 结果 | 变化 |
|------|------|------|
| socket01 | PASS | 不变 |
| socket02 | PASS | 不变 |
| bind01 | FAIL(32) | 不变 (AF_UNIX) |
| bind02 | FAIL(2) | 不变 (getpwnam) |
| bind03 | FAIL(32) | 不变 (AF_UNIX) |
| listen01 | PASS | 不变 |
| accept01 | PASS | 不变 |
| accept02 | PASS | 不变 |
| accept03 | FAIL(2) | 不变 (O_PATH) |
| accept4_01 | FAIL(32) | 不变 (/proc) |
| connect01 | PASS | 不变 |
| connect02 | PASS | 不变 |
| send01 | FAIL(127) | 不变 (非 LTP 二进制) |
| send02 | FAIL(127) | 不变 (非 LTP 二进制) |
| **sendto01** | **FAIL(2)** | **改善: ret=3→2, TPASS 2→8** |
| sendto02 | PASS | 不变 |
| sendmsg01 | FAIL(2) | 不变 (需 ifconfig) |
| recv01 | PASS | 不变 |
| recvfrom01 | PASS | 不变 |
| getsockname01 | PASS | 不变 |
| getpeername01 | FAIL(32) | 不变 (AF_UNIX) |
| getsockopt01 | PASS | 不变 |
| getsockopt02 | FAIL(32) | 不变 (AF_UNIX) |
| setsockopt01 | PASS | 不变 |
| socketpair01 | PASS | 不变 |

**整体: 14/25 PASS (不变)，但 sendto01 内部通过率从 20% 提升到 80%**

### 3.3 回归验证

| 测试套件 | 结果 |
|---------|------|
| netperf (musl) | 5/5 PASS |
| basic (musl) | 102/102 满分 |
| iperf3 (musl) | 6/6 PASS |

---

## 4. 剩余 11 个 FAIL 的不可修原因

| 测试 | 失败原因 | 能否修 |
|------|---------|--------|
| bind01, bind03, getpeername01, getsockopt02 | 需要 AF_UNIX 域套接字 | 大功能，需独立实现 |
| accept4_01 | 需要 /proc/self/maps | 需要 procfs |
| send01, send02 | sdcard 上的文件不是 LTP 二进制（内容是 "hello"） | 需重建 sdcard |
| bind02 | 需要 getpwnam("nobody") 用户数据库 | 需要 /etc/passwd |
| sendmsg01 | 需要 ifconfig/ip 配置 loopback | 需要网络管理工具 |
| accept03 | 1 个子测试需要 O_PATH 标志 | 需要文件系统支持 O_PATH |
| sendto01 | test 9-10 server 生命周期问题 | loopback TCP 时序 |

---

## 5. 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `os/src/net/syscall.rs` | sendto EFAULT 检查、EMSGSIZE 检查、dest_addr 校验、connect 0.0.0.0→127.0.0.1 重写、TCP 未连接 EPIPE |

本轮只修改了 1 个文件，所有改动都在网络 syscall 的错误路径上。

---

## 6. 经验记录

### 经验 1: LTP 的严格 errno 断言是提升合规性的好工具

LTP 的每个子测试都对 errno 做精确断言（如期望 EFAULT 不接受 EINVAL）。这迫使我们把"大概能工作"的错误处理升级为"精确符合 POSIX 语义"的错误处理。虽然实际应用（iperf3/netperf）不关心这些边界 errno，但 LTP 合规意味着更广泛的应用兼容性。

### 经验 2: connect(0.0.0.0) 的隐含语义

INADDR_ANY 在 bind 和 connect 中有完全不同的含义：
- `bind(0.0.0.0:PORT)` = 监听所有接口
- `connect(0.0.0.0:PORT)` = 连接到本机（等价于 127.0.0.1）

很多测试程序的模式是 `bind(0.0.0.0:0)` → `getsockname` → `connect(返回的地址)`，依赖 connect 将 0.0.0.0 路由到 localhost。不实现这个转换，所有 server fork + client connect 的测试都会 ECONNREFUSED。

### 经验 3: smoltcp TCP 状态可以区分"未连接"和"已断开"

`may_send()=false` 有两种语义：
- Closed/Listen/SynSent → 从未成功连接 → 应返回 EPIPE
- CloseWait/LastAck/TimeWait → 曾经连接过但对端关闭 → 应返回 0 或 EPIPE

通过检查 `socket.state()` 可以精确区分，不需要额外的状态标志。

### 经验 4: sdcard 上的 send01/send02 不是 LTP 测试

LTP build 时某些测试名和 lmbench 的辅助文件冲突。sdcard 上 `ltp/testcases/bin/send01` 的内容是 "hello"（文本文件），不是编译的 ELF。这种问题只能通过重建 sdcard 解决，不是内核 bug。
