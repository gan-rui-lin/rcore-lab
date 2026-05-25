# PCB/TCB 中断安全锁拆分重构说明

## 背景

当前 rcore-lab 仍大量使用 `UPIntrFreeCell<T>` 作为单核场景下的中断屏蔽型内部可变性原语。这个设计在早期很直接：进入临界区时关闭中断，借助 `RefCell` 保证同一份内核状态不会被重复可变借用。

随着进程、线程、信号、文件描述符、内存映射、等待/退出语义逐步补齐，`ProcessControlBlockInner` 和 `TaskControlBlockInner` 变成了热点大结构。很多路径为了读一个 fd、一个 token、一个任务状态，都会拿到整块 PCB/TCB 的独占访问。这会带来三个问题：

1. 临界区语义过粗，读路径和写路径没有表达差异。
2. fd I/O、VFS 操作、用户缓冲区转换等路径容易在拿着 PCB 锁时继续做较重操作。
3. `RefCell` borrow conflict 的排查成本高，热点路径之间的真实依赖关系不清晰。

本轮重构的目标不是一步到位实现完整 SMP 锁模型，也不是立即改造 VFS、inode、lwext4 的全局锁，而是先把 PCB/TCB 的锁语义从“裸 `UPIntrFreeCell`”迁移到显式的中断安全锁抽象，并开始收敛热点访问 API。

## 重构目的

本轮重构服务于四个目的。

### 1. 明确锁语义

`UPIntrFreeCell` 本质上是一种“关闭中断 + 内部可变性”的底层机制，但名字并不表达互斥锁或读写锁语义。新增 `UPIntrMutex<T>` 和 `UPIntrRwLock<T>` 后，上层代码可以按访问模型选择：

- `UPIntrMutex<T>`：短临界区、独占修改、小状态集合。
- `UPIntrRwLock<T>`：读多写少状态，为后续 memory/fs/identity 等锁组拆分准备接口。
- `UPIntrFreeCell<T>`：继续保留，兼容已有路径和底层实现。

### 2. 缩短热点路径持锁时间

文件读写是当前非常敏感的路径。原来 `sys_read`/`sys_write` 往往先拿 PCB inner，再从 fd table 里取文件对象，然后继续做 file 状态检查、用户缓冲区转换和真实 `read/write`。

本轮把这类路径改为：

```text
短锁读取 fd_table
  -> clone Arc<dyn File>
  -> 释放 PCB 锁
  -> 执行 VFS/File read/write/ioctl 等重操作
```

这符合后续锁顺序要求：PCB 锁只用来拿快照，不在持锁状态下执行可能较慢的 VFS 或用户内存大循环操作。

### 3. 为 PCB/TCB 物理拆组铺路

计划中的最终结构是按访问者和热点领域拆组，例如：

- `memory`：`memory_set`、`heap_bottom`、`program_brk`、`mmap_base`、`tls_area`
- `fs`：`fd_table`、`cwd`、`root_dir`
- `signals`：进程级 pending signal、siginfo、signal actions
- `threads`：`tasks`、`task_res_allocator`
- `family`：`parent`、`children`、zombie/exit/wait/vfork/job-control 状态
- `identity`：`name`、uid/gid/caps、nice、session/pgid、ptrace 标志
- `sync_objects`：用户态 mutex/semaphore/condvar 列表
- `limits_timers`：rlimit、interval timer、real timer 状态

但当前仓库中 `inner_exclusive_access()` 调用点很多，直接把结构物理拆开会一次性牵动 syscall、task、signal、fs、procfs、net 等大量模块，风险较高。因此本轮先建立显式锁类型和访问 helper，后续再按模块逐步把直接字段访问迁移到对应锁组。

### 4. 保持单核中断安全模型

本阶段仍然坚持“单核 + 中断屏蔽”假设，不引入完整 SMP 语义。也就是说，新锁不是为了替代未来 SMP 设计，而是把当前单核临界区的语义整理清楚，降低 borrow conflict 和热点路径锁粒度问题。

## 重构范围

本轮实际修改范围控制在 PCB/TCB 及其直接热点访问路径。

### 1. 同步原语

关键文件：

- `os/src/sync/up.rs`
- `os/src/sync/mod.rs`

新增内容：

- `UPIntrRef<'a, T>`：中断屏蔽下的共享借用 guard。
- `UPIntrMutex<T>`：基于 `UPIntrFreeCell<T>` 的互斥锁语义封装。
- `UPIntrRwLock<T>`：基于 `UPIntrFreeCell<T>` 的读写锁语义封装。
- `UPIntrMutexGuard`、`UPIntrRwLockReadGuard`、`UPIntrRwLockWriteGuard` 类型别名。
- `shared_access()` / `try_shared_access()`，支持只读借用。

`UPIntrFreeCell` 没有删除，仍作为底层机制和兼容入口保留。

### 2. ProcessControlBlock

关键文件：

- `os/src/task/process.rs`
- `os/src/task/id.rs`

已完成：

- `ProcessControlBlock.inner` 从 `UPIntrFreeCell<ProcessControlBlockInner>` 改为 `UPIntrMutex<ProcessControlBlockInner>`。
- `inner_exclusive_access()` 改为返回 `UPIntrMutexGuard`。
- `try_inner_exclusive_access()` 改为走 `try_lock()`。
- `TaskUserRes::new/dealloc_tid` 改为通过 `process.alloc_tid()` / `process.dealloc_tid()` 操作 tid allocator。

新增 PCB 高层访问 API：

- `get_user_token()`
- `thread_count()`
- `get_task(tid)`
- `tasks_snapshot()`
- `alloc_tid()` / `dealloc_tid(tid)`
- `name()`
- `cwd()`
- `root_dir()`
- `get_file(fd)`
- `alloc_fd()`
- `set_fd(fd, file)`
- `take_fd(fd)`
- `with_memory_set(...)`
- `with_memory_set_mut(...)`

这些 API 的核心作用是把“调用者想做什么”表达出来，而不是让调用者直接拿整块 PCB inner。

### 3. TaskControlBlock

关键文件：

- `os/src/task/task.rs`
- `os/src/task/manager.rs`
- `os/src/task/processor.rs`

已完成：

- `TaskControlBlock.inner` 从 `UPIntrFreeCell<TaskControlBlockInner>` 改为 `UPIntrMutex<TaskControlBlockInner>`。
- `inner_exclusive_access()` 改为返回 `UPIntrMutexGuard`。
- `try_inner_exclusive_access()` 改为走 `try_lock()`。
- `current_trap_cx()` 改为通过 `task.trap_cx()` 获取 trap context。
- `wakeup_task()` 改为通过 `task.set_status(TaskStatus::Ready)` 修改状态。

新增 TCB 高层访问 API：

- `get_user_token()`
- `tid()`
- `trap_cx()`
- `status()`
- `set_status(status)`
- `exit_code()`
- `set_exit_code(exit_code)`
- `last_syscall()`
- `set_last_syscall(syscall_id)`

### 4. fd 热点路径

关键文件：

- `os/src/syscall/fs.rs`

已迁移路径：

- `sys_write`
- `sys_read`
- `sys_close`

这三条路径现在只在 PCB 锁内 clone/take `Arc<dyn File>`，释放 PCB 锁后再执行文件对象上的检查和 I/O。这样可以避免 `fd_table` 访问和 VFS 文件操作共用同一段 PCB 独占临界区。

## 本轮重心

本轮重心可以概括为一句话：

先把 PCB/TCB 的粗锁入口语义显式化，并优先迁移 fd I/O 热点路径，减少持 PCB 锁做重操作的情况。

因此，本轮不是完整的字段物理拆分，而是“可编译、可回归、可继续拆”的中间层：

1. 底层已有 `UPIntrFreeCell` 保持兼容。
2. 新代码优先使用 `UPIntrMutex` / `UPIntrRwLock` 语义。
3. PCB/TCB 对外逐步暴露 helper。
4. 热点调用点先从 helper 迁移，减少直接拿 inner。
5. 后续再把 helper 背后的存储从单个 inner 移到多个锁组。

这个顺序可以避免一次性重写数百个调用点导致 fork/exec/wait/signal/fs 等基础行为同时变动。

## 锁顺序约束

后续继续迁移时需要保持以下规则：

1. 全局队列或全局表只用于 clone `Arc` 快照，随后立即释放。
2. 需要同时访问进程和线程时，按 `Process` 后 `Task` 的顺序。
3. 多个 `Process` 需要同时访问时，按 pid 从小到大。
4. 不在持 PCB/TCB 锁时执行调度切换、阻塞等待、VFS/lwext4 I/O、用户内存大循环复制。
5. fd 查找路径只在进程锁内 clone/take `Arc<dyn File>`，文件对象操作放到锁外。

## 验证

已执行编译验证：

```bash
make -C os rv
```

结果：通过。构建过程中存在 vendor crate 的既有 warning，没有新增编译错误。

```bash
make -C os la
```

结果：通过。构建过程中存在 vendor/arch 的既有 warning，没有新增编译错误。

曾尝试直接执行：

```bash
cargo check --target riscv64gc-unknown-none-elf
```

该入口因 debug initcode include 缺失失败，属于当前仓库构建入口差异；本次以 `make -C os rv` 和 `make -C os la` 作为有效编译验证。

## 未覆盖范围

以下内容本轮没有直接完成：

1. 没有把 `ProcessControlBlockInner` 物理拆成 `ProcessMemory`、`ProcessFs`、`ProcessSignals` 等多个结构。
2. 没有把 `TaskControlBlockInner` 物理拆成 `TaskUserState`、`TaskSchedState`、`TaskSignalState`。
3. 没有把 `last_syscall`、cancel/illegal loop counters 改为 atomic 标量。
4. 没有改造 VFS/inode/lwext4 全局锁。
5. 没有引入真正 sleep lock。
6. 没有完成所有 syscall、procfs、net、signal 路径的 helper 化迁移。
7. 没有运行 `SINGLE_TEST=musl`、`SINGLE_TEST=glibc` 等长测。

这些都应作为后续阶段继续推进。

## 后续路线

建议按下面顺序继续做。

### 1. 收敛直接 inner 访问

优先把高频、低风险读路径替换为 helper：

- `process.name()`
- `process.cwd()`
- `process.root_dir()`
- `process.get_user_token()`
- `process.thread_count()`
- `process.tasks_snapshot()`
- `task.status()`
- `task.tid()`

目标是减少 `inner_exclusive_access()` 在 syscall 层的出现频率。

### 2. 扩展 fd helper

继续迁移：

- `dup/dup3`
- `fcntl`
- `poll/select/ppoll/pselect`
- `ioctl`
- `stat/fstat`
- `getdents`
- `close_range`

所有路径遵循“锁内拿 fd 快照，锁外做文件操作”的规则。

### 3. 物理拆分 ProcessControlBlockInner

当访问点收敛到 helper 后，再把 PCB 改成多个字段锁组：

```rust
pub struct ProcessControlBlock {
    pub pid: PidHandle,
    memory: UPIntrRwLock<ProcessMemory>,
    fs: UPIntrRwLock<ProcessFs>,
    signals: UPIntrMutex<ProcessSignals>,
    threads: UPIntrMutex<ProcessThreads>,
    family: UPIntrMutex<ProcessFamily>,
    identity: UPIntrRwLock<ProcessIdentity>,
    sync_objects: UPIntrMutex<ProcessSyncObjects>,
    limits_timers: UPIntrMutex<ProcessLimitsTimers>,
}
```

拆分时应优先保证 fork/exec/exit/wait/brk/mmap/fd I/O 能编译并通过基础回归。

### 4. 物理拆分 TaskControlBlockInner

TCB 可拆为：

```rust
pub struct TaskControlBlock {
    pub process: Weak<ProcessControlBlock>,
    pub kstack: KernelStack,
    user: UPIntrMutex<TaskUserState>,
    sched: UPIntrMutex<TaskSchedState>,
    signals: UPIntrMutex<TaskSignalState>,
}
```

`last_syscall` 等 debug/counter 字段再单独评估是否改为 atomic。

### 5. 增加运行回归

建议后续验证矩阵：

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=/musl/basic/fork LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=/musl/basic/wait LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 bash run.sh -t rv
SINGLE_TEST=busybox LOG=INFO timeout 300 bash run.sh -t rv
SINGLE_TEST=cyclictest LOG=INFO timeout 300 bash run.sh -t rv
```

性能观察可继续使用：

```bash
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f <rv-img> -t rv
```

关注点包括：

- 是否还有 `UPIntrFreeCell borrow conflict`。
- fd I/O 是否出现 EBADF/EFAULT/EINTR 顺序回归。
- fork/exec/wait 是否卡住。
- signal/setitimer 是否出现 pending signal 丢失或重复处理。
- tmp-iozone 是否 panic、hang 或 throughput 明显回退。

## 总结

本轮重构完成的是 PCB/TCB 锁拆分的第一块地基：新增中断安全互斥锁/读写锁抽象，把 PCB/TCB 从裸 `UPIntrFreeCell` 迁移到 `UPIntrMutex`，并将 fd 读写关闭路径改为短锁获取文件对象、锁外执行 I/O。

它的价值不在于一次性消灭所有粗粒度锁，而在于把后续拆分的接口边界立起来：调用者逐步从“拿整块 inner”转向“请求具体能力”，之后才能安全地把 memory、fs、signals、threads、family 等状态拆到独立锁组中。
