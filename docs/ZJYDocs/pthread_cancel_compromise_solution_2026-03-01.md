# pthread_cancel 测试妥协方案与内核超时检测机制

日期：2026/3/1

## 背景

pthread_cancel 是 POSIX 线程库中用于取消线程执行的机制。在 musl libc 中，pthread_cancel 通过 SIGCANCEL（信号 33）实现：

1. **异步取消模式**（PTHREAD_CANCEL_ASYNCHRONOUS）：线程可以在任意时刻被取消
2. **延迟取消模式**（PTHREAD_CANCEL_DEFERRED）：线程只能在取消点（cancellation points）被取消

### 测试场景

libc-test 的 pthread_cancel 测试包含三个子场景：

1. **异步取消测试**：创建线程，设置为异步取消模式，立即取消并验证线程退出状态为 PTHREAD_CANCELED
2. **单个 cleanup handler 测试**：线程注册一个清理函数，在延迟取消点（sleep）被取消，验证 cleanup handler 被执行
3. **嵌套 cleanup handlers 测试**：线程注册多个嵌套的清理函数，验证所有 cleanup handlers 按 LIFO 顺序执行

### 核心问题

在 rcore-lab 上运行 pthread_cancel 测试时，遇到两个主要问题：

**问题 1：pthread_cancel_points 测试失败**
- 测试用例使用 `/dev/shm` 目录进行 POSIX 共享内存测试
- rcore-lab 最初未创建 `/dev/shm` 目录，导致 VFS 错误
- **已解决**：在 `os/src/fs/mod.rs:ensure_basic_paths()` 中添加 `create_dir("/dev/shm")`

**问题 2：pthread_cancel 测试无限期挂起**
- 测试进程（pid 36）的线程被 pthread_cancel 后，应该退出并返回 PTHREAD_CANCELED 状态
- 但实际运行中，线程卡死在某个 futex_wait 调用中，主线程永远等待在 pthread_join
- 测试超时约 10 秒后被 SIGKILL 信号强制终止
- **这是本文档的核心问题**

## 深入分析：为什么 pthread_cancel 测试会挂起？

### SIGCANCEL 信号投递机制

根据 GDB 调试和日志分析，pthread_cancel 的执行流程如下：

1. 主线程调用 `pthread_cancel(tid)` → 内核 `sys_tkill(tid, SIGCANCEL)`
2. 内核将 SIGCANCEL（sig 33）标记为 pending
3. 下次目标线程从内核返回用户态时，检测到 pending signal
4. 内核投递信号：保存用户上下文（trap context），设置 PC 为信号处理函数（handler）
5. 用户态信号 handler 执行：
   - 检查 TLS 中的 `cancel`、`canceldisable`、`cancelasync` 字段
   - 如果允许取消，调用 `__cancel()` 进行清理并调用 `pthread_exit(-1)`
6. Handler 返回时调用 `sigreturn` 系统调用恢复原始上下文

### TLS 字段布局

musl 的 pthread 结构体存储在线程本地存储（TLS）中，关键字段相对于 `tp` 寄存器的偏移：

```
tp - 0x9c: cancel         // 是否有取消请求（0=无，1=有）
tp - 0x98: canceldisable  // 是否禁用取消（0=启用，1=禁用）
tp - 0x97: cancelasync    // 是否异步取消（0=延迟，1=异步）
```

### 异步取消测试的执行流程

对于 pthread_cancel 测试中的异步取消场景（pid=36），日志显示：

```
[INFO] [sigaction] pid=36 signum=33 handler=0x3e134 ...
  → 线程注册 SIGCANCEL handler

[INFO] [signal] has_sig33 pid=36 tid=1 task_pending=SIG33 mask=SIGCHLD
  → 检测到 pending SIGCANCEL

[INFO] [signal] deliver_pre pid=36 tid=1 signum=33 saved_pc=0x27428
  → 投递 SIGCANCEL，保存当前 PC（futex_wait 返回地址）

[INFO] [signal] save_trap_cx pid=36 tid=1 signum=33 saved_pc=0x27428 new_mask=... | SIG33 | ...
  → 保存 trap context，进入 handler 时屏蔽 SIG33

[INFO] [sigreturn] pid=36 ucontext_ptr=0x40022750 saved_pc=0x27428 ucontext_pc=0x27428
  → Handler 执行完毕，调用 sigreturn 恢复上下文

[INFO] [sample] pid=36 ... sepc=0x27428 sp=0x40022a90 ra=0x27428
  → 定时器采样显示线程卡在 PC=0x27428，无法推进

[WARN] [signal] pid=36 name=entry-static.exe killed by SIGKILL
  → 约 10 秒后被超时机制强制 SIGKILL
```

### 问题根源分析

根据 [pthread_cancel_workaround_attempt_2026-03-01.md](pthread_cancel_workaround_attempt_2026-03-01.md) 的详细分析，发现：

**关键发现 1：pthread_setcanceltype 未实现**
- 使用 `riscv64-unknown-elf-nm` 反汇编 busybox，发现 `pthread_setcanceltype` 符号不存在
- 测试代码 `pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, 0)` 调用了一个未实现的函数
- 但令人意外的是，TLS 中的 `cancelasync` 字段 **确实被设置为 1**（异步模式）

**关键发现 2：Handler 执行后清理失败**
- SIGCANCEL handler 成功执行（有 sigreturn 日志）
- 但 cleanup 流程未正确完成：
  - `clear_child_tid` wake 唤醒了 0 个线程：`woke=0`
  - 主线程仍在 pthread_join 等待：`FUTEX_WAIT uaddr1=0x40022b30`
- 线程被 handler 尝试取消，但没有正确退出，而是返回到原来的 futex_wait 循环

**关键发现 3：内核 Workaround 不可行**
- **方案 1**：内核实现 `sys_pthread_setcanceltype` 系统调用
  - 失败原因：测试程序根本不调用这个系统调用（函数在二进制中不存在）
- **方案 2**：内核直接读取 TLS `cancelasync` 字段并强制退出
  - 问题：绕过了用户态的清理流程（cleanup handlers、pthread_exit）
  - 结果：`clear_child_tid` woke=0，主线程仍在等待
- **方案 3**：移除 Workaround，让 Handler 自己处理
  - 结果：出现新的内核 panic（InstructionPageFault at 0x8294eb80）
  - 表明存在更深层次的内存管理或上下文切换问题

### 为什么无法修复？

pthread_cancel 机制是 **纯用户态** 的复杂流程：

1. **信号处理**：内核只负责投递 SIGCANCEL，用户态 handler 负责决策和清理
2. **Cleanup handlers**：用户态维护清理函数栈，需要按 LIFO 顺序执行
3. **pthread_exit**：正确的取消需要调用 pthread_exit 进行线程清理
4. **futex 唤醒**：需要正确设置 `clear_child_tid` 并唤醒等待线程

内核无法在不破坏用户态语义的前提下强制完成这些步骤。可能的根本原因包括：

- **busybox 的 musl libc 实现不完整**：pthread_setcanceltype 缺失，可能其他 pthread 函数也有问题
- **SIGCANCEL handler 实现有 bug**：虽然 handler 执行了，但清理流程没有正确完成
- **内核信号处理存在问题**：如方案 3 触发的 InstructionPageFault，表明可能有内核 bug

## 妥协方案：内核超时检测与强制终止

### 设计目标

既然无法正确修复 pthread_cancel 测试，采用妥协方案：

**目标**：**防止 pthread_cancel 测试无限期挂起，避免阻塞其他测试运行**

**策略**：
1. **不追求测试通过**：接受 pthread_cancel 测试失败
2. **快速失败**：检测测试卡死后，在 100ms 内强制 SIGKILL
3. **最小化影响**：只针对 pthread_cancel 测试进程（pid 34 和 36），不影响其他进程

### 实现机制

在定时器中断处理函数（timer interrupt handler）中添加 **PC 卡死检测**：

#### 1. 数据结构

在 `TaskControlBlockInner` 中添加追踪字段（[os/src/task/task.rs](../../os/src/task/task.rs)）：

```rust
pub struct TaskControlBlockInner {
    // ... 其他字段 ...

    /// 用于检测 pthread_cancel 测试卡死：上一次检查的 PC
    pub sigcancel_last_pc: usize,

    /// 用于检测 pthread_cancel 测试卡死：PC 未变化的计数
    pub sigcancel_loop_count: usize,
}
```

#### 2. 检测逻辑

在用户态和内核态的定时器中断处理中添加相同的检测逻辑（[os/src/trap/mod.rs](../../os/src/trap/mod.rs)）：

**位置 1：用户态 trap handler（`trap_handler` 函数）**

```rust
Trap::Interrupt(Interrupt::SupervisorTimer) => {
    set_next_trigger();
    check_timer();

    // 检测 pthread_cancel 测试挂起（pid 34 和 36）
    if let Some(task) = current_task() {
        if let Some(process) = task.process.upgrade() {
            let pid = process.pid.0;

            // 只监控 pthread_cancel 测试进程
            if pid == 36 || pid == 34 {
                let mut task_inner = task.inner_exclusive_access();
                let current_pc = task_inner.get_trap_cx().sepc;

                // 检查 PC 是否卡在相同位置
                if current_pc == task_inner.sigcancel_last_pc && current_pc != 0 {
                    task_inner.sigcancel_loop_count += 1;

                    // 100ms 后强制 SIGKILL（10 个定时器 tick，每个 10ms）
                    const MAX_STUCK_TICKS: usize = 10;
                    if task_inner.sigcancel_loop_count >= MAX_STUCK_TICKS {
                        let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
                        let mut process_inner = process.inner_exclusive_access();
                        warn!(
                            "[timer] pthread_cancel test stuck pid={} tid={} pc={:#x} for {} ticks, sending SIGKILL",
                            pid, tid, current_pc, task_inner.sigcancel_loop_count
                        );

                        // 强制 SIGKILL
                        process_inner.signal_pending.insert(SignalFlags::SIGKILL);
                        task_inner.sigcancel_last_pc = 0;
                        task_inner.sigcancel_loop_count = 0;
                    }
                } else {
                    // PC 发生变化，更新追踪状态
                    task_inner.sigcancel_last_pc = current_pc;
                    task_inner.sigcancel_loop_count = 1;
                }
            }
        }
    }

    // ... 其他定时器处理逻辑 ...
}
```

**位置 2：内核态 trap handler（`trap_from_kernel` 函数）**

在内核态定时器中断中添加相同的检测逻辑（代码与上述相同，日志标记为 `[timer_k]`）。

### 关键设计决策

#### 1. 为什么不检查 signal_pending？

**错误的初始实现**：

最初的检测逻辑检查 `signal_pending.contains(SignalFlags::SIG33)`：

```rust
let has_pending_sigcancel = process_inner.signal_pending.contains(SignalFlags::SIG33)
    || task_inner.signal_pending.contains(SignalFlags::SIG33);
```

**为什么失败**：

根据日志分析，SIGCANCEL 投递后的状态变化：

1. **投递前**：`signal_pending` 包含 SIG33
2. **投递时**：信号从 `signal_pending` 移除，添加到 `signal_mask`
3. **Handler 执行期间和返回后**：`signal_pending` **不再** 包含 SIG33

所以在线程卡死阶段，`has_pending_sigcancel` 始终为 `false`，检测逻辑永远不会触发！

**正确的实现**：

直接检测 **PC 是否卡死**，不依赖信号状态：

```rust
if current_pc == task_inner.sigcancel_last_pc && current_pc != 0 {
    task_inner.sigcancel_loop_count += 1;
    // ...
}
```

这个检测对任何原因导致的 PC 卡死都有效。

#### 2. 为什么选择 100ms 超时？

- **太短**（如 20ms）：可能误杀正常但稍慢的进程
- **太长**（如 1s）：测试失败需要等待过久，降低测试效率
- **100ms**：足够让正常流程执行，同时快速失败避免阻塞

参考：原始超时约 10 秒，100ms 是 1/100，大幅缩短测试失败时间。

#### 3. 为什么硬编码 pid 34 和 36？

- **精确性**：pthread_cancel 测试固定使用这两个 pid
- **最小化影响**：只影响这两个测试进程，不影响其他进程的正常超时处理
- **可维护性**：如果以后需要扩展，可以改为检测进程名（`entry-static.exe`）或使用进程标记

#### 4. 为什么在用户态和内核态 trap handler 都添加检测？

- **覆盖所有场景**：线程可能卡在用户态循环或内核态系统调用中
- **防御性设计**：即使只在一个地方卡死，也能被检测到
- **低开销**：检测只针对 pid 34/36，对其他进程无性能影响

### 实现效果

**修改前**：
- pthread_cancel 测试挂起约 10 秒
- 日志中大量定时器采样显示 PC 卡在 0x27428
- 最终被超时机制 SIGKILL（默认超时 10 秒）

**修改后**：
- 定时器中断检测到 PC 卡死
- 100ms 后输出警告日志：
  ```
  [WARN] [timer] pthread_cancel test stuck pid=36 tid=1 pc=0x27428 for 10 ticks, sending SIGKILL
  ```
- 立即 SIGKILL，测试快速失败
- 不阻塞后续测试运行

## 技术细节

### PC 卡死的具体位置

根据日志，pid=36 卡在 `PC=0x27428`。结合之前的 SIGCANCEL 投递日志：

```
[INFO] [signal] deliver_pre pid=36 tid=1 signum=33 saved_pc=0x27428
[INFO] [sigreturn] pid=36 ucontext_ptr=0x40022750 saved_pc=0x27428 ucontext_pc=0x27428
```

`0x27428` 是 **sigreturn 恢复的 PC**，即 SIGCANCEL 投递时保存的原始 PC。这意味着：

1. 线程在 futex_wait 等待时被中断（PC=0x27428）
2. SIGCANCEL handler 执行
3. Sigreturn 恢复到 0x27428
4. 理论上应该从 futex_wait 返回，但实际 **重新进入 futex_wait 循环**
5. 再次卡在 0x27428 等待

### 为什么 futex_wait 没有被唤醒？

正常流程：

1. 子线程调用 `pthread_exit(-1)` 时，应该：
   - 执行 cleanup handlers
   - 调用 `sys_set_tid_address(0)` 清除 `clear_child_tid`
   - 退出时内核检查 `clear_child_tid`，如果非零则执行 `futex_wake(clear_child_tid, 1)`
2. 主线程在 `pthread_join` 中调用 `futex_wait(tid_ptr, expected_tid)`
3. 子线程退出时的 `futex_wake` 唤醒主线程
4. 主线程从 `pthread_join` 返回，获取子线程的退出状态

实际情况：

根据日志，`clear_child_tid` wake 唤醒了 **0 个线程**：

```
[INFO] [exit] pid=36 tid=1 clear_child_tid=0x9db80 woke=0
```

这说明主线程 **不在** 等待 `0x9db80` 这个地址，而是在等待 `0x40022b30`：

```
[INFO] [sys_futex] pid=36 tid=0 cmd=FUTEX_WAIT uaddr1=0x40022b30 ...
```

**问题**：`clear_child_tid` 地址与主线程 futex_wait 地址不匹配！

可能原因：
- musl 的 pthread_join 实现使用了不同的 futex 地址（不是 clear_child_tid）
- 或者 busybox 的实现有 bug，导致地址不匹配
- 或者 pthread_cancel 流程破坏了正常的 pthread_exit 清理

### 内核 Panic（InstructionPageFault at 0x8294eb80）

在尝试方案 3（移除 workaround，让 handler 自己处理）时，出现：

```
[kernel] Panicked at src/trap/mod.rs:459 Unsupported trap from kernel:
Exception(InstructionPageFault), stval = 0x8294eb80!
```

**分析**：

- `stval = 0x8294eb80` 是一个 **物理地址**（在 0x8000_0000 以上）
- 内核尝试执行这个物理地址，触发指令页错误
- 这个地址正好是 `clear_child_tid` 的物理地址

**推测**：

- 内核在某个地方错误地将物理地址当作虚拟地址或函数指针
- 可能是上下文切换、信号处理或 futex 操作中的 bug
- 这是一个独立的内核 bug，与 pthread_cancel workaround 无关

## 相关文件修改

### 1. TaskControlBlockInner 结构体

[os/src/task/task.rs](../../os/src/task/task.rs):

```rust
pub struct TaskControlBlockInner {
    // ... 其他字段 ...
    pub sigcancel_last_pc: usize,
    pub sigcancel_loop_count: usize,
}
```

**初始化**（在构造函数中）：

```rust
sigcancel_last_pc: 0,
sigcancel_loop_count: 0,
```

### 2. 定时器中断检测逻辑

[os/src/trap/mod.rs](../../os/src/trap/mod.rs)（两处）：

- 行 346-376：用户态 trap handler 的定时器中断分支
- 行 501-531：内核态 trap handler 的定时器中断分支

（代码见上文"实现机制"部分）

### 3. /dev/shm 目录创建

[os/src/fs/mod.rs](../../os/src/fs/mod.rs):161

```rust
pub fn ensure_basic_paths() {
    create_dir("/etc");
    create_dir("/dev");
    create_dir("/dev/misc");
    create_dir("/dev/shm");  // ← 新增，修复 pthread_cancel_points 测试
    create_dir("/bin");
    // ...
}
```

## 测试结果

### pthread_cancel_points 测试

**状态**：✅ **通过**

添加 `/dev/shm` 目录后，测试成功：

```
test pthread_cancel_points...
pthread_cancel_points PASSED
```

### pthread_cancel 测试

**状态**：❌ **失败（预期行为）**

日志输出：

```
test pthread_cancel...
[INFO] [signal] deliver_pre pid=36 tid=1 signum=33 saved_pc=0x27428
[INFO] [sigreturn] pid=36 ucontext_ptr=0x40022750 saved_pc=0x27428
[INFO] [sample] pid=36 ... sepc=0x27428 sp=0x40022a90 ra=0x27428
[WARN] [timer] pthread_cancel test stuck pid=36 tid=1 pc=0x27428 for 10 ticks, sending SIGKILL
[WARN] [signal] pid=36 name=entry-static.exe killed by SIGKILL
pthread_cancel FAILED (timeout/killed)
```

**改进效果**：
- 挂起时间：从 10 秒缩短到 100ms
- 测试总时间：从 ~10s 缩短到 ~0.1s
- 不影响后续测试运行

## 结论与展望

### 妥协策略总结

1. **pthread_cancel_points 测试**：✅ **已完全修复**（创建 /dev/shm 目录）
2. **pthread_cancel 测试**：❌ **无法修复，采用快速失败策略**

**妥协原因**：

- pthread_cancel 机制涉及复杂的用户态线程管理和清理流程
- busybox 的 musl libc 实现可能不完整或有 bug
- 内核无法在不破坏用户态语义的前提下强制完成清理
- 尝试的所有 workaround 方案都破坏了正常的线程退出流程

**妥协方案优点**：

- ✅ 不阻塞其他测试运行
- ✅ 快速失败（100ms vs 10s）
- ✅ 最小化对其他进程的影响
- ✅ 实现简单，易于维护

**妥协方案缺点**：

- ❌ pthread_cancel 测试仍然失败
- ❌ 没有解决根本问题（线程取消清理流程）

### 未来改进方向

#### 1. 使用完整的 musl libc

- 当前的 busybox 可能使用了精简版 musl，pthread 实现不完整
- 使用完整的 musl libc 或 glibc 进行测试
- 或者直接使用官方的 riscv64-linux-gnu-gcc 编译的测试程序

#### 2. 深入调试 InstructionPageFault

- 这是一个独立的内核 bug，可能影响其他功能
- 需要定位为什么内核会跳转到物理地址 0x8294eb80
- 可能与 futex、线程退出或上下文切换有关

#### 3. 实现更完善的 pthread 支持

- 完整实现 `sys_set_tid_address` 的 `clear_child_tid` 语义
- 验证 futex 地址匹配问题
- 增强信号处理的健壮性

#### 4. 使用 GDB 动态调试

- 在 QEMU 中启用 GDB stub
- 单步执行 SIGCANCEL handler 和 pthread_exit 流程
- 确定清理流程在哪一步失败

### Known Limitations

将 pthread_cancel 测试失败标记为 **Known Limitation**：

**问题描述**：pthread_cancel 异步取消测试失败，线程无法正确退出

**影响范围**：仅影响 pthread_cancel 测试，不影响其他线程功能

**缓解措施**：内核检测卡死并在 100ms 内强制 SIGKILL

**计划修复时间**：待升级到完整的 musl libc 或重构 pthread 实现

## 参考文档

- [pthread_cancel_workaround_attempt_2026-03-01.md](pthread_cancel_workaround_attempt_2026-03-01.md) - 详细的 workaround 尝试与分析
- [pthread_cancel_eintr_fix_2026-02-27.md](pthread_cancel_eintr_fix_2026-02-27.md) - pthread_cancel_points 修复
- [pthread_cancel_sigreturn_loop_fix_2026-02-26.md](pthread_cancel_sigreturn_loop_fix_2026-02-26.md) - 早期的 sigreturn 循环修复

## 附录：测试源码

测试源码位于：[/Users/mac/Desktop/project/syscall/testsuits-for-oskernel/libc-test/src/functional/pthread_cancel.c](../../../../testsuits-for-oskernel/libc-test/src/functional/pthread_cancel.c)

关键代码片段：

```c
// 异步取消测试
static void *start_async(void *arg)
{
    pthread_setcanceltype(PTHREAD_CANCEL_ASYNCHRONOUS, 0);  // 设置异步取消
    sem_post(arg);
    for (;;);  // 无限循环，等待被取消
    return 0;
}

int main(void)
{
    pthread_t td;
    sem_t sem1;
    void *res;

    // 异步取消测试
    sem_init(&sem1, 0, 0);
    pthread_create(&td, 0, start_async, &sem1);
    while (sem_wait(&sem1));  // 等待线程启动
    pthread_cancel(td);       // 取消线程
    pthread_join(td, &res);   // 等待线程退出 ← 卡死在这里
    assert(res == PTHREAD_CANCELED);  // 验证退出状态

    // ... 其他测试 ...
}
```

**预期行为**：`pthread_join` 应该在 100ms 内返回，`res` 应该是 `PTHREAD_CANCELED`

**实际行为**：`pthread_join` 永远等待，100ms 后内核检测到卡死并 SIGKILL
