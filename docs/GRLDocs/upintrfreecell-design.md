# UPIntrFreeCell 设计说明

本文说明 `UPIntrFreeCell` 的设计目的、实现策略，以及与中断开关的关系与位置。

## 1. 设计目标

`UPIntrFreeCell<T>` 面向**单核**场景，提供：
- **临界区内自动关闭中断**，避免中断打断导致的重入/竞态。
- 保持与 `RefCell` 相似的借用语义（可提供 `RefMut` 风格接口）。
- 最小侵入地为设备驱动与内核同步提供“安全可变访问”。

主要使用场景：
- VirtIO 队列操作（提交/回收请求）。
- 中断上下文与进程上下文共享数据结构。  

## 2. 结构与接口

位置：`os/src/sync/up.rs`

核心接口：
- `UPIntrFreeCell::new(value)`：创建 cell。
- `exclusive_access()`：进入临界区，返回可变引用封装。
- `exclusive_session(f)`：简化闭包访问。

内部维护：
- `IntrMaskingInfo`：记录嵌套层级与进入前的 `SIE` 状态。
- `INTR_MASKING_INFO`：全局单例。

## 3. 中断屏蔽机制

进入临界区时：
- 读取 `sstatus::sie()`（是否允许 S 态中断）。
- **立即关闭 S 态中断**（`sstatus::clear_sie()`）。
- 记录进入前状态；支持嵌套。

退出临界区时：
- 嵌套计数归零时，若进入前 SIE 为 1，则恢复 `sstatus::set_sie()`。

## 4. 中断开关位置（当前实现）

### A. UPIntrFreeCell 内部
- **进入**：`sstatus::clear_sie()`
- **退出**：`sstatus::set_sie()`（仅在最外层退出时恢复）

这是**最细粒度**的自动关中断机制。

### B. Trap/内核流程
位置：`os/src/trap/mod.rs`

- **打开定时器中断**：
  ```rust
  trap::enable_timer_interrupt(); // sie::set_stimer()
  ```
- **用户态 syscalls 时允许外部中断打断内核**：
  在 `trap_handler()` 的 `UserEnvCall` 分支执行 `sstatus::set_sie()`。
- **返回用户态前关闭 S 态中断**：
  `trap_return()` 中调用 `disable_supervisor_interrupt()`，执行 `sstatus::clear_sie()`。

### C. Board 层
位置：`os/src/boards/qemu.rs`

- `device_init()` 中开启 PLIC 外设中断并设置优先级：
  - `plic.enable(...)`
  - `plic.set_priority(...)`
- **开启 S 态外部中断**：
  ```rust
  sie::set_sext();
  ```

## 5. 与 Condvar/调度协作

- 设备驱动中阻塞路径使用 `Condvar::wait_no_sched()`：
  - 将当前任务加入等待队列
  - 返回 `TaskContext` 指针
  - 调用 `schedule()` 切换
- 中断处理 `handle_irq()` 中唤醒等待任务

此时 `UPIntrFreeCell` 确保 wait 队列与 VirtIO 队列等共享数据结构的操作**不被中断打断**。

## 6. 小结

`UPIntrFreeCell` 是当前设备驱动与调度体系的关键基础：
- 提供单核安全的“关中断临界区”。
- 与 Condvar + schedule 形成可中断阻塞 I/O。
- 与 trap/board 的中断开启策略协同保证系统响应性。
