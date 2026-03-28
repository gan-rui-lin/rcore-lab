# exit_group 线程组终止修复 + tkill SIG33 泄露修复

**日期**: 2026/3/25（更新，初版 2026/3/24）

## 罪魁祸首

两个独立的线程相关 bug：

1. **`sys_exit_group` 不杀兄弟线程**：实现等价于 `sys_exit`，非主线程调用 `exit_group` 时主线程不受影响，进程永远不变成 zombie，父进程 `waitpid` 永久阻塞。
2. **`sys_tkill` 的 TID 查找顺序错误**：先全局查找 `pid2process(tid)` 再查当前进程，导致 musl 内部的 `tkill(1, SIG33)` 将 pthread cancel 信号发送到 PID=1 的 initproc 而非当前进程的 tid=1 工作线程。

## 一、问题背景

### exit 与 exit_group 的语义区别

在 Linux 中：
- **`exit`（syscall 93）**：仅终止**当前线程**
- **`exit_group`（syscall 94）**：终止**整个线程组**。glibc/musl 的 `exit()` 和 `_exit()` 实际调用的都是 `exit_group`

Linux 内核的 `do_group_exit()` 实现：
1. 设置 `signal->group_exit_code`
2. `zap_other_threads()` 向线程组内所有其他线程发送 `SIGKILL`
3. 当前线程执行 `do_exit()` 退出

### musl 的 SIG33 (SIGCANCEL)

musl libc 使用信号 33 (`__SIGCANCEL`) 作为 pthread cancel 机制的内部信号：
1. `pthread_cancel()` 设置目标线程的取消标志
2. 通过 `tkill(internal_tid, 33)` 向目标线程发送 SIG33
3. 目标线程的信号处理器检查取消标志并执行栈展开（unwinding）

这个信号绝不应该泄露到进程外部。

### rCore-Lab 的进程/线程结构

```
ProcessControlBlock (进程)
├── pid: PidHandle                    // 全局进程 ID
├── tasks: Vec<Option<Arc<TCB>>>      // 线程列表（索引=内部 tid）
│   ├── tasks[0] = 主线程 (tid=0, gettid() 返回 PID)
│   ├── tasks[1] = 工作线程 (tid=1)
│   └── tasks[N] = 工作线程 (tid=N)
├── signal_actions: SignalActions     // 信号处理器（进程共享）
├── memory_set: MemorySet            // 地址空间（线程共享）
└── fd_table: Vec<...>               // 文件描述符（线程共享）

TaskControlBlock (线程)
├── process: Weak<PCB>               // 所属进程
├── kstack: KernelStack              // 内核栈（每线程独立）
├── signal_mask: SignalFlags          // 信号屏蔽字（每线程独立）
├── signal_pending: SignalFlags       // 待处理信号（每线程独立）
├── clear_child_tid: usize           // futex 唤醒地址（pthread_join 用）
└── exit_code: Option<i32>           // 退出码
```

## 二、Bug #1: exit_group 卡死 —— 调试全过程

### 2.1 复现

在 `hwt-ltp1` 分支 (commit f4cece57) 上运行 LTP 测试：

```bash
SINGLE_TEST=musl-ltp LOG=INFO OFFLINE=1 timeout 180 bash run.sh -f sdcard-rv.img -t rv > ltp1.log 2>&1
```

结果：180 秒后被 timeout kill（exit code 124），测试卡在 `af_alg02`。

### 2.2 从日志发现问题

**第一步：确认卡在哪**

```bash
strings ltp1.log | grep -E "RUN LTP CASE|PASS LTP|FAIL LTP"
```
输出只有 `RUN LTP CASE af_alg02`，没有对应的 PASS/FAIL——说明 af_alg02 没有退出。

**第二步：检查日志末尾**

```
af_alg02.c:65: TBROK: tst_checkpoint_wait(0, 10000) failed: ETIMEDOUT (110)
[ INFO] [timer] check_timer count=2000 time_ms=62992
[ INFO] [timer] check_timer count=8000 time_ms=137847
qemu-system-riscv64: terminating on signal 15
```

只有 timer tick 在跑，没有任何进程活动。说明系统中所有进程都在阻塞等待。

**第三步：追踪 af_alg02 的线程生命周期**

```bash
strings ltp1.log | grep -E "pid=4.*af_alg02.*(clone|exit_group)"
```
```
[clone] pid=4 tid=1 child_cleartid=0xee174
```

af_alg02 (pid=4) 创建了工作线程 tid=1，但日志中 **找不到 tid=1 或 tid=0 的 exit 记录**。

```bash
strings ltp1.log | grep "[exit] pid=4.*af_alg02"
# 无输出！
```

这意味着：af_alg02 的两个线程一个都没退出。

**第四步：追踪 exit_group 调用**

在日志中搜索发现 af_alg02 确实调用了 exit_group（来自 TBROK 后的退出逻辑），但 `exit_group` 只调用了 `sys_exit`：

```
[WARN] [exit_group] pid=4 name=af_alg02 code=32
[INFO] [exit] pid=4 tid=1 name=af_alg02 code=32       ← 只有 tid=1 退出
[INFO] [exit] pid=4 tid=1 clear_child_tid=0xee174
```

**tid=0（主线程）完全没有收到任何终止信号**。

**第五步：理解为什么进程不会变成 zombie**

查看 `exit_current_and_run_next` (os/src/task/mod.rs:230)：

```rust
if tid == 0 || all_threads_exited {
    // 标记进程为 zombie
}
```

- tid=1 退出：`tid != 0`，第一个条件不满足
- tid=0 还活着：`all_threads_exited = false`
- → **进程永远不变成 zombie**
- → 父进程 `waitpid` 永久阻塞
- → 连锁阻塞整个进程树

### 2.3 完整的卡死链

```
pid=4 af_alg02:
  tid=1 (工作线程): exit_group(32) → sys_exit() → 只退出自己
  tid=0 (主线程):   futex checkpoint wait → ETIMEDOUT → 尝试 exit_group → 但还活着
                                                                              ↑ 实际上 tid=0 似乎
                                                                              在 TBROK 后进入了
                                                                              某种用户态循环
pid=3 (busybox sh): waitpid(pid=4) → 永久阻塞（pid=4 不是 zombie）
pid=2 (busybox sh): waitpid(pid=3) → 永久阻塞
pid=1 (initproc):   waitpid(pid=2) → 永久阻塞
→ 整个系统死锁
```

### 2.4 修复

参考 Linux `do_group_exit()` 和已有的 SIGKILL 广播逻辑 (os/src/task/mod.rs:591-610)：

```rust
pub fn sys_exit_group(exit_code: i32) -> ! {
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();

    // 向所有兄弟线程注入 SIGKILL
    {
        let process_inner = process.inner_exclusive_access();
        for other_task in process_inner.tasks.iter().filter_map(|t| t.as_ref()) {
            if !Arc::ptr_eq(other_task, &task) {
                let mut other_inner = other_task.inner_exclusive_access();
                if other_inner.exit_code.is_some() {
                    continue; // 已退出
                }
                other_inner.signal_pending.insert(SignalFlags::SIGKILL);
                // 必须唤醒阻塞线程（如 futex wait）
                if other_inner.task_status == TaskStatus::Blocked {
                    futex_remove_waiter_any(other_task);
                    other_inner.interrupted_by_signal = true;
                    other_inner.task_status = TaskStatus::Ready;
                    drop(other_inner);
                    add_task(other_task.clone());
                }
            }
        }
    }
    drop(process);
    drop(task);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit_group!");
}
```

**关键细节**：仅设置 `signal_pending` 不够——如果兄弟线程阻塞在 futex wait 中，需要 `futex_remove_waiter_any` 将其从等待队列移除并加入就绪队列，否则它永远不会被调度到去处理 SIGKILL。

### 2.5 验证

修复后重跑：
```bash
SINGLE_TEST=musl-ltp LOG=INFO bash run.sh > ltp2.log
```

af_alg02 正常退出：
```
[WARN] [exit_group] pid=3 name=af_alg02 code=34
[INFO] [exit] pid=3 tid=0 name=af_alg02 code=34
FAIL LTP CASE af_alg02 : 34
```

测试继续执行到 `bpf_prog07` 才卡住（因为 BPF syscall 未实现，与线程终止无关）。

## 三、Bug #2: tkill SIG33 泄露 —— 调试全过程

### 3.1 现象

修复 exit_group 后跑全量测试 (`-t all`)，测试能跑到 libctest 的 `pthread_cancel` 系列，但之后 initproc 被杀死：

```
[WARN] [signal] pid=2 name=busybox default handler for signal 33 -> terminate
[WARN] [signal] pid=1 name=initproc default handler for signal 33 -> terminate
[INFO] [exit] pid=1 tid=0 name=initproc code=-33
[kernel] Panicked at src/sync/up.rs:116
```

SIG33 是 musl 内部信号，不应该出现在 busybox (pid=2) 和 initproc (pid=1) 中。

### 3.2 定位

**第一步：找到 SIG33 的来源**

```bash
strings all1.log | grep "sig33\|SIG33\|signum=33" | head -10
```
```
[SYSCALL] kernel:pid[23] sys_sigaction signum=33
[SYSCALL] kernel:pid[23] sys_tkill tid=1 signum=33   ← 发给 tid=1
[SYSCALL] kernel:pid[23] sys_tkill tid=1 signum=33
[SYSCALL] kernel:pid[23] sys_tkill tid=2 signum=33
```

pid=23 执行 `pthread_cancel` 测试，通过 `tkill(tid=1, sig=33)` 发送取消信号。

**第二步：检查 tkill 旧实现**

hwt-ltp1 分支修复前的 `sys_tkill`：

```rust
pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    // 第一步：全局查找 pid2process(tid)
    if let Some(process) = pid2process(tid) {
        // 在这个进程的线程列表里找 target_tid
        let ret = send_signal_to_task_from_list(tid, ...);
        if ret == 0 { return 0; }
    }
    // 第二步：才查当前进程
    let process = current_process();
    let ret = send_signal_to_task_from_list(tid, ...);
    ret
}
```

**问题**：当 pid=23 的线程执行 `tkill(1, 33)` 时：
- `pid2process(1)` → 匹配到 **initproc**（PID=1）！
- 在 initproc 的线程列表中找 tid=1 → 找不到（initproc 只有 tid=0）
- 返回 ESRCH，继续查当前进程 → 找到 tid=1 → 发送成功

看起来应该没问题？但实际上 `send_signal_to_task_from_list` 中的 `task_matches_linux_tid` 有一个映射：

```rust
fn task_matches_linux_tid(process_pid: usize, task: &Arc<TCB>, target_tid: usize) -> bool {
    let internal_tid = res.tid;
    // tid=0 的线程用 PID 匹配
    internal_tid == target_tid || (internal_tid == 0 && process_pid == target_tid)
}
```

当 `target_tid=1, process_pid=1`（initproc）时：`internal_tid == 0 && process_pid == 1 == target_tid` → **匹配成功！** initproc 的主线程（内部 tid=0）被当作 target_tid=1 匹配了，因为 `process_pid == target_tid`。

于是 SIG33 被发送到了 initproc 的主线程！

### 3.3 信号传播链

```
pid=23 pthread_cancel 测试:
  tkill(1, 33)
    → pid2process(1) → initproc
    → task_matches_linux_tid(pid=1, tid=0, target=1) → true!  ← BUG
    → initproc.signal_pending |= SIG33
    → initproc 没有 SIG33 handler → default action = terminate
    → initproc 退出 → kernel panic
```

### 3.4 修复

优先搜索当前进程（最常见场景），只在找不到时才全局查找：

```rust
pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    // 优先搜索当前进程（pthread_cancel 最常见的场景）
    let process = current_process();
    let inner = process.inner_exclusive_access();
    let ret = send_signal_to_task_from_list(tid, process.getpid(), &inner.tasks, flag);
    drop(inner);
    if ret == 0 { return 0; }

    // 回退：全局搜索（跨进程 tkill，较少见）
    if let Some(process) = pid2process(tid as usize) {
        let inner = process.inner_exclusive_access();
        let ret = send_signal_to_task_from_list(tid, process.getpid(), &inner.tasks, flag);
        drop(inner);
        return ret;
    }
    ret
}
```

这样 `tkill(1, 33)` 会先在当前进程找 tid=1 → 找到 → 正确投递。不会再误匹配到 initproc。

## 四、影响范围

### exit_group 修复影响

所有多线程程序中非主线程调用 `exit_group`/`exit()` 的场景：
- LTP 测试框架（所有使用线程的用例）
- musl `pthread_cancel` 系列（取消后调用 exit）
- 任何 C 程序调用 `exit()` 的多线程场景

### tkill 修复影响

所有使用 `pthread_cancel` 的程序，以及任何在进程内使用 `tkill` 发信号的场景。尤其当内部 tid 恰好等于某个全局 PID 时会触发（例如 tid=1 匹配 initproc 的 PID=1）。

## 五、待关注

1. **主仓库 (loongarch-net) 的 tkill 实现**用内部 tid 直接索引 `inner.tasks[tid]`，避免了全局查找问题，但当 Linux TID 与内部 tid 不一致时可能有其他问题
2. **bpf_prog07 及后续 LTP 测试卡住**：BPF/网络相关 syscall 未实现，需要在测试脚本中跳过或添加超时
3. **pthread_cancel 测试本身 timeout**：可能是 pthread cancel 机制实现不完整（SIG33 信号处理、线程取消点等），需要单独调试
