# IRQ 中断唤醒触发 BorrowMutError 的调度器重入调试报告

## 一、问题现象与罪魁祸首（先下结论）

本次崩溃的直接原因是 `RefCell::borrow_mut` 触发 `already borrowed: BorrowMutError`，其**罪魁祸首**是：在**内核中断上下文**里调用了 `Condvar::signal()`，进而调用 `wakeup_task()`/`add_task()`，导致 `TASK_MANAGER` 在**中断路径**与**正常调度/内核执行路径**发生重入式可变借用冲突。

关键证据来自 gdb 栈：

- 顶层 panic 发生在 `os::lang_items::panic`，错误为 `already borrowed: BorrowMutError`。
- 借用失败点在 `UPSafeCell::exclusive_access`（即 `RefCell::borrow_mut`），对应 `TASK_MANAGER.exclusive_access()`。
- 调用链清晰显示来自中断：`trap_from_kernel` → `irq_handler` → `virtio_blk::handle_irq` → `Condvar::signal` → `wakeup_task` → `add_task`。

这意味着：**中断处理里直接唤醒任务、修改就绪队列，导致调度器内部可变借用重入**。而 `UPSafeCell` 本身不屏蔽中断，允许在持有借用期间被打断，最终触发 borrow 冲突。

## 二、调试过程与证据链

### 1. 初始 panic 栈定位

最初看到 panic：

- `os::lang_items::panic` at `src/lang_items.rs:20`
- `core::cell::panic_already_borrowed`
- `UPSafeCell<os::task::manager::TaskManager>::exclusive_access`
- `os::task::manager::add_task`
- `os::task::manager::wakeup_task`
- `os::sync::condvar::Condvar::signal`

这说明崩溃发生在 `TASK_MANAGER.exclusive_access()`，也就是 `ready_queue` 的 `RefCell` 被重复可变借用。

### 2. 补充中断栈（关键证据）

补充的 gdb backtrace 进一步明确是在中断路径里调用了 `Condvar::signal()`：

```
#8  os::drivers::block::virtio_blk::handle_irq
#9  os::sync::up::UPIntrFreeCell<...>::exclusive_session
#10 os::drivers::block::virtio_blk::handle_irq
#11 os::board::irq_handler
#12 os::trap::trap_from_kernel
```

这条栈非常关键，它说明：**`Condvar::signal()` 不是在普通 syscall 或线程上下文里执行，而是被 IRQ 中断路径触发。**

因此，最合理的解释是：

- 某个正常路径（比如系统调用或调度逻辑）正在持有 `TASK_MANAGER` 的可变借用。
- 期间被中断打断，进入 IRQ 处理。
- IRQ 中又触发 `Condvar::signal()` → `wakeup_task()` → `add_task()`。
- 再次尝试 `TASK_MANAGER.exclusive_access()` 导致 `already borrowed`。

### 3. 为什么 `UPSafeCell` 会触发重入问题

`UPSafeCell` 的语义只是“单核环境下安全可变借用”，它依赖 `RefCell` 的动态借用检查，但**没有任何中断屏蔽机制**。所以一旦中断发生，借用会被“打断”，从而在中断路径中二次借用同一对象，引发 panic。

这与 `UPIntrFreeCell` 不同：`UPIntrFreeCell` 在进入借用时会屏蔽中断（保存并清除 SIE），离开时再恢复中断，保证借用过程不可被中断打断。

### 4. 最后锁定“罪魁祸首”

综合栈与代码路径，根因是：

- **调度器与 PID 映射在中断路径也会被访问**，但使用的是 `UPSafeCell`，导致中断重入可变借用。

这一点与 rCore 教程版实现不同，教程版已经将 `TASK_MANAGER`/`PID2PCB` 改为 `UPIntrFreeCell`，正是为了避免中断路径触发 borrow 冲突。

## 三、修复策略与实现

### 1. 修复思路

有两种思路：

1) 禁止 IRQ 中断上下文直接调用 `Condvar::signal()` 或 `wakeup_task()`，改用“底半部”或软中断延迟唤醒；
2) 保持现有 IRQ 行为，但让 `TASK_MANAGER`/`PID2PCB` 的访问过程**自动屏蔽中断**。

本次选择第 2 种方案，理由：

- 修改范围最小，直接对齐 rCore-Tutorial-v3 的成熟实现；
- 内核现有中断路径（virtio IRQ）已在实际使用 `Condvar::signal()`，短期内不修改调度语义更稳妥；
- 使用 `UPIntrFreeCell` 的语义与现有 `UPSafeCell` 一致，接口最小变化。

### 2. 具体修改

将 `TASK_MANAGER` 与 `PID2PCB` 从 `UPSafeCell` 改为 `UPIntrFreeCell`，对应文件：

- `os/src/task/manager.rs`

修改点：

- `use crate::sync::UPSafeCell;` → `use crate::sync::UPIntrFreeCell;`
- `TASK_MANAGER` 与 `PID2PCB` 的类型与初始化均改为 `UPIntrFreeCell`。

这确保在 `add_task()`/`fetch_task()`/`remove_task()` 等访问中**屏蔽中断**，避免 IRQ 过程中重入 `RefCell`。

## 四、为何该修复能解决问题

因为 `UPIntrFreeCell` 在 `exclusive_access()` 时：

- 进入借用前调用 `sstatus::clear_sie()`，屏蔽 S 模式中断；
- 退出借用时再恢复 `sie`；
- 所以持有 `RefMut` 时不会被 IRQ 打断。

这样，当内核主路径正在操作 `TASK_MANAGER` 时，中断不会抢占进入 `virtio_blk::handle_irq`，自然不会在中断里再次调用 `add_task()`，从而避免 `already borrowed`。

## 五、验证建议

建议在修复后进行以下验证：

1. 重复触发之前的场景（例如块设备读写带来的 IRQ），观察不再出现 `BorrowMutError`。
2. 若要进一步验证，可以在 `add_task()` 中临时打印 `sstatus::read().sie()`：
   - 修复前在 crash 前应常见 `sie=1`；
   - 修复后进入 `exclusive_access()` 时应看到 `sie=0`，表明中断已屏蔽。

## 六、背景补充：为何 IRQ 里直接唤醒会出问题

在内核设计中，IRQ 处理通常要求“快进快出”，不要做复杂调度操作，尤其不要触碰可能被内核主路径持有的锁或可变借用结构。这里的 `Condvar::signal()` 触发了调度器队列的修改，属于“调度域”操作。

如果不屏蔽中断，则会出现类似“可变借用重入”的问题。在多核或抢占更复杂的系统里，这种问题往往需要更严谨的锁语义（自旋锁 + 禁中断），而在 rCore 的单核模型中，`UPIntrFreeCell` 就是最直接的解决手段。

## 七、结论

- **故障原因**：IRQ 中断上下文调用 `Condvar::signal()` → `wakeup_task()` → `add_task()`，导致 `TASK_MANAGER` 发生 `RefCell` 可变借用重入。
- **关键证据**：gdb backtrace 显示调用链从 `trap_from_kernel`/`irq_handler` 进入 `virtio_blk::handle_irq` 后触发 `Condvar::signal`，最终 panic 在 `UPSafeCell::exclusive_access`。
- **修复措施**：参照 `rCore-Tutorial-v3`，将 `TASK_MANAGER`/`PID2PCB` 改为 `UPIntrFreeCell`，在访问时屏蔽中断，避免重入。
- **预期效果**：IRQ 期间不会打断调度器的可变借用，BorrowMutError 消失。

以上调试过程与修复路径具备可复现性与可解释性，符合本次 crash 的全部证据链。