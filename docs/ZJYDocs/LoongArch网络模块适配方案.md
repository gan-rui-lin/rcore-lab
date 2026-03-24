# LoongArch64 网络模块适配方案

**日期**: 2026/3/24

---

## 1. 背景

rCore-lab 的网络模块（smoltcp + VirtIO-net）目前只在 RISC-V 上工作。LoongArch64 构建使用 `net_stub.rs` 作为替代，所有网络 syscall 返回 `-ENOSYS`。这意味着 LoongArch 上 netperf、iperf3 等网络测试完全无法运行，评分为 0。

本文档记录了对现有代码、参考项目的分析，以及 LoongArch 网络适配的具体实现方案。

---

## 2. 现状分析

### 2.1 RISC-V 网络模块架构

RISC-V 上的网络栈已经完整可用：

```
用户态 (netperf/iperf3)
    │
    ├── sys_socket / sys_bind / sys_sendto / sys_recvfrom / ...
    │
os/src/net/syscall.rs          ← 网络 syscall 实现
os/src/net/socket_file.rs      ← SocketFile: File trait 实现
os/src/net/mod.rs              ← NetStack: smoltcp 全局网络栈
    │
    ├── VirtIONetDevice         ← 外部网络 (10.0.2.15/24)
    │   os/src/drivers/net/mod.rs
    │   使用 virtio-drivers-old (MMIO transport)
    │
    └── Loopback                ← 本地回环 (127.0.0.1/8)
        smoltcp 内置
```

**关键依赖**：

| 组件 | RISC-V | LoongArch |
|------|--------|-----------|
| virtio-drivers 版本 | `virtio-drivers-old` (VirtIOHeader/MMIO) | `virtio-drivers-new` (PciTransport) |
| VirtIO 总线 | MMIO (`0x10001000`) | PCI (`ECAM 0x20000000`) |
| HAL 实现 | `drivers/bus/virtio_rv.rs` | `drivers/bus/virtio_la.rs` |
| VirtIONet API | `new(header)`, `recv(buf)→len`, `send(buf)` | `new(transport, buf_len)`, `receive()→RxBuffer`, `send(TxBuffer)` |

### 2.2 LoongArch 现状

- **QEMU 启动参数已就绪**：`run-la.sh` 第 140 行已配置 `-device virtio-net-pci,netdev=net0`
- **PCI 总线基础已就绪**：`virtio_blk_pci.rs` 已实现 PCI 设备发现、BAR 分配、bus-master 启用
- **VirtIO HAL 已就绪**：`virtio_la.rs` 实现了 LoongArch 的 DMA 分配和 DMW 地址转换
- **缺失**：PCI 版 VirtIO 网卡驱动 + boot 路径调用 `net::init()` + 移除 `net_stub.rs`

### 2.3 两个 API 的核心差异

RISC-V 用的 `virtio-drivers-old`：

```rust
// 创建：传入 MMIO header 指针
let net = VirtIONet::<VirtioHal>::new(header)?;
// 收包：传入可变 buffer，返回长度
let len = net.recv(&mut buf)?;
// 发包：传入 byte slice
net.send(&buf)?;
```

LoongArch 用的 `virtio-drivers-new`（v0.7.1）：

```rust
// 创建：传入 PciTransport + buffer 长度
let net = VirtIONet::<VirtioHal, PciTransport, 32>::new(transport, 2048)?;
// 收包：返回 RxBuffer 对象（拥有 ownership）
let rx_buf: RxBuffer = net.receive()?;
let data: &[u8] = rx_buf.packet();
net.recycle_rx_buffer(rx_buf)?; // 必须回收
// 发包：先创建 TxBuffer，填充后发送
let mut tx_buf = net.new_tx_buffer(len);
tx_buf.packet_mut().copy_from_slice(&data);
net.send(tx_buf)?;
```

### 2.4 参考项目分析

#### chronix（oskernel2025-chronix-retest）

- 使用 smoltcp + VirtIO-net，架构中性设计
- 网卡驱动 `VirtIoNetDev<T: Transport>` 泛型化 transport 层
- LoongArch 用 PCI transport，RISC-V 用 MMIO transport
- **当前默认回退到 Loopback**（VirtIO-net 初始化被注释掉了）
- 网络测试（netperf/iperf）仅依赖 loopback 127.0.0.1，外部网卡非必须
- QEMU 配置：`virtio-net-pci` + `user,id=net0,hostfwd=tcp::5555-:5555`

#### T202410487992457-1800

- **没有真正的网络栈**，只有 pipe 模拟的 SimpleSocket
- 所有 socket syscall 是桩函数（bind/listen/connect 返回 0，sendto/recvfrom 返回假数据）
- 仅 RISC-V，无 LoongArch 支持
- 参考价值低

---

## 3. 适配方案

### 3.1 总体思路

核心观察：**netperf 和 iperf3 的所有测试都在 loopback 127.0.0.1 上运行**。不需要外部网卡驱动也能通过所有测试——只需 smoltcp 的 Loopback 设备。

因此适配分两步：
1. **第一步（高优先级）**：让 LoongArch 使用 smoltcp Loopback，移除 net_stub.rs，所有网络测试能跑
2. **第二步（低优先级）**：实现 PCI 版 VirtIO 网卡驱动，支持外部网络

第一步改动量极小（~10 行），但能直接解锁 netperf/iperf3 全部评分。

### 3.2 第一步：Loopback-only 网络栈

**改动清单**：

| 文件 | 改动 |
|------|------|
| `os/src/main.rs` | 移除 `#[cfg_attr(target_arch = "loongarch64", path = "net_stub.rs")]` |
| `os/src/net/mod.rs` | `init()` 中 VirtIONetDevice 创建用 `#[cfg(target_arch = "riscv64")]` 条件编译，LoongArch 跳过 |
| `os/src/net/mod.rs` | `NetStack` 的 `device` 和 `iface` 字段改为 `Option`，LoongArch 只创建 loopback |
| `os/src/boot.rs` | LoongArch `rust_main()` 添加 `crate::net::init()` 调用 |
| `os/src/drivers/net/mod.rs` | 整个模块用 `#[cfg(target_arch = "riscv64")]` 包裹 |

**核心改动**：`net/mod.rs` 的 `init()` 函数拆分为"外部网卡初始化"（RV only）+ "loopback 初始化"（共用）。

### 3.3 第二步：PCI VirtIO 网卡驱动（可选）

参照 `virtio_blk_pci.rs` 的 PCI 设备发现流程，创建 `drivers/net/virtio_net_pci.rs`：

1. 扫描 PCI bus 0 找 `DeviceType::Network`
2. 分配 BAR，启用 bus-master
3. 创建 `PciTransport` → `VirtIONet::new(transport, 2048)`
4. 实现 smoltcp `Device` trait（适配 RxBuffer/TxBuffer API）

这一步不影响评分（测试用 loopback），但可以支持 QEMU host 通信。

---

## 4. 风险与注意事项

### 4.1 smoltcp 编译兼容性

smoltcp 是纯 Rust、架构无关的库，不应有 LoongArch 编译问题。但需确认：
- `os/Cargo.toml` 中 smoltcp 依赖没有 `cfg(target_arch = "riscv64")` 限制
- `net/mod.rs` 和 `net/syscall.rs` 中没有 RISC-V 特有的汇编或地址常量

### 4.2 poll_net 与中断

RISC-V 的 `boards/qemu.rs` 有 PLIC 中断路由：`VIRTIO_NET_IRQ => poll_net_if_available()`。LoongArch 没有这个。但 loopback 不需要中断——所有数据在 `poll_net()` 的 4 轮 poll 中同步处理。外部网卡如果后续实现，需要配置 LoongArch 的中断控制器（LS7A/EIOINTC）。

### 4.3 `net_stub.rs` 的 SocketType 依赖

`net_stub.rs` 导出了 `SocketType` 枚举。如果其他 LoongArch 代码引用了 `net::SocketType`，移除 stub 后需确认真正的 `net/mod.rs` 也导出同名类型（它通过 `pub use socket_file::SocketType` 导出，没问题）。

### 4.4 测试要求

评测环境的测试脚本全部使用 127.0.0.1 loopback：
- iperf: `iperf3 -s -p 5001 -D` + `iperf3 -c 127.0.0.1 -p 5001`
- netperf: `netserver -D -L 127.0.0.1 -p 12865` + `netperf -H 127.0.0.1`

不需要外部网络即可拿满分。

---

## 5. 预期收益

| 测试套件 | 当前（LoongArch） | 适配后 |
|----------|-------------------|--------|
| netperf-musl | 0 分 | ~7.8/10 |
| netperf-glibc | 0 分 | ~7.9/10（需 glibc 动态链接也工作） |
| iperf-musl | 0 分 | 6.0/6（底分） |
| iperf-glibc | 0 分 | 6.0/6（底分） |

仅 loopback 适配（第一步）即可解锁全部网络评分。
