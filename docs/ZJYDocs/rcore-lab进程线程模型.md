# rCore-Lab 进程线程模型

**日期**: 2026/3/25（更新，初版 2026/3/24）

## 1. 概述

rCore-Lab 采用 **两层数据结构** 实现进程与线程：`ProcessControlBlock (PCB)` 代表进程，`TaskControlBlock (TCB)` 代表线程（内核中称为"任务"）。一个进程可以拥有多个线程，所有线程共享进程的地址空间、文件描述符表和信号处理器，但各自拥有独立的内核栈、用户栈、信号掩码和执行上下文。

这与 Linux 的模型高度相似——Linux 中每个线程都是一个 `task_struct`，同一线程组（thread group）的线程共享 `mm_struct`（地址空间）、`files_struct`（文件表）和 `sighand_struct`（信号处理器），但有各自的 `signal_struct`（信号掩码/挂起列表）和内核栈。

## 2. 数据结构层次

### 2.1 ProcessControlBlock（进程控制块）

定义于 `os/src/task/process.rs`：

```
ProcessControlBlock
├── pid: PidHandle                          // 全局唯一进程 ID
└── inner: UPIntrFreeCell<ProcessControlBlockInner>
    ├── is_zombie: bool                     // 是否为僵尸进程
    ├── memory_set: MemorySet               // 虚拟地址空间（所有线程共享）
    ├── parent: Option<Weak<PCB>>           // 父进程
    ├── children: Vec<Arc<PCB>>             // 子进程列表
    ├── exit_code: i32                      // 进程退出码
    ├── fd_table: Vec<Option<Arc<dyn File>>>// 文件描述符表（所有线程共享）
    ├── signal_actions: SignalActions        // 信号处理器表（所有线程共享）
    ├── signal_pending: SignalFlags          // 进程级挂起信号
    ├── tasks: Vec<Option<Arc<TCB>>>        // 线程列表（下标即 tid）
    ├── task_res_allocator: RecycleAllocator // TID 分配器
    ├── cwd: String                         // 工作目录（所有线程共享）
    └── ...
```

**关键点**：`tasks` 向量的下标就是线程的 tid。`tasks[0]` 是主线程（线程组 leader），`tasks[1]`, `tasks[2]`... 是通过 `clone(CLONE_THREAD)` 创建的工作线程。

### 2.2 TaskControlBlock（任务控制块/线程控制块）

定义于 `os/src/task/task.rs`：

```
TaskControlBlock
├── process: Weak<ProcessControlBlock>      // 归属进程（弱引用，避免循环引用）
├── kstack: KernelStack                     // 内核栈（每线程独立）
└── inner: UPIntrFreeCell<TaskControlBlockInner>
    ├── res: Option<TaskUserRes>            // 用户态资源
    │   ├── tid: usize                      // 线程 ID（进程内唯一）
    │   └── ustack_base: usize             // 用户栈基址
    ├── trap_cx_ppn: PhysPageNum            // trap 上下文物理页
    ├── task_cx: TaskContext                 // 任务切换上下文（ra, sp, s0-s11）
    ├── task_status: TaskStatus             // Ready / Running / Blocked
    ├── exit_code: Option<i32>              // 线程退出码（None = 未退出）
    ├── signal_mask: SignalFlags             // 信号掩码（每线程独立）
    ├── signal_pending: SignalFlags          // 线程级挂起信号
    ├── clear_child_tid: usize              // exit 时清零并 futex_wake 的地址
    ├── handling_sig: isize                 // 当前正在处理的信号编号
    └── ...
```

### 2.3 共享 vs 独立资源对比

| 资源 | 作用域 | 说明 |
|------|--------|------|
| 地址空间 (MemorySet) | **进程** | 所有线程共享同一页表 |
| 文件描述符表 | **进程** | 所有线程共享，open/close 互相可见 |
| 信号处理器 (SignalActions) | **进程** | sigaction 设置的 handler 全组共享 |
| 工作目录 (cwd) | **进程** | chdir 影响所有线程 |
| 信号掩码 (signal_mask) | **线程** | sigprocmask 只影响当前线程 |
| 信号挂起 (signal_pending) | **两级** | 进程级 + 线程级各一份 |
| 用户栈 | **线程** | 每个线程有独立栈区域 |
| 内核栈 | **线程** | 每个线程有独立内核栈 |
| TLS (线程本地存储) | **线程** | 通过 tp 寄存器指向，clone 时设置 |
| trap 上下文 | **线程** | 每线程独立的寄存器保存区 |

## 3. TID 分配与映射

### 3.1 分配机制

`RecycleAllocator`（定义于 `os/src/task/id.rs`）为每个进程维护一个 TID 分配器：

```rust
pub struct RecycleAllocator {
    current: usize,       // 下一个可分配的 TID
    recycled: Vec<usize>, // 已回收的 TID，优先复用
}
```

- 进程创建时，主线程获得 `tid=0`
- `clone(CLONE_THREAD)` 创建的线程获得 `tid=1, 2, 3...`
- 线程退出后 TID 可被回收复用

### 3.2 TID 与 PID 的映射

rCore-Lab 采用**进程内 TID** 而非 Linux 的全局唯一 TID：

```rust
// sys_gettid 的映射规则
fn sys_gettid() -> isize {
    if tid == 0 { process_pid }  // 主线程返回 PID（与 Linux 一致）
    else { tid }                  // 工作线程返回进程内 tid（1, 2, 3...）
}
```

**与 Linux 的差异**：Linux 中每个线程有全局唯一的 tid（等于内核 task 的 pid），而 rCore-Lab 的非主线程 tid 只是进程内的索引号。

### 3.3 两个分支的 TID 实现对比

主仓库 (loongarch-net) 和 hwt-ltp1 分支对 TID 的处理方式不同：

| 操作 | Linux 标准 | 主仓库 (loongarch-net) | hwt-ltp1 分支 |
|------|-----------|----------------------|--------------|
| `gettid()` 主线程 | 返回 PID | 返回 **0** | 返回 **PID** |
| `gettid()` 工作线程 | 返回全局 TID | 返回内部 tid (1,2,...) | 返回内部 tid (1,2,...) |
| `set_tid_address` 返回值 | 返回全局 TID | 返回内部 tid | 返回 tid（主线程返回 PID）|
| `clone` 返回值/ctid 写入 | 返回全局 TID | 返回内部 tid | 返回内部 tid |
| `tkill(tid, sig)` 查找 | 全局 TID 查找 | `tasks[tid]` 直接索引 | `task_matches_linux_tid` 匹配 |

**主仓库方案的自洽性**：虽然返回的 tid 不符合 Linux 标准，但内部一致——`clone` 返回内部 tid → musl 存入 `pthread->tid` → `tkill` 用同一个值索引 `tasks` 数组 → 正确命中目标线程。只要不发生跨进程 tkill（如 `tgkill(other_pid, tid, sig)`），这套方案可以工作。

**hwt-ltp1 方案的映射函数**：由于主线程的 gettid 返回 PID（而非内部 tid=0），tkill 需要额外的映射逻辑：

```rust
fn task_matches_linux_tid(process_pid: usize, task: &TCB, target_tid: usize) -> bool {
    internal_tid == target_tid ||
    (internal_tid == 0 && process_pid == target_tid)  // 主线程也匹配 PID
}
```

**hwt-ltp1 的 tkill 陷阱**：此映射函数在全局查找时会误匹配。例如 `tkill(1, 33)` 想发给当前进程的工作线程 tid=1，但 `pid2process(1)` 找到 initproc 后，`task_matches_linux_tid(pid=1, tid=0, target=1)` 因 `pid == target` 而匹配 initproc 的主线程——SIG33 被发给了 init 进程！修复方式是优先搜索当前进程（详见 exit_group 调试文档）。

**主仓库 gettid 返回 0 的影响**：musl 启动时通过 `set_tid_address(&self->tid)` 获取 TID，返回值写入 `self->tid`。如果主线程返回 0，musl 记录 `self->tid=0`。这在大多数场景下不影响功能（musl 通常用 `getpid()` 获取进程 ID），但在需要主线程 TID 的场景（如 `pthread_self` 比较、robust mutex 等）可能出问题。

## 4. 进程/线程创建

### 4.1 fork（创建新进程）

```
sys_fork() / sys_clone(flags=SIGCHLD)
├── 1. 调用 process.fork()
│   ├── 创建新 ProcessControlBlock
│   ├── 深拷贝地址空间 (MemorySet::from_existed_user)
│   ├── 克隆文件描述符表
│   └── 克隆信号处理器
├── 2. 在新进程中创建 tid=0 的主线程
├── 3. 设置子进程 trap 上下文（返回值=0）
└── 4. 加入调度队列
```

**特点**：
- 新进程有独立地址空间（COW 语义由 MemorySet 实现）
- 新进程有独立的 PID 和 TID 空间
- 父子进程通过 `waitpid` 同步

### 4.2 clone(CLONE_THREAD)（创建新线程）

```
sys_clone(flags=CLONE_THREAD|CLONE_VM|...)
├── 1. 分配新 TID（进程内）
├── 2. 创建 TaskControlBlock（同一进程内）
├── 3. 如果 stack 非空，使用用户指定的栈
│   否则在默认位置分配新栈
├── 4. 继承父线程的信号掩码
├── 5. 处理 CLONE_SETTLS → 设置 tp 寄存器
├── 6. 处理 CLONE_PARENT_SETTID → 写 tid 到父线程地址
├── 7. 处理 CLONE_CHILD_SETTID → 写 tid 到子线程地址
├── 8. 处理 CLONE_CHILD_CLEARTID → 记录 clear_child_tid 地址
└── 9. 加入调度队列
```

**特点**：
- 新线程共享进程的地址空间（同一 MemorySet/token）
- 每个线程的用户栈通过 `ustack_base + tid * stride` 计算偏移
- `clear_child_tid` 机制使得 musl 的 `pthread_join` 可以通过 futex 等待线程退出

### 4.3 线程栈布局

```
高地址 ←────────────────────────────────────→ 低地址

tid=0:  [guard page][  user stack (8KB)  ] @ ustack_base
tid=1:  [guard page][  user stack (8KB)  ] @ ustack_base + 1 * stride
tid=2:  [guard page][  user stack (8KB)  ] @ ustack_base + 2 * stride
...

stride = PAGE_SIZE(guard) + USER_STACK_SIZE
```

注意：如果 `clone` 时用户指定了 `stack` 参数（pthread_create 必然会指定），则使用用户分配的栈而非内核默认位置。

## 5. 线程退出与进程终止

### 5.1 sys_exit（单线程退出）

`exit_current_and_run_next(exit_code)` 的流程：

```
1. 处理 clear_child_tid
   ├── 将 *clear_child_tid 写为 0
   ├── futex_wake(private_key, 1)  // 唤醒 pthread_join 等待者
   └── futex_wake(shared_key, 1)

2. 标记线程退出
   ├── task_inner.exit_code = Some(exit_code)
   └── task_inner.res = None  // 释放用户栈等资源

3. 检查是否整个进程退出
   └── if tid == 0 || all_threads_exited:
       ├── 标记进程为 zombie
       ├── 向父进程发送 SIGCHLD
       ├── 将子进程托管给 initproc
       ├── 回收所有线程的用户资源
       ├── 回收内存页和文件描述符
       └── 清理 tasks 列表

4. 切换到下一个可运行任务
```

**已知问题**：`tid == 0` 的条件意味着主线程退出会立即触发进程清理，即使其他线程还在运行。Linux 的行为是：主线程退出后变成一个 zombie stub，但进程资源保留到所有线程都退出。不过在实践中，C 库的 `exit()` 总是调用 `exit_group` 而非 `exit`，所以这个问题很少被触发。

### 5.2 sys_exit_group（线程组退出）

修复后的 `exit_group` 实现了 Linux `do_group_exit()` 的语义：

```
1. 向所有兄弟线程注入 SIGKILL
   ├── 遍历 process.tasks
   ├── 跳过已退出的线程（exit_code.is_some()）
   ├── 设置 signal_pending |= SIGKILL
   └── 如果线程被阻塞（futex wait 等），强制唤醒

2. 退出当前线程
   └── exit_current_and_run_next(exit_code)
```

这确保了即使非主线程调用 `exit()` / `_exit()`，主线程和其他兄弟线程也会在下次调度时被 SIGKILL 终止。

### 5.3 waitpid（进程等待）

`sys_waitpid` 只操作**子进程**（process.children），不涉及线程：

```rust
// 在 children 中查找匹配的 zombie 子进程
inner.children.iter().find(|p| p.is_zombie && matches(pid))
```

线程退出的等待通过 **futex + clear_child_tid** 机制实现（musl 的 `pthread_join`）。

## 6. 信号模型

### 6.1 两级信号挂起

```
进程级 signal_pending（process_inner.signal_pending）
  └── kill(pid, sig) 产生的信号放这里
  └── 任何未屏蔽该信号的线程都可以处理

线程级 signal_pending（task_inner.signal_pending）
  └── tkill(tid, sig) / tgkill(tgid, tid, sig) 产生的信号放这里
  └── 只有目标线程可以处理
```

### 6.2 信号处理优先级

`handle_signals()` 中的处理顺序：

1. **SIGKILL 最优先**：无论在进程级还是线程级，SIGKILL 立即处理
2. 线程级挂起信号优先于进程级
3. 受信号掩码 `signal_mask` 过滤——每个线程可以独立屏蔽信号
4. SIGKILL 和 SIGSTOP **不可被屏蔽**

### 6.3 SIGKILL 的线程组广播

当 SIGKILL 被投递给任一线程时：

```rust
// 注入 SIGKILL 到所有兄弟线程
for other_task in process.tasks {
    other_inner.signal_pending.insert(SIGKILL);
    if blocked: unblock → ready → add_task
}
// 当前线程立即退出
exit_current_and_run_next(-(SIGKILL));
```

这确保了 `kill -9` 能终止整个进程的所有线程。

### 6.4 信号处理器的 trampoline 机制

当用户注册了信号处理器（`sa_handler != SIG_DFL/SIG_IGN`）时：

```
1. 保存当前 trap 上下文到 signal_trap_cx
2. 在用户栈上推送 SignalContext（ucontext + siginfo）
3. 修改 trap 上下文：
   ├── sepc = handler 地址
   ├── a0 = signum
   ├── a1 = &siginfo
   ├── a2 = &ucontext
   └── ra = sa_restorer（信号返回 trampoline）
4. 返回用户态执行 handler
5. handler 返回时跳转到 sa_restorer
6. sa_restorer 调用 sys_sigreturn
7. sys_sigreturn 恢复原始 trap 上下文
```

## 7. 调度模型

### 7.1 全局 FIFO 队列

`TaskManager`（`os/src/task/manager.rs`）维护一个简单的 FIFO 就绪队列：

```rust
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>
}
```

- **进程和线程混合调度**：所有 Ready 状态的 TCB 在同一队列中，不区分进程和线程
- `add_task()`: 入队
- `fetch_task()`: 出队（FIFO）
- 无优先级区分

### 7.2 处理器调度循环

```rust
fn run_tasks() {
    loop {
        if let Some(task) = fetch_task() {
            switch_to(task);
        } else {
            // 无就绪任务，自旋等待
        }
    }
}
```

单核模型，无 SMP 支持。

## 8. 已知问题与改进方向

### 8.1 已修复

| 问题 | 影响 | 修复 |
|------|------|------|
| `exit_group` 不杀死兄弟线程 | LTP af_alg02 等多线程测试卡死 | 向兄弟线程注入 SIGKILL |
| hwt-ltp1: `tkill` 全局查找优先 | SIG33 泄露到 initproc，kernel panic | tkill 优先搜索当前进程 |

### 8.2 已知但暂不影响实际运行

| 问题 | 说明 | 影响 |
|------|------|------|
| TID 非全局唯一 | 工作线程的 tid 是进程内索引（1,2,3...），不是全局唯一 | 内部一致（tkill/ptid/ctid 用同一套编号），但不支持 `/proc/[tid]` 等需要全局 tid 的场景 |
| 主线程退出立即清理进程 | `tid==0` 退出触发 `is_zombie` + 资源回收 | C 库 `exit()` 总是用 `exit_group`，不会单独退 `tid=0`；但 `pthread_exit()` 从主线程调用理论上会出问题 |
| 主仓库 `gettid` 主线程返回 0 | musl 记录 `self->tid=0`，影响 robust mutex、`pthread_self` 等 | 尚未触发明显错误，但 glibc-la 可能更敏感 |
| `sys_tgkill` 不支持跨进程 | 限制了 `tgid == current_pid` | 跨进程发信号可用 `sys_kill`，LTP 测试暂未触发 |
| waitpid busy-wait | 没有进入 Blocked 状态，而是 `suspend_current_and_run_next()` 轮询 | 浪费 CPU，但功能正确 |

### 8.3 glibc-la（LoongArch glibc）特别关注

glibc 对 TID 语义的依赖比 musl 更严格，以下差异在 LA + glibc 环境中更容易暴露问题：

1. **主线程 `gettid` 返回 0**：glibc 的 `nptl` 线程库在 `__pthread_initialize_minimal` 中通过 `set_tid_address` 获取主线程 TID，返回 0 可能导致后续 `THREAD_GETMEM(tid)` 异常
2. **clone 参数顺序**：LoongArch64 的 clone 系统调用 `args[3]=ctid, args[4]=tls`（与 RISC-V 的 `args[3]=tls, args[4]=ctid` 相反），已通过 `#[cfg(target_arch)]` 适配
3. **TLS 初始化**：glibc 的 TLS 模型比 musl 更复杂（使用 `DTV` 动态线程向量），对 tp 寄存器的设置更敏感
4. **robust futex**：glibc 的 `pthread_mutex_lock` 使用 robust list，需要 `set_robust_list` 系统调用（当前返回 0 但未真正实现）

### 8.4 未来改进方向

1. **全局 TID 分配**：让每个线程都有全局唯一的 tid，与 PID 共享同一个 ID 空间（类似 Linux 的 `alloc_pid()`），从而正确支持 `/proc/[tid]`、跨进程 `tgkill`、`waitid(P_PID, tid)` 等语义。

2. **主线程 zombie stub**：主线程退出时不立即回收进程资源，而是保留一个 stub 直到所有线程都退出。

3. **waitpid 事件驱动**：将 waitpid 从 busy-wait 改为基于 SIGCHLD 信号的事件驱动等待，减少 CPU 浪费。

4. **进程组（Process Group）**：目前 `setpgid` 有基本实现但不完善，无法支持 job control（`Ctrl+C` 发 SIGINT 给前台进程组等）。

5. **CFS 调度器**：当前 FIFO 调度在多线程场景下不够公平，线程数多的进程会获得更多 CPU 时间。
