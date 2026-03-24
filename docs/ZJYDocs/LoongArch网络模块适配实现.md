# LoongArch64 网络模块适配实现

**日期**: 2026/3/24

---

## 1. 适配思路与架构

### 1.1 核心观察

rCore-lab 的网络评测（netperf、iperf3）全部使用 **127.0.0.1 loopback** 进行测试。这意味着不需要实现完整的 VirtIO 网卡 PCI 驱动，只需让 LoongArch 共享 RISC-V 已有的 smoltcp 网络栈，并启用 Loopback 设备即可。

### 1.2 适配架构

```
适配前 (LoongArch64):
  main.rs --[cfg_attr]--> net_stub.rs (所有 syscall 返回 -ENOSYS)

适配后 (LoongArch64):
  main.rs --> net/mod.rs (完整 smoltcp 栈, 仅 Loopback)
                |
                +-- net/syscall.rs   (共享, 全部网络 syscall 实现)
                +-- net/socket_file.rs (共享, SocketFile: File trait)
                +-- smoltcp Loopback  (127.0.0.1/8)
                +-- [无 VirtIO 外部网卡, 通过 #[cfg] 屏蔽]
```

**关键设计决策**: 采用 `#[cfg(target_arch = "riscv64")]` 条件编译将 VirtIO 网卡相关代码限制在 RISC-V 上,而非使用 `Option<T>` 包装。原因是 `VirtIONetDevice` 类型本身在 LoongArch 上不存在(其依赖的 `virtio-drivers-old` 仅在 RISC-V 配置中引入), 使用 cfg 可以避免引入不必要的类型依赖和运行时开销。

---

## 2. 具体改动

### 2.1 Cargo.toml: 为 LoongArch 添加 smoltcp 依赖

**问题**: smoltcp 原本只在 `[target.'cfg(target_arch = "riscv64")'.dependencies]` 中声明, LoongArch 编译时完全没有网络栈库。

**解决**: 在 `[target.'cfg(target_arch = "loongarch64")'.dependencies]` 中添加相同的 smoltcp 依赖配置:

```toml
[target.'cfg(target_arch = "loongarch64")'.dependencies]
smoltcp = { path = "../vendor/smoltcp", default-features = false, features = [
    "alloc", "log",
    "medium-ethernet", "medium-ip",
    "proto-ipv4",
    "socket-tcp", "socket-udp", "socket-icmp", "socket-raw",
] }
```

smoltcp 是纯 Rust 实现的网络协议栈,架构无关,不包含任何平台特定汇编,因此可以直接在 LoongArch 上编译。

### 2.2 main.rs: 移除 net_stub.rs 路径重定向

**原代码**:
```rust
#[cfg_attr(target_arch = "loongarch64", path = "net_stub.rs")]
pub mod net;
```

这会让 LoongArch 使用 `net_stub.rs` 替代完整的 `net/` 目录。移除 `cfg_attr` 后,两个架构共享同一个 `net/` 模块。

### 2.3 net/mod.rs: NetStack 条件编译重构

这是改动量最大、也最关键的部分。

**NetStack 结构体**: 使用 `#[cfg]` 条件编译字段:

```rust
pub struct NetStack {
    #[cfg(target_arch = "riscv64")]
    pub device: VirtIONetDevice,     // VirtIO 网卡 (RV only)
    #[cfg(target_arch = "riscv64")]
    pub iface: Interface,             // 外部网络接口 (RV only)
    pub lo_device: Loopback,          // 回环设备 (共享)
    pub lo_iface: Interface,          // 回环接口 (共享)
    pub sockets: SocketSet<'static>,  // 套接字集合 (共享)
}
```

**poll_external() 辅助方法**: 原来散落在 `poll_net()`, `poll_net_if_available()`, `poll_net_force()`, `socket_file.rs`, `syscall.rs` 中的 `stack.iface.poll(now, &mut stack.device, &mut stack.sockets)` 调用共有 **7 处**。如果每处都加 `#[cfg]` 块会非常分散且难以维护。

解决方案是在 `NetStack` 上添加 `poll_external()` 方法:

```rust
impl NetStack {
    pub fn poll_external(&mut self, now: smoltcp::time::Instant) {
        #[cfg(target_arch = "riscv64")]
        {
            self.iface.poll(now, &mut self.device, &mut self.sockets);
        }
        let _ = now; // suppress unused warning on non-riscv64
    }
}
```

这样所有调用点只需改为 `stack.poll_external(now)`,一行代码替代七处 cfg 块。

**init() 函数分支**: VirtIO 网卡初始化仅在 RISC-V 上执行,Loopback 初始化在所有架构上执行:

```rust
pub fn init() {
    let now = smoltcp_now();

    #[cfg(target_arch = "riscv64")]
    let (device, iface) = {
        // VirtIO-Net 初始化: MMIO 扫描, MAC 获取, IP 配置...
    };

    // Loopback 初始化 (所有架构)
    let mut lo_device = Loopback::new(Medium::Ip);
    // ...
}
```

### 2.4 TCP connect 的 InterfaceContext 借用问题

这是适配过程中遇到的最棘手的问题,花了较长时间解决。

**背景**: smoltcp 的 `tcp::Socket::connect()` 需要一个 `&mut Context` (即 `&mut InterfaceInner`) 参数。在 RISC-V 上,外部地址用 `stack.iface.context()`,本地地址用 `stack.lo_iface.context()`。

最初尝试封装为 `NetStack::connect_context(&mut self, is_loopback: bool)` 方法,但遇到了 Rust 借用检查器的限制:

```rust
let socket = stack.sockets.get_mut::<tcp::Socket>(handle);  // 借用 stack.sockets
let cx = stack.connect_context(is_loopback);                  // 借用整个 stack!
socket.connect(cx, ...);                                      // 冲突!
```

`connect_context` 作为 `&mut self` 方法,会借用整个 `NetStack`,与之前的 `stack.sockets` 借用冲突。

**解决方案**: 在调用点内联 cfg 逻辑,直接访问字段(Rust 借用检查器可以证明 `lo_iface`/`iface` 和 `sockets` 是不相交的字段):

```rust
let cx = if is_loopback {
    stack.lo_iface.context()
} else {
    #[cfg(target_arch = "riscv64")]
    { stack.iface.context() }
    #[cfg(not(target_arch = "riscv64"))]
    { stack.lo_iface.context() }
};
let socket = stack.sockets.get_mut::<tcp::Socket>(handle);
socket.connect(cx, connect_remote, local_port)?;
```

注意这里还调换了 context 获取和 socket 获取的顺序(context 先于 sockets),确保两个可变借用的生命周期不重叠。

### 2.5 fs/mod.rs: File trait 方法解除 cfg 限制

**问题**: `File` trait 中的 `as_socket()`, `set_connected_remote()`, `get_connected_remote()` 三个方法原本被 `#[cfg(target_arch = "riscv64")]` 限制。移除 net_stub 后,LoongArch 的 `net/syscall.rs` 和 `net/socket_file.rs` 也需要这些方法。

**解决**: 直接移除这三个方法上的 `#[cfg]` 属性。这些方法都有默认实现(返回 `None` / `{}` / `None`),不会影响其他 File 实现者。

### 2.6 boot.rs: 添加 net::init() 调用

在 LoongArch 的 `rust_main()` 中,在 `crate::fs::list_apps()` 之后、`crate::task::add_initproc()` 之前添加 `crate::net::init()` 调用。这与 RISC-V 的启动顺序保持一致。

---

## 3. 不需要改动的部分

### 3.1 drivers/net/mod.rs (VirtIO 网卡驱动)

`drivers/mod.rs` 中已有 `#[cfg(target_arch = "riscv64")]` 保护 `pub mod net;`,整个网卡驱动模块在 LoongArch 上不会编译,无需修改。

### 3.2 timer.rs (定时器轮询)

`timer.rs` 中的 `crate::net::poll_net_if_available()` 调用不需要修改。该函数内部使用 `try_exclusive_access()` 非阻塞获取锁,并通过 `poll_external()` 方法自动处理了架构差异。

### 3.3 boards/qemu.rs (PLIC 中断路由)

RISC-V 的 PLIC 中断路由 `VIRTIO_NET_IRQ => poll_net_if_available()` 仅在 RV 的 board 文件中,LoongArch 使用不同的 board 文件,不受影响。Loopback 模式不依赖外部中断,所有数据在 `poll_net()` 的 4 轮同步 poll 中处理。

### 3.4 net_stub.rs

该文件不再被引用,可以保留作为参考或删除。当前选择保留。

---

## 4. 改动统计

| 文件 | 改动类型 | 改动行数 |
|------|---------|---------|
| `os/Cargo.toml` | 添加 smoltcp 依赖 | +5 |
| `os/src/main.rs` | 移除 cfg_attr | -1, +1 |
| `os/src/net/mod.rs` | 重构 NetStack + init() + poll | ~50 行改动 |
| `os/src/net/syscall.rs` | 3 处 iface.poll 替换 + 1 处 context 内联 | ~15 行改动 |
| `os/src/net/socket_file.rs` | 2 处 iface.poll 替换 | ~4 行改动 |
| `os/src/fs/mod.rs` | 移除 3 个方法的 cfg 限制 | -3 行 |
| `os/src/boot.rs` | 添加 net::init() | +1 |

总改动量约 80 行,核心逻辑改动集中在 `net/mod.rs`。

---

## 5. 预期效果

### 5.1 评分预期

| 测试套件 | 适配前 | 适配后 |
|----------|--------|--------|
| netperf-musl | 0 (ENOSYS) | ~7.8/10 |
| iperf-musl | 0 (ENOSYS) | 6.0/6 |
| netperf-glibc | 0 (ENOSYS) | ~7.9/10 (需 glibc 动态链接) |
| iperf-glibc | 0 (ENOSYS) | 6.0/6 (需 glibc 动态链接) |

### 5.2 后续可选优化

1. **PCI VirtIO 网卡驱动**: 参照 `virtio_blk_pci.rs` 实现 PCI 版网卡驱动,支持外部网络通信
2. **LoongArch 中断路由**: 配置 LS7A/EIOINTC 中断控制器路由 VirtIO-net 中断
3. **删除 net_stub.rs**: 该文件已无引用,可以安全删除

---

## 6. 背景知识

### smoltcp 网络栈

smoltcp 是一个面向嵌入式系统的 TCP/IP 协议栈,纯 Rust 实现,无需操作系统支持。它提供:

- **Device trait**: 抽象网络设备接口(VirtIO 网卡、Loopback 等)
- **Interface**: 管理 IP 地址、路由表、ARP/NDP 缓存
- **SocketSet**: 管理多个并发的 TCP/UDP 套接字
- **Loopback device**: 内置的回环设备,数据在同一个 poll 周期内从 TX 传递到 RX

### Loopback 工作原理

Loopback 设备不涉及任何硬件,数据流程为:

1. 应用 A 通过 `send()` 将数据写入 TCP/UDP socket 的 TX buffer
2. `Interface::poll()` 将 TX buffer 中的数据"发送"到 Loopback device
3. 同一次或下一次 `poll()` 从 Loopback device "接收"数据
4. smoltcp 根据目标端口将数据路由到应用 B 的 RX buffer
5. 应用 B 通过 `recv()` 读取数据

由于 Loopback 的 TX->RX 需要多次 poll 才能完成完整往返,代码中使用 4 轮循环 poll(足以完成 TCP 三次握手: SYN -> SYN-ACK -> ACK -> Data)。

### 为什么不需要外部网卡

评测脚本的典型调用方式:

```bash
# iperf3
iperf3 -s -p 5001 -D            # 服务器监听 127.0.0.1:5001
iperf3 -c 127.0.0.1 -p 5001     # 客户端连接 127.0.0.1:5001

# netperf
netserver -D -L 127.0.0.1 -p 12865
netperf -H 127.0.0.1 -p 12865
```

所有测试流量都在 loopback 内完成,不经过外部网卡,因此仅 Loopback 即可拿到全部网络评分。
