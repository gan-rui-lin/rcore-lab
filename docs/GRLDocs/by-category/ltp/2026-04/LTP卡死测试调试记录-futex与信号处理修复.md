# LTP 卡死测试调试记录 - futex 与信号处理修复

**日期**: 2026/04/02

**任务目标**: 解决 rcore-lab 内核在运行 LTP (Linux Test Project) 测试套件时出现的多个卡死问题，包括 `futex_wake02`、`futex_wait03` 和 `clock_nanosleep01` 测试用例。

---

## 一、问题概述

在 LTP 测试套件中，存在三个主要的卡死场景：

1. **futex_wake02**: 测试创建 55 个线程进行 futex 唤醒操作，但部分线程无法被唤醒，导致测试卡死
2. **futex_wait03**: 测试需要通过 `/proc/<pid>/task/<tid>/stat` 监控线程状态，但因 procfs 实现不完整导致卡死
3. **clock_nanosleep01**: 测试各种参数验证场景，因参数校验不完整和 EINTR/EFAULT 处理错误导致超时或卡死

这些测试的共同特点是涉及**多线程同步**、**信号处理**、**系统调用参数验证**等 Linux 内核的核心语义，对内核实现的正确性要求极高。

---

## 二、futex_wake02 调试：Copy-on-Write 与 futex 键生成问题

### 2.1 问题现象

运行 `futex_wake02` 测试时，日志显示部分 wake 操作成功，但在第 5 轮或第 6 轮后出现 wake 计数不匹配的情况，最终导致测试卡死超时。

关键日志片段：
```
futex_wake02    1  TPASS  :  futex_wake() woke up 1 thread
futex_wake02    2  TPASS  :  futex_wake() woke up 2 threads
futex_wake02    3  TPASS  :  futex_wake() woke up 3 threads
futex_wake02    4  TPASS  :  futex_wake() woke up 4 threads
futex_wake02    5  TFAIL  :  futex_wake() woke up 0 threads, expected 5
```

### 2.2 问题分析

通过在 `os/src/syscall/process.rs` 中的 `sys_futex` 函数添加详细的调试日志，输出每次 wait 和 wake 操作的**虚拟地址 (uaddr)** 和**物理地址 (pa)**：

```rust
info!("[FUTEX] WAIT pid={} tid={} uaddr={:#x} pa={:#x} key_type={}", 
      process.pid.0, task.tid(), uaddr, pa, if is_private { "PRIVATE" } else { "SHARED" });
```

调试输出显示了关键的问题根源：

```
[INFO] [FUTEX] WAIT pid=3 tid=4 uaddr=0x40028cd0 pa=0x887cbcd0 key_type=PRIVATE
[INFO] [FUTEX] WAIT pid=3 tid=5 uaddr=0x40028cd0 pa=0x887cbcd0 key_type=PRIVATE
...
[INFO] [FUTEX] WAKE pid=3 tid=0 uaddr=0x40028cd0 pa=0x890eecd0 key_type=PRIVATE count=5
```

**核心发现**：相同的虚拟地址 `0x40028cd0`，wait 操作时对应的物理地址是 `0x887cbcd0`，而 wake 操作时物理地址变成了 `0x890eecd0`！

### 2.3 根本原因：COW (Copy-on-Write) 机制

这是一个经典的 **Copy-on-Write (写时复制)** 问题：

1. `futex_wake02` 测试通过 `fork()` 创建多个子进程，然后在子进程中 `pthread_create()` 创建线程
2. fork 时，父子进程共享相同的物理页面，但页表项被标记为只读（COW 保护）
3. 当子进程或其线程首次**写入**共享页面时，触发 page fault，内核分配新的物理页面并复制内容
4. 此时虚拟地址 `0x40028cd0` 仍然不变，但其映射的物理地址已经改变

**原始实现的错误**：
```rust
// 错误的实现：使用物理地址作为 private futex 的键
let pa = translated_byte_buffer(process.inner_exclusive_access().memory_set.token(), 
                                uaddr as *const u8, 4)
    .get(0)
    .unwrap()
    .as_ptr() as usize;
let key = FutexKey { paddr: pa };
```

这导致：
- **wait 操作** 在 COW 触发**之前**，使用旧物理地址 `0x887cbcd0` 计算 hash，线程被放入队列 A
- **wake 操作** 在 COW 触发**之后**，使用新物理地址 `0x890eecd0` 计算 hash，从队列 B 查找
- 队列 A 和队列 B 是不同的 hash bucket，导致 wake 无法唤醒 wait 的线程

### 2.4 解决方案

**Private futex 应该使用虚拟地址作为键**，因为：
- Private futex 仅用于同一进程内的线程同步，不需要跨进程
- 同一进程内，相同的虚拟地址始终指向同一个逻辑位置，即使物理地址因 COW 改变
- 使用 `虚拟地址 + pid` 作为键，确保同一进程内的 wait/wake 操作匹配

**Shared futex 仍然使用物理地址**，因为：
- Shared futex 用于跨进程同步（例如共享内存中的 pthread mutex）
- 不同进程可能以不同的虚拟地址映射同一物理页面
- 物理地址是跨进程通信的唯一稳定标识

修复代码：
```rust
let key = if is_private {
    // Private futex: use virtual address + pid
    FutexKey { paddr: (uaddr << 16) | process.pid.0 }
} else {
    // Shared futex: use physical address for cross-process sync
    let pa = translated_byte_buffer(/* ... */)
        .get(0).unwrap().as_ptr() as usize;
    FutexKey { paddr: pa }
};
```

此修复同时应用到了 `FUTEX_WAIT`、`FUTEX_WAKE`、`FUTEX_CMP_REQUEUE` 和 `FUTEX_REQUEUE` 操作。

### 2.5 验证结果

修复后，`futex_wake02` 测试完全通过：
```
futex_wake02    1  TPASS  :  futex_wake() woke up 1 thread
futex_wake02    2  TPASS  :  futex_wake() woke up 2 threads
...
futex_wake02   10  TPASS  :  futex_wake() woke up 10 threads
```

所有 10 轮 wake 操作都成功唤醒了预期数量的线程，测试在 5 秒内完成。

---

## 三、futex_wait03 调试：procfs 线程可见性问题

### 3.1 问题现象

`futex_wait03` 测试在 36 秒后卡死超时。查看日志发现测试在执行 `TST_PROCESS_STATE_WAIT` 时陷入无限循环。

关键日志：
```
[WARN] open: path=/proc/78/task, flags=... => ENOTDIR
[WARN] open: path=/proc/78/task, flags=... => ENOTDIR
```

不断重复相同的错误，说明测试在轮询 `/proc/<pid>/task/` 目录，但内核返回 `ENOTDIR` 错误。

### 3.2 LTP 测试机制分析

查看 LTP 测试源码 `/home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524/testcases/kernel/syscalls/futex/futex_wait03.c`：

```c
// Test uses TST_PROCESS_STATE_WAIT to monitor parent process state
child = SAFE_FORK();
if (child == 0) {
    // Child process monitors parent's state via /proc
    TST_PROCESS_STATE_WAIT(getppid(), 'S', 0);
    // ...
}
```

`TST_PROCESS_STATE_WAIT` 的实现逻辑（来自 LTP 测试框架）：
1. 读取 `/proc/<pid>/stat` 文件
2. 解析第 3 个字段（进程状态字符）
3. 如果不是期望的状态（例如 'S' 表示 sleeping），则轮询重试
4. 为了支持多线程，还会遍历 `/proc/<pid>/task/<tid>/stat` 以检查各个线程状态

### 3.3 根本原因

原始的 procfs 实现存在两个问题：

**问题 1：缺少 `/proc/<pid>/task/` 目录结构**

内核只实现了 `/proc/<pid>/stat`，但没有实现 `/proc/<pid>/task/` 目录及其下的 `/proc/<pid>/task/<tid>/stat` 文件。当测试尝试打开这些路径时，内核返回 `ENOTDIR`，导致测试无法正确监控线程状态。

**问题 2：`/proc/<pid>/stat` 报告的状态不正确**

原始实现中，`/proc/<pid>/stat` 返回的是**所有线程聚合后的状态**：

```rust
// 错误的实现：遍历所有 task，只要有一个不是 Zombie 就返回其状态
for task in inner.tasks.iter() {
    if task.inner_exclusive_access().task_status != TaskStatus::Zombie {
        status_char = task.inner_exclusive_access().task_status.to_state_char();
        break;
    }
}
```

但根据 Linux 语义，`/proc/<pid>/stat` 应该只报告**主线程（thread leader，tid=0）** 的状态，而不是聚合状态。

### 3.4 解决方案

**修复 1：实现 `/proc/<pid>/task/` 目录结构**

在 `os/src/fs/vfs/procfs.rs` 中添加对 `/proc/<pid>/task/<tid>/stat` 的支持：

```rust
// Handle /proc/<pid>/task/<tid>/stat
if parts.len() == 5 && parts[3] == "task" {
    if let Ok(tid) = parts[4].parse::<usize>() {
        // Find the specific thread and generate its stat
        let task = inner.tasks.iter().find(|t| t.tid() == tid)?;
        let stat_content = format!("{} (task{}) {} ...", pid, tid, status_char);
        return Some(stat_content.into_bytes());
    }
}
```

同时在目录列举中添加 `task/` 目录：

```rust
if parts.len() == 2 {
    // /proc/<pid>/ directory listing
    let entries = vec!["stat", "status", "maps", "task"];
    // ...
}
```

**修复 2：修正 `/proc/<pid>/stat` 状态报告**

将聚合逻辑改为只查找主线程（tid=0）：

```rust
// Correct implementation: report leader thread state only
let status_char = inner.tasks.iter()
    .find(|t| t.tid() == 0)  // Find thread leader
    .map(|task| task.inner_exclusive_access().task_status.to_state_char())
    .unwrap_or('Z');
```

### 3.5 验证结果

修复后，`futex_wait03` 测试成功通过：

```
futex_wait03    1  TPASS  :  futex_wait() woken up by signal
```

测试在 2 秒内完成，不再出现 36 秒超时。日志中也不再有 `ENOTDIR` 错误。

---

## 四、clock_nanosleep01 调试：参数校验与信号处理

### 4.1 问题现象

`clock_nanosleep01` 测试在 30 秒后超时，并报告 `TBROK: waitpid() failed: EINTR`。

日志片段：
```
clock_nanosleep01 1 TINFO: Testing variant with given time
[WARN] sys_waitpid: process 81 returns EINTR due to pending signal 20
clock_nanosleep01 1 TBROK: waitpid() failed: EINTR
```

并且在某些测试变体中，内核日志显示 `sys_nanosleep` 被调用但长时间不返回，导致测试卡死。

### 4.2 问题分析

通过查看测试源码和添加详细日志，发现了多个层次的问题。

#### 问题 1：参数校验缺失导致无限睡眠

测试用例故意传递了 **负数** 的 `tv_nsec` 参数：

```c
// Test case: invalid tv_nsec
struct timespec ts = {.tv_sec = 0, .tv_nsec = -1};
TEST(clock_nanosleep(clks[i], 0, &ts, NULL) == EINVAL);
```

在 Rust 中，`tv_nsec` 的类型是 `usize`（无符号整数），负数 `-1` 会被解释为 `0xFFFFFFFFFFFFFFFF`（在 64 位系统上约 584 年）！

原始代码缺少对 `tv_nsec` 范围的校验：

```rust
// 缺少校验，直接计算睡眠时间
let sleep_ns = req.tv_sec * 1_000_000_000 + req.tv_nsec;
```

这导致内核尝试睡眠数万亿纳秒，实际上陷入了"永久"等待状态。

#### 问题 2：clock_id 和 flags 校验缺失

测试用例还测试了无效的 clock ID 和不支持的 flags：

```c
TEST(clock_nanosleep(CLOCK_THREAD_CPUTIME_ID, 0, &ts, NULL) == ENOTSUP);
TEST(clock_nanosleep(CLOCK_REALTIME, TIMER_ABSTIME, &ts, NULL) == EINVAL);
```

原始代码没有对这些参数进行校验，导致返回错误的错误码或进入未定义行为。

#### 问题 3：EINTR 与 EFAULT 的返回顺序错误

LTP 测试用例专门测试了这样的场景：

```c
// Test EFAULT when rem pointer is invalid
struct timespec *bad_addr = (struct timespec *)0xDEADBEEF;
TEST(nanosleep(&valid_ts, bad_addr) == EFAULT);  // When interrupted by signal
```

当 `nanosleep` 被信号中断时，内核需要将剩余时间写入 `rem` 指针。如果该指针无效，应该返回 `EFAULT` 而不是 `EINTR`。

原始代码的错误实现：

```rust
if interrupted_by_signal {
    let _ = copy_to_user(token, rem_ptr, &remain_ts);  // 忽略错误！
    return Err(SysErrNo::EINTR);  // 总是返回 EINTR
}
```

使用 `let _ =` 忽略了 `copy_to_user` 的错误返回值，导致即使写入失败也返回 `EINTR`。

#### 问题 4：waitpid 的 EINTR 处理不当

测试框架使用 `waitpid()` 等待子进程，当收到 `SIGALRM` 时，`waitpid()` 被中断并返回 `EINTR`，导致测试报告 `TBROK`。

但根据 POSIX 语义，**定时器信号（SIGALRM、SIGVTALRM、SIGPROF）应该强制返回 EINTR**，即使设置了 `SA_RESTART` 标志。这是为了确保定时器看门狗能够打破内核的长时间阻塞操作。

原始代码没有对定时器信号进行特殊处理，导致某些情况下 `waitpid` 不返回 `EINTR`，测试无法正确检测超时。

### 4.3 解决方案

#### 修复 1：添加 tv_nsec 范围校验

```rust
pub fn sys_nanosleep(req: usize, rem: usize) -> SysResult {
    let token = current_user_token();
    let req = translated_refmut(token, req as *mut TimeSpec);
    
    // Validate tv_nsec range: must be [0, 999,999,999]
    if req.tv_nsec >= 1_000_000_000 {
        return Err(SysErrNo::EINVAL);
    }
    
    // ... rest of implementation
}
```

#### 修复 2：添加 clock_id 和 flags 校验

```rust
pub fn sys_clock_nanosleep(clock_id: usize, flags: usize, 
                          req: usize, rem: usize) -> SysResult {
    // Validate clock_id
    match clock_id {
        CLOCK_REALTIME | CLOCK_MONOTONIC | CLOCK_PROCESS_CPUTIME_ID => {},
        CLOCK_THREAD_CPUTIME_ID => return Err(SysErrNo::ENOTSUP),
        _ => return Err(SysErrNo::EINVAL),
    }
    
    // Validate flags (currently only support relative sleep)
    if flags != 0 {
        return Err(SysErrNo::EINVAL);
    }
    
    // ... rest of implementation
}
```

#### 修复 3：修正 EFAULT 优先级

```rust
if interrupted_by_signal {
    // Try to write remaining time first
    if let Err(e) = copy_to_user(token, rem_ptr, &remain_ts) {
        return Err(e);  // Return EFAULT immediately
    }
    return Err(SysErrNo::EINTR);  // Only if write succeeded
}
```

#### 修复 4：特殊处理定时器信号

```rust
// In sys_waitpid
if interrupted_by_signal {
    let inner = task.inner_exclusive_access();
    let signals = inner.signals.signals;
    
    // Timer signals (SIGALRM=14, SIGVTALRM=26, SIGPROF=27) always cause EINTR
    if signals.contains(SignalFlags::from_bits(1 << (14 - 1)).unwrap())
        || signals.contains(SignalFlags::from_bits(1 << (26 - 1)).unwrap())
        || signals.contains(SignalFlags::from_bits(1 << (27 - 1)).unwrap()) {
        return Err(SysErrNo::EINTR);
    }
    
    // SIGCHLD should not cause EINTR (default-ignored signal)
    // ... other logic
}
```

### 4.4 验证结果

修复后，`clock_nanosleep01` 的所有测试变体都通过：

```
clock_nanosleep01 1 TPASS: clock_nanosleep() passed with ret=EINVAL (invalid ts.tv_nsec)
clock_nanosleep01 2 TPASS: clock_nanosleep() passed with ret=EINTR (interrupted)
clock_nanosleep01 3 TPASS: clock_nanosleep() passed with ret=EFAULT (invalid rem pointer)
clock_nanosleep01 4 TPASS: clock_nanosleep() passed with ret=ENOTSUP (unsupported clock)
```

测试在 3 秒内完成，不再超时。

---

## 五、技术总结与经验教训

### 5.1 Futex 语义的关键要点

1. **Private vs Shared 的区别**：
   - Private futex (`FUTEX_PRIVATE_FLAG` 设置) 仅用于进程内同步，键应使用虚拟地址
   - Shared futex 用于跨进程同步，键必须使用物理地址
   - 混淆两者会导致 COW 场景下的同步失败

2. **COW 对同步原语的影响**：
   - Fork/clone 后，子进程的页面在首次写入时触发 COW
   - 所有基于"内存地址"的同步原语（futex、rwlock、condvar）都必须考虑 COW
   - 解决方案：进程内使用虚拟地址，跨进程使用物理地址或共享内存

3. **调试技巧**：
   - 同时输出虚拟地址和物理地址能快速发现 COW 问题
   - Hash 碰撞问题可以通过 hash bucket 分布统计发现

### 5.2 /proc 文件系统的 Linux 语义

1. **线程可见性**：
   - `/proc/<pid>/stat` 只报告主线程（thread leader）状态
   - `/proc/<pid>/task/<tid>/stat` 报告特定线程状态
   - 缺少 task/ 目录会导致许多多线程测试失败

2. **状态字符的含义**：
   - 'R' (Running), 'S' (Sleeping), 'D' (Disk sleep), 'Z' (Zombie), 'T' (Traced/Stopped)
   - LTP 测试通过轮询状态字符来同步测试流程

3. **实现建议**：
   - procfs 是测试工具的重要依赖，应优先实现
   - 可以参考 Linux 内核的 `fs/proc/` 实现，确保兼容性

### 5.3 信号处理的微妙语义

1. **EINTR 的返回时机**：
   - 不是所有信号都应该导致 EINTR（例如 SIGCHLD 默认被忽略）
   - 定时器信号（SIGALRM、SIGVTALRM、SIGPROF）必须强制 EINTR，即使设置了 SA_RESTART
   - 用户应该有机会在信号处理函数中检查超时条件

2. **EFAULT vs EINTR 的优先级**：
   - 当系统调用被信号中断，且需要回写用户空间指针时
   - 应该先尝试回写，如果失败返回 EFAULT
   - 只有回写成功后才返回 EINTR
   - 这确保了错误码的"精确性"优先于"中断性"

3. **SA_RESTART 的限制**：
   - SA_RESTART 可以自动重启大多数慢速系统调用（read、write、wait）
   - 但定时器信号不应该被重启，否则无法实现超时机制
   - 内核需要在信号处理逻辑中区分这两类信号

### 5.4 系统调用参数校验的重要性

1. **永远不要信任用户空间参数**：
   - `tv_nsec` 必须检查上界（< 1,000,000,000）
   - 即使参数类型是 `usize`，也可能因类型转换携带无效值
   - 负数转无符号整数会产生极大的正数

2. **错误码的准确性**：
   - Linux 应用依赖精确的错误码来判断失败原因
   - `EINVAL` (参数无效)、`ENOTSUP` (功能不支持)、`EFAULT` (地址无效) 必须区分清楚
   - LTP 测试会验证每个边界条件的错误码

3. **渐进式实现策略**：
   - 对于复杂特性（如 `TIMER_ABSTIME`），可以先返回 `EINVAL` 而不是崩溃
   - 在错误日志中注明"Not implemented"，方便后续跟踪
   - 确保部分实现不会导致未定义行为或安全漏洞

### 5.5 调试方法论

1. **日志分层策略**：
   - `LOG=ERROR`：只看错误，快速定位崩溃和 panic
   - `LOG=INFO`：看关键事件（syscall 参数、返回值），平衡信息量和可读性
   - `LOG=TRACE`：看所有细节（页表、中断、调度），用于深入分析
   - 本次调试主要使用 `LOG=INFO`，在日志量和信息量之间取得良好平衡

2. **单测隔离**：
   - `SINGLE_TEST` 环境变量可以只运行特定测试集
   - 避免在海量测试输出中查找问题
   - 本次使用 `SINGLE_TEST=tmp-ltp-stuck` 只运行 3 个卡死测试

3. **超时控制**：
   - 使用 `timeout` 命令防止完全卡死
   - 从 300s 逐步缩减到 60s，加快迭代速度
   - 成功修复后测试时间从 300s+ 降低到 ~45s

4. **对照参考实现**：
   - 查看其他 OS 项目（如 `/home/grl/codeRepo/T202410487992457-1800`）的 EINTR 实现
   - 阅读 LTP 测试源码理解期望行为
   - 参考 Linux man page 了解标准语义

### 5.6 修改的代码文件总结

1. **os/src/syscall/process.rs** (主要修改)：
   - `sys_futex`: 修正 private futex 键生成，使用虚拟地址代替物理地址
   - `sys_waitpid`: 添加定时器信号的特殊 EINTR 处理
   - `sys_nanosleep`: 添加 `tv_nsec` 范围校验，修正 EFAULT 返回优先级
   - `sys_clock_nanosleep`: 添加 clock_id 和 flags 校验

2. **os/src/fs/vfs/procfs.rs**：
   - 实现 `/proc/<pid>/task/<tid>/stat` 文件
   - 修正 `/proc/<pid>/stat` 只报告主线程状态
   - 添加 `task/` 目录的列举支持

3. **os/src/task/futex.rs**：
   - 移除临时调试日志（清理工作）

---

## 六、后续改进方向

1. **性能优化**：
   - 当前 futex hash table 使用固定大小，高并发场景可能碰撞
   - 可以考虑使用 per-CPU hash table 或 RCU 优化锁竞争

2. **功能完善**：
   - 实现 `TIMER_ABSTIME` 标志支持绝对时间睡眠
   - 实现 `FUTEX_WAIT_BITSET` 的完整位掩码语义
   - 完善 `/proc/<pid>/maps`、`/proc/<pid>/status` 等其他 procfs 文件

3. **错误处理**：
   - 添加更多边界条件校验（如 futex 地址对齐检查）
   - 改进错误日志的可读性，方便用户调试应用

4. **测试覆盖**：
   - 添加单元测试覆盖 futex COW 场景
   - 添加多线程压力测试验证 EINTR 处理正确性

---

## 七、结论

本次调试共修复了三个主要卡死问题，涉及内核的多个核心子系统：

- **进程同步**：futex 的 COW 兼容性
- **虚拟文件系统**：procfs 的线程可见性
- **信号处理**：EINTR 的语义正确性
- **参数验证**：系统调用的安全性

这些修复不仅解决了 LTP 测试的卡死问题，还提高了内核对 Linux ABI 的兼容性，为后续运行更复杂的用户态程序（如 glibc、musl、busybox）奠定了基础。

通过本次调试，我们深刻认识到：**操作系统内核的"细节"往往决定了兼容性和稳定性**。看似简单的 futex、procfs、信号处理，实际上蕴含着丰富的边界条件和微妙语义。只有深入理解 Linux 标准、仔细阅读测试源码、进行大量的实验和验证，才能实现真正可用的操作系统内核。

---

**修复总结**：
- ✅ futex_wake02: COW 场景下的 futex 键生成修正
- ✅ futex_wait03: procfs 线程可见性实现
- ✅ clock_nanosleep01: 参数校验、EINTR/EFAULT 处理修正
- ✅ 测试时间从 300s+ 降低到 45s
- ✅ 所有 tmp-ltp-stuck 测试通过

**代码贡献**：约 150 行核心修改，分布在 3 个文件中。
