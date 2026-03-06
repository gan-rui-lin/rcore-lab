# smoltcp 网络协议栈适配实现记录

**日期**: 2026/03/06
**分支**: `net-work`
**当前版本**: v1.0 — socket 测试通过（static + dynamic），零 warning 编译

---

## 一、工作概述

对 rCore-Lab 的网络子系统进行**推倒重构**，将原先 52 行的玩具级 VirtIO 网络驱动替换为基于 smoltcp v0.11.0 的完整 TCP/IP 协议栈适配层。新实现覆盖从硬件驱动到系统调用的完整链路，socket 测试（UDP loopback + TCP loopback server/client）全部通过。

**数据流架构**:
```
用户态 syscall(198-212)
    → os/src/syscall/mod.rs (16个 dispatch arms)
        → os/src/net/syscall.rs (socket/bind/listen/connect/accept/sendto/recvfrom...)
            → os/src/net/socket_file.rs (SocketFile 实现 File trait: read/write/poll/Drop)
                → os/src/net/mod.rs (全局 NetStack: 双 Interface + SocketSet)
                    ├── os/src/drivers/net/mod.rs (VirtIO-Net → smoltcp Device)
                    └── smoltcp::phy::Loopback (127.0.0.1 loopback 设备)
```

---

## 二、开发历程

### Phase 1: smoltcp 版本选择（走了弯路）

1. **初始尝试 v0.12.0+**：vendor 目录里原有 smoltcp Git HEAD（162 commits past v0.12.0），使用 `edition = "2024"`
2. **修补 let-chain 语法**：在 9 个文件中修复了 11 处 `if let ... && let ...` → 嵌套 if-let
3. **遇到 25 处 core::net API 不兼容**：`Ipv4Addr::from_octets()`、`to_bits()`、`is_multiple_of()` 等方法不存在于 Rust 1.79
4. **放弃 v0.12，回退到 v0.11.0**：`git checkout v0.11.0`，零 API 不兼容

**教训**: smoltcp v0.12 的 `edition = "2024"` 不仅是语法问题，更深层的是它开始使用 `core::net::Ipv4Addr`（标准库的 IP 类型），而 v0.11 使用 smoltcp 自己的地址类型，与旧 Rust 完全兼容。

### Phase 2: 基础框架搭建

1. Vendor 了 4 个新依赖：`managed-0.8`, `heapless-0.8`, `hash32`, `stable_deref_trait`
2. 全部改为 `path = "../xxx"` 本地路径引用
3. 重写 VirtIO-Net 驱动（118 行，实现 smoltcp Device trait）
4. 创建全局 NetStack（Interface + SocketSet）
5. 实现 SocketFile（File trait）
6. 实现 16 个网络 syscall 的框架

### Phase 3: 首次运行 — UDP socket 测试失败

**错误输出**:
```
bind(s, (void *)&sa, sizeof sa)==0 failed: errno = Address in use
sendto(c, "x", 1, 0, (void *)&sa, sizeof sa)==1 failed: errno = Invalid argument
```

**根因分析**（通过阅读 musl socket.c 测试源码）:
1. `bind(0.0.0.0:0)` — port=0 表示内核自动分配，但 smoltcp 的 `udp::Socket::bind()` 拒绝 port=0（返回 `BindError::Unaddressable`）
2. `sendto` 到 `127.0.0.1` — smoltcp 没有 loopback 路由，无法投递

**修复**:
1. **port=0 自动分配**: `bind` 时检测 port=0，用 `alloc_ephemeral_port()` 分配 49152-65535 临时端口
2. **0.0.0.0 → wildcard**: 在 `endpoint_to_listen()` 中将 `0.0.0.0` 和 `127.x.x.x` 映射为 `IpListenEndpoint { addr: None }`
3. **UDP loopback**: 在 smoltcp 的 `udp.rs` 中新增 `inject_recv()` 方法，`sendto(127.x.x.x)` 时直接在内核中找到目标 socket 并注入数据

### Phase 4: TCP socket 测试失败

**错误输出**:
```
fcntl(s, F_GETFD)&FD_CLOEXEC failed: SOCK_CLOEXEC did not work
fcntl(c, F_GETFL)&O_NONBLOCK failed: SOCK_NONBLOCK did not work
connect(c, ...)  failed: errno = Connection refused
accept(s, ...)   failed: errno = Not supported
```

**逐一修复**:
1. **SOCK_CLOEXEC/SOCK_NONBLOCK**: SocketFile 添加 `cloexec`/`nonblock` 字段，File trait 添加 `fd_flags()`/`status_flags()` 方法，fcntl `F_GETFD`/`F_GETFL` 调用这些方法
2. **TCP bind(port=0)**: 同 UDP，自动分配端口，存入 `bound_port`（AtomicU16）
3. **TCP listen**: 调用 smoltcp 的 `tcp::Socket::listen()`
4. **TCP loopback connect**: 添加 smoltcp `Loopback` 设备 + `lo_iface`（127.0.0.1/8），connect 到 127.x.x.x 时使用 `lo_iface.context()`
5. **TCP accept**: 等待 listen socket 变 ESTABLISHED → 创建新 listen socket 替换原来的 → 旧 handle 作为 accepted fd

### Phase 5: accept 的 Drop panic

**错误**: `Panicked at socket_set.rs:116 handle does not refer to a valid socket`

**根因**: `sys_accept` 替换 `fd_table[listen_fd]` 时，旧的 `SocketFile` 被 Drop，其 `Drop` impl 调用 `socket.abort()` + `sockets.remove(listen_handle)` —— 把刚转移给 accepted fd 的连接也删了。

**修复**: 添加 `transferred: AtomicBool` 标志。accept 在替换前调用 `old_file.mark_transferred()`，Drop 中检查此标志跳过清理。

### Phase 6: 编译 warning 清理

修复了约 15 处 warning（smoltcp 内的 unused imports/vars、managed 的 trailing semicolons、initcode.rs 的 unused imports）。最终零 warning 编译。

---

## 三、解决的问题清单

| # | 问题 | 根因 | 解决方案 | 涉及文件 |
|---|------|------|---------|---------|
| 1 | smoltcp v0.12 编译失败 | edition 2024 + core::net API | 回退到 v0.11.0 | vendor/smoltcp/ |
| 2 | `bind(0.0.0.0:0)` 返回 EADDRINUSE | smoltcp 拒绝 port=0 | 自动分配临时端口 | net/syscall.rs |
| 3 | `sendto(127.0.0.1)` 返回 EINVAL | 无 loopback 路由 | UDP: inject_recv; TCP: Loopback 设备 | net/syscall.rs, net/mod.rs, udp.rs |
| 4 | SOCK_CLOEXEC 不生效 | fcntl F_GETFD 始终返回 0 | SocketFile.cloexec + fd_flags() | net/socket_file.rs, syscall/fs.rs |
| 5 | SOCK_NONBLOCK 不生效 | fcntl F_GETFL 不含 O_NONBLOCK | SocketFile.nonblock + status_flags() | net/socket_file.rs, syscall/fs.rs |
| 6 | TCP connect 到 127.0.0.1 失败 | 无 loopback 接口 | 添加 smoltcp Loopback + lo_iface | net/mod.rs |
| 7 | accept 返回 EOPNOTSUPP | 未实现 | 完整实现 listen→accept 流程 | net/syscall.rs |
| 8 | accept 时 panic | Drop 误删已转移的 socket | transferred 标志 | net/socket_file.rs |
| 9 | VirtIO-Net 地址错误 | 旧代码硬编码 0x10004000 | 修正为 0x10002000 (bus.1) | drivers/net/mod.rs |
| 10 | 旧驱动 spin_loop 阻塞 | VirtIO recv() 忙等待 | can_recv() 检查后非阻塞读取 | drivers/net/mod.rs |

---

## 四、修改文件完整清单

### 新建文件

| 文件 | 行数 | 说明 |
|------|------|------|
| `os/src/net/mod.rs` | ~130 | 网络子系统入口：双接口 NetStack（eth + loopback）、init、poll |
| `os/src/net/socket_file.rs` | ~250 | SocketFile 实现 File trait（TCP/UDP read/write/poll/Drop） |
| `os/src/net/syscall.rs` | ~650 | 16 个网络系统调用实现 |
| `vendor/managed-0.8/` | (外部) | smoltcp 依赖 |
| `vendor/heapless/` | (外部) | smoltcp 依赖 |
| `vendor/hash32/` | (外部) | heapless 依赖 |
| `vendor/stable_deref_trait/` | (外部) | heapless 依赖 |

### 重写文件

| 文件 | 行数 | 说明 |
|------|------|------|
| `os/src/drivers/net/mod.rs` | ~118 | VirtIO-Net smoltcp Device 适配（原 52 行全部替换） |

### 修改文件

| 文件 | 改动 |
|------|------|
| `os/Cargo.toml` | +smoltcp 依赖（alloc, log, medium-ethernet, medium-ip, proto-ipv4, socket-tcp/udp/icmp/raw） |
| `os/src/main.rs` | +`pub mod net;` +`net::init()` |
| `os/src/fs/mod.rs` | File trait +`as_socket()`, `fd_flags()`, `status_flags()`, `bound_port()`, `set_bound_port()`, `is_listening()`, `set_listening()`, `mark_transferred()` |
| `os/src/syscall/mod.rs` | +16 个 syscall 常量(198-242) + 16 个 dispatch arms |
| `os/src/syscall/fs.rs` | fcntl F_GETFD/F_GETFL 调用 File trait 方法 |
| `os/src/boards/qemu.rs` | PLIC 启用 VIRTIO_NET_IRQ + irq_handler 网络分发 |
| `os/src/drivers/mod.rs` | 移除 `pub use net::*` |
| `vendor/smoltcp/` | 回退到 v0.11.0 + path 依赖 + warning 修复 |
| `vendor/smoltcp/src/socket/udp.rs` | +`inject_recv()` 方法（loopback UDP 注入） |
| `vendor/managed-0.8/src/map.rs` | 移除 trailing semicolons |
| `user/src/bin/initcode.rs` | cfg-gate unused imports |

### 已实现的系统调用

| 编号 | 名称 | 状态 |
|------|------|------|
| 198 | socket | 完整（TCP/UDP, SOCK_CLOEXEC, SOCK_NONBLOCK） |
| 199 | socketpair | stub (EOPNOTSUPP) |
| 200 | bind | 完整（TCP: 记录端口; UDP: smoltcp bind, port=0 自动分配） |
| 201 | listen | 完整（smoltcp tcp::Socket::listen） |
| 202/242 | accept/accept4 | 完整（loopback TCP 三次握手 + handle 交换） |
| 203 | connect | 完整（TCP loopback + 外网; UDP no-op） |
| 204 | getsockname | 完整 |
| 205 | getpeername | 完整 |
| 206 | sendto | 完整（TCP: send_slice; UDP: loopback inject 或 smoltcp send） |
| 207 | recvfrom | 完整（阻塞等待） |
| 208 | setsockopt | stub（常用选项返回 0） |
| 209 | getsockopt | stub（SO_ERROR 返回 0） |
| 210 | shutdown | 完整（tcp::close / udp::close） |
| 211 | sendmsg | stub (EOPNOTSUPP) |
| 212 | recvmsg | stub (EOPNOTSUPP) |

---

## 五、测试结果

```
========== START entry-static.exe socket ==========
Pass!
========== END entry-static.exe socket ==========

========== START entry-dynamic.exe socket ==========
Pass!
========== END entry-dynamic.exe socket ==========
```

编译: **零 warning，零 error**。

不相关的 FAIL（与网络无关）：`dlopen`, `tls_get_new_dtv`（动态链接/TLS 已知问题）。

---

## 六、架构设计要点

### 6.1 双接口模型

```
NetStack {
    device: VirtIONetDevice     ← 外部网络（10.0.2.15/24, 网关 10.0.2.2）
    iface: Interface            ← smoltcp 以太网接口
    lo_device: Loopback         ← 127.0.0.1 本地回环
    lo_iface: Interface         ← smoltcp IP 接口
    sockets: SocketSet          ← 共享 socket 集（最多 64 个）
}
```

`poll_net()` 同时 poll 两个接口，使 loopback TCP 三次握手能完成。

### 6.2 UDP Loopback: inject_recv()

smoltcp 的 Loopback 设备不支持 UDP（没有 ARP，UDP 需要目的地址解析）。所以 UDP loopback 通过内核直接注入：在 smoltcp 的 `udp.rs` 中新增 `inject_recv()` 公开方法，`sendto(127.x.x.x)` 时遍历 SocketSet 找到目标端口的 socket 直接写入 rx_buffer。

### 6.3 TCP accept 的 handle 交换

smoltcp 没有 accept 队列。listen socket 收到 SYN 后自己变成 ESTABLISHED。accept 的实现：
1. 等待 listen socket 变 active
2. 创建新 TCP socket 在同一端口重新 listen
3. 标记旧 SocketFile 为 `transferred`（阻止 Drop 清理）
4. 旧 handle（ESTABLISHED）分配给新 fd
5. 新 listen handle 替换 listen fd
