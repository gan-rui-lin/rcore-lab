# 设备层抽象层级设计（当前实现 2/11/2026）

本文描述当前 `rcore-lab` 的设备层抽象结构与关键数据流，强调**可移植性**与**可扩展性**（面向后续 VisionFive 等平台）。

## 1. 分层与职责

### A. Board 层：平台差异收敛
位置：`os/src/boards/qemu.rs`（后续可新增 `visionfive.rs` 等）

职责：
- **提供平台常量**：`CLOCK_FREQ`、`MEMORY_END`、`MMIO`、`VIRT_PLIC`、`VIRT_UART` 等。
- **绑定具体设备实现类型**：
  - `pub type BlockDeviceImpl = crate::drivers::block::VirtIOBlock;`
  - `pub type CharDeviceImpl = crate::drivers::chardev::NS16550a<VIRT_UART>;`
- **平台初始化入口**：`device_init()` 中配置 PLIC、开启 S 态外部中断。
- **外设中断分发**：`irq_handler()` 根据 IRQ 源号分发到具体设备的 `handle_irq()`。

这一层是**平台差异的唯一入口**，上层只通过 `crate::board::*` 访问。

### B. Config 层：平台常量透传
位置：`os/src/config.rs`

职责：
- 统一导出 board 的平台常量：
  ```rust
  pub use crate::board::{CLOCK_FREQ, MEMORY_END, MMIO};
  ```
- 使内核其它模块只依赖 `config`，而不直接依赖具体 board 文件。

### C. Drivers 层：设备驱动实现
位置：`os/src/drivers/*`

职责：
- **总线/基础设施**：`drivers/bus/virtio.rs`，提供 VirtIO HAL、DMA 相关支持。
- **具体设备驱动**：
  - `drivers/block/virtio_blk.rs`
  - `drivers/chardev/ns16550a.rs`
  - `drivers/input/mod.rs`
  - `drivers/gpu/mod.rs`
  - `drivers/net/mod.rs`
- **中断控制器**：`drivers/plic.rs`。

驱动层统一提供设备对象，并暴露**同步/阻塞与中断处理**能力。

### D. 同步/调度层：中断安全的内核同步原语
位置：`os/src/sync/*`

职责：
- `UPIntrFreeCell` 提供“进入临界区自动关中断”的内核单核安全互斥访问。
- `Condvar` 与 `task::schedule()` 协作，形成**基于中断的设备阻塞/唤醒模型**。

### E. Filesystem / VFS 层：面向块设备抽象
位置：`easy-fs/src/block_dev.rs`（以及 OS 内 VFS）

职责：
- `BlockDevice` trait 统一 I/O 接口：
  ```rust
  fn read_block(&self, block_id: usize, buf: &mut [u8]);
  fn write_block(&self, block_id: usize, buf: &[u8]);
  fn handle_irq(&self);
  ```
- 上层 FS 仅依赖 trait，不关心底层具体设备类型。

## 2. 关键数据流

### A. 启动初始化路径
`os/src/main.rs`：
1. `mm::init()` 完成基础内存映射。
2. `trap::init()` 设置 trap 入口。
3. `trap::enable_timer_interrupt()` 打开定时器中断。
4. `board::device_init()` 初始化 PLIC 并开启 S 态外部中断。
5. `DEV_NON_BLOCKING_ACCESS = true` 进入**中断驱动 I/O 模式**。

### B. I/O 读写与中断
- `VirtIOBlock::read_block/write_block` 支持两种路径：
  - **轮询**：直接阻塞等待硬件完成。
  - **中断**：提交请求 → `Condvar::wait_no_sched()` 阻塞 → `handle_irq()` 通过 `signal()` 唤醒。

### C. IRQ 分发链路
1. 中断进入：`trap` 捕获 `SupervisorExternal`。
2. `crate::board::irq_handler()` 分发 IRQ 源。
3. 设备驱动 `handle_irq()` 执行：
   - VirtIO blk 调用 `pop_used()` → 唤醒等待者。
   - UART/Input 读取数据并唤醒阻塞线程。

## 3. 可扩展性原则

为新增平台（如 VisionFive）建议遵循：
1. **新增 `os/src/boards/visionfive.rs`**：填写 MMIO / IRQ / 频率 / 内存边界。
2. **只在 Board 层绑定具体设备类型与 IRQ 号**；驱动层保持不变。
3. 若设备形态变化（如 PCI/SDIO），扩展 `drivers/bus/*`，不破坏上层 FS/VFS API。

## 4. 当前设计的抽象边界

- **平台差异**被限制在 `boards/*` 与 `config` 的重导出。
- **驱动实现**集中在 `drivers/*`，依赖 `sync` 提供的内核临界区与条件变量。
- **文件系统**只依赖 `BlockDevice` trait，不直接接触硬件细节。

这保证了：
- 平台扩展时，最小改动范围（优先改 board）。
- 设备驱动可复用（VirtIOBlock 不依赖具体 board）。
- 上层 FS/VFS 稳定。
