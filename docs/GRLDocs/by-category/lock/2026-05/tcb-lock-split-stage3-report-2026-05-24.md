# TCB 物理拆分与访问点强收敛报告

## 摘要

本次第三阶段重构聚焦 `TaskControlBlock`。核心目标是把原先单个 `TaskControlBlockInner` 粗粒度锁拆成按访问者分类的三组中断安全锁，并把 syscall、task、trap、procfs、futex、timer 等常规路径中对整块 TCB inner 的直接借用收敛到领域 helper。

本轮不拆 PCB 的 `memory/signals/family` 等过渡状态，也不引入 SMP spin lock 或 sleep lock。并发模型仍然保持“单核 + 中断屏蔽”，但 TCB 的锁粒度和访问语义已经从“拿整块 inner 再读写字段”推进到“按 user/sched/signals/debug 访问对应状态”。

## 背景问题

重构前，`TaskControlBlock` 的所有线程本地状态都集中在一个 `UPIntrMutex<TaskControlBlockInner>` 中：

- 用户态资源：`res`、`trap_cx_ppn`
- 调度状态：`task_cx`、`task_status`、`exit_code`
- 信号状态：`signal_trap_cx`、`signal_mask`、`signal_pending`、`clear_child_tid`、`handling_sig`
- 诊断字段：`last_syscall`、SIGCANCEL/非法指令重复计数

这会导致几个实际问题：

1. 访问语义不清晰。很多调用点只是读 `tid`、`status` 或 `signal_mask`，却必须借用整块 TCB inner。
2. 热点路径互相耦合。调度切换、信号投递、futex 唤醒、procfs 诊断会争用同一把 TCB 锁。
3. 借用冲突难定位。一旦出现 `UPIntrFreeCell`/`UPIntrMutex` borrow conflict，很难从字段关系判断是哪类访问互相冲突。
4. 后续拆 PCB/信号/等待语义时缺少边界。TCB 内部状态不先拆开，后续 wait/signal/futex 很容易继续扩大粗锁范围。

一句话根因：TCB 把生命周期、调度、信号和诊断计数放在同一个独占临界区里，导致不同访问者没有锁边界，热点路径只能通过整块 inner 交互。

## 改动范围

本次主要修改以下文件：

- `os/src/task/task.rs`
- `os/src/task/mod.rs`
- `os/src/task/process.rs`
- `os/src/task/processor.rs`
- `os/src/task/manager.rs`
- `os/src/task/futex.rs`
- `os/src/syscall/process.rs`
- `os/src/syscall/thread.rs`
- `os/src/syscall/mod.rs`
- `os/src/trap/mod.rs`
- `os/src/trap/user_trap_riscv64.rs`
- `os/src/trap/user_trap_loongarch64.rs`
- `os/src/fs/pipe.rs`
- `os/src/fs/vfs/procfs.rs`
- `os/src/timer.rs`

`user/src/bin/initcode.rs` 和 `hotspot.md` 是本轮前已有的未归属改动，不属于本次 TCB 重构范围。

## 核心改动

### 1. TCB 物理拆分

`TaskControlBlock` 从单个 inner 拆成：

```rust
pub struct TaskControlBlock {
    pub process: Weak<ProcessControlBlock>,
    pub kstack: KernelStack,
    user: UPIntrMutex<TaskUserState>,
    sched: UPIntrMutex<TaskSchedState>,
    signals: UPIntrMutex<TaskSignalState>,
    last_syscall: AtomicUsize,
    sigcancel_last_pc: AtomicUsize,
    sigcancel_loop_count: AtomicUsize,
    illegal_last_sepc: AtomicUsize,
    illegal_repeat_count: AtomicUsize,
}
```

锁组含义：

- `TaskUserState`：`res`、`trap_cx_ppn`
- `TaskSchedState`：`task_cx`、`task_status`、`exit_code`
- `TaskSignalState`：线程级 signal mask/pending、signal frame、clear child tid、sigsuspend/interrupted 状态
- atomic debug/counter：`last_syscall`、SIGCANCEL 循环计数、非法指令重复计数

本轮完成后，常规路径不再依赖 `TaskControlBlockInner`，代码中也不再保留该类型。

### 2. 新增 TCB helper

新增或强化的 helper 包括：

- user：`tid()`、`user_res_snapshot()`、`ustack_base()`、`ustack_top()`、`trap_cx()`、`with_trap_cx_mut(...)`、`take_user_res()`、`set_ustack_base_and_refresh_trap_cx_ppn(...)`
- sched：`status()`、`set_status(...)`、`exit_code()`、`set_exit_code(...)`、`task_cx_ptr_mut_for_switch(...)`、`task_cx_ptr_for_switch(...)`
- signals：`signal_mask()`、`set_signal_mask(...)`、`update_signal_mask(...)`、`pending_signal()`、`insert_pending_signal(...)`、`remove_pending_signal(...)`、`mark_interrupted()`、`take_interrupted()`、`handling_sig()`、`set_handling_sig(...)`、`clear_child_tid()`、`set_clear_child_tid(...)`、`take_signal_frame(...)`
- debug：`last_syscall()`、`set_last_syscall(...)`、`record_illegal_instruction(...)`、`try_debug_snapshot()`

这些 helper 把调用点从“知道 TCB 内部字段布局”改成“表达自己要访问的领域状态”。

### 3. 调度路径收敛

迁移点：

- `suspend_current_and_run_next`
- `block_current_task`
- `run_tasks`
- timer/futex 唤醒路径

新的路径只通过 sched helper 修改 `TaskStatus` 和取得 `TaskContext` 指针。这样调度状态与 signal/user 资源不再共享同一个整块锁。

收益：

- `task_status` 的访问范围更小。
- futex/timer 唤醒不再借用整块 TCB。
- ready queue 诊断通过 `try_debug_snapshot()` 获取快照，失败时输出 busy，而不是强行借用整块 TCB。

### 4. fork/clone/exec/thread 路径收敛

迁移点：

- `ProcessControlBlock::new`
- `ProcessControlBlock::exec_with_interp`
- `ProcessControlBlock::fork`
- `sys_fork`
- `sys_clone`
- `sys_thread_create`
- `sys_gettid`
- `sys_set_tid_address`
- `sys_waittid`

典型变化：

```text
旧路径：
  task.inner_exclusive_access()
    -> 读 res/tid/ustack_base
    -> 写 signal_mask
    -> 写 trap_cx

新路径：
  task.ustack_base()
  task.signal_mask()
  task.set_signal_mask(...)
  task.with_trap_cx_mut(...)
```

收益：

- fork/clone 不再为了初始化 trap context 或继承 signal mask 拿整块 TCB。
- thread create 只读父线程 user state，初始化新线程 trap context 后再入队。
- clear child tid 与 exit code 进入对应 helper，减少 pthread join/exit 路径的锁耦合。

### 5. signal/trap 路径收敛

迁移点：

- `handle_signals`
- `sys_sigprocmask`
- `sys_sigreturn`
- `sys_rt_sigsuspend`
- RISC-V page fault / illegal instruction 注入信号
- LoongArch page fault 注入信号

`handle_signals` 现在先读取 signal 快照，再根据 pending/action 决定处理路径；实际修改 pending、mask、handler frame、trap context 时只触碰对应 helper。

`sys_sigreturn` 不再持有整块 TCB inner 完成 frame 恢复，而是拆成：

1. 读取当前 trap sepc。
2. 从 signals 中取出 saved trap context。
3. 检查 canary / ucontext。
4. 用 `with_trap_cx_mut` 恢复 trap context。
5. 用 signal helper 恢复或调整 signal mask。

收益：

- signal mask/pending 与 sched/user 状态分离。
- 同步异常注入 SIGSEGV/SIGILL 时只操作线程信号组。
- 非法指令重复计数改 atomic，诊断计数不再扩大 signals 锁。

### 6. procfs/诊断路径收敛

迁移点：

- `/proc/<pid>/stat`
- `/proc/<pid>/task/<tid>/stat`
- ready queue brief
- kernel timer sample

这些路径改为使用 `try_debug_snapshot()` 或短暂读取 trap context helper。失败时允许诊断路径输出 `<busy>`，避免诊断本身制造 TCB borrow conflict。

### 7. pipe/futex/timer 路径收敛

迁移点：

- pipe 写端断开时注入 `SIGPIPE`
- futex wait/wake/requeue 诊断
- itimer 到期唤醒 blocked task

这些路径现在通过 `insert_pending_signal`、`mark_interrupted`、`set_status` 等 helper 修改状态。

## 本次改动的收益

### 1. 降低 TCB 热点锁竞争面

调度状态、信号状态、用户资源以前同处一个锁。现在：

- 调度只碰 `sched`
- trap context/user resource 只碰 `user`
- pending/mask/sigreturn frame 只碰 `signals`
- 诊断计数用 atomic

这使得高频路径之间的逻辑边界变清楚，后续定位 borrow conflict 时可以直接看是哪一组锁，而不是整块 TCB。

### 2. 减少 RefCell/UPIntrMutex borrow conflict 风险

旧代码中常见的模式是：

```text
拿 task inner
  -> 读写 signal
  -> 读写 trap context
  -> 判断 status
  -> 可能调用 wake/schedule/helper
```

这种模式容易在嵌套 helper 或诊断路径里再次借用 TCB。拆分后，即使某条路径需要 signal 和 trap context，也可以按阶段短持不同锁，减少整块 TCB 重入借用概率。

### 3. 为 wait/signal/futex 后续深化改造铺路

本轮没有拆 PCB family/process signals，但 TCB 侧已经把线程级 signal mask/pending、clear child tid、exit code、task status 拆开。下一轮如果继续做：

- process-level signal actions/pending 拆组
- wait/exit/family 拆组
- futex wait EINTR 语义细化

就不需要再先处理 TCB 粗锁这个前置问题。

### 4. 提升代码可读性和访问意图

调用点从：

```rust
let mut inner = task.inner_exclusive_access();
inner.signal_pending.insert(flag);
inner.task_status = TaskStatus::Ready;
```

变成：

```rust
task.insert_pending_signal(flag);
task.set_status(TaskStatus::Ready);
```

读代码时可以直接看出访问的是信号状态还是调度状态。字段归属也更稳定，减少跨模块随意改 TCB 内部布局的可能。

### 5. 诊断路径更安全

procfs 和 ready queue 诊断现在通过 `try_debug_snapshot()` 获取轻量快照。拿不到锁时返回 busy，不会为了输出状态而强制进入整块 TCB 临界区。

这对长测和 hang/debug 场景有价值：诊断不应该成为新的死锁或 borrow conflict 来源。

## 已知边界

1. 本轮没有拆 PCB 的 `memory_set/signal_actions/signal_pending/family/rlimits`，这些仍属于后续阶段。
2. `trap_cx()` 仍返回 `&'static mut TrapContext`，只是访问入口统一到 helper；彻底收敛 unsafe alias 语义需要后续单独设计。
3. 调度切换仍会短暂取得 `task_cx` 指针并释放锁后交给 arch switch，沿用当前单核模型假设。
4. 本轮只跑了一个轻量运行用例；signal/futex/pthread/cyclictest 仍需要更大范围回归。

## 后续建议

1. 跑第三阶段计划中的扩展回归：

```bash
SINGLE_TEST=/musl/basic/fork LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=/musl/basic/wait LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=busybox LOG=INFO timeout 300 bash run.sh -t rv
SINGLE_TEST=cyclictest LOG=INFO timeout 300 bash run.sh -t rv
```

2. 第四阶段优先拆 PCB process-level signal/family：

- `signal_pending`
- `signal_actions`
- pending siginfo
- `parent/children/is_zombie/exit_code/child_wait_event/group_stopped/ptrace_stop_signal`

3. 建议新增一个 CI/static check，禁止在 `os/src/syscall os/src/trap os/src/fs os/src/timer.rs` 中重新引入 TCB `inner_exclusive_access()`。

4. 继续把诊断路径改成 snapshot 风格，保持“诊断不制造新锁冲突”的原则。
