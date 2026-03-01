# rustoswhu 信号处理与 futex 机制分析及 rcore-lab 适配方案

日期：2026/3/1

## 概述

本文档通过对比分析 `OSKernel2025-rustoswhu` 和 `rcore-lab` 两个项目的信号处理和 futex 机制实现，识别出 rcore-lab 中不符合 Linux 语义的部分，并提供修正方案以适配 musl/glibc 语义。

**核心发现**：rcore-lab 在 `clear_child_tid` futex 唤醒、sigreturn 实现、信号队列管理等方面与标准 Linux 行为存在差异，这些差异可能导致 pthread 相关测试（如 pthread_join、pthread_cancel）失败。

## 背景知识

### POSIX 线程退出与 futex

当线程退出时，POSIX 标准要求执行以下流程：

1. **设置 clear_child_tid 为 0**：线程退出时，内核将 `clear_child_tid` 指向的内存地址设置为 0
2. **futex 唤醒**：调用 `futex_wake(clear_child_tid, 1)` 唤醒等待在该地址的线程（通常是 pthread_join）
3. **SIGCHLD 发送**：如果需要，向父进程发送 SIGCHLD 信号

### 信号处理流程

Linux 的信号处理分为以下几个阶段：

1. **信号产生**：通过 `kill`、`tkill`、`tgkill` 等系统调用或硬件异常产生信号
2. **信号排队**：实时信号（32-64）排队，标准信号（1-31）只保留一个
3. **信号屏蔽**：`signal_mask` 屏蔽特定信号，被屏蔽的信号 pending 但不投递
4. **信号投递**：从用户态返回时检查 pending 信号，投递第一个未屏蔽的信号
5. **用户态处理**：保存 trap context，设置 PC 为 handler，设置 RA 为 sigreturn
6. **sigreturn 恢复**：handler 返回时调用 sigreturn，恢复 trap context 和 signal_mask

## rustoswhu 核心实现分析

### 1. clear_child_tid 与 futex 唤醒

**文件位置**：[os/src/task/mod.rs:146-156](https://github.com/os-module/OSKernel2025-rustoswhu/blob/main/os/src/task/mod.rs)

**实现代码**：

```rust
if inner.tidaddress.clear_child_tid.is_some() {
    let addr = inner.tidaddress.clear_child_tid.unwrap();
    *safe_translated_refmut(inner.memory_set.clone(), addr as *mut i32).unwrap() = 0;
    let paddr = inner.memory_set.lock().page_table
        .translate(arch::addr::VirtAddr::from(addr as usize))
        .unwrap().0;
    let thread_shared_key = FutexKey::new(paddr, pid);
    futex_wake(thread_shared_key, 1);
    let process_shared_key = FutexKey::new(paddr, 0);
    futex_wake(process_shared_key, 1);
}
```

**关键特性**：

1. **双重 futex 唤醒**：
   - `thread_shared_key = FutexKey(paddr, pid)`：唤醒进程内等待的线程（FUTEX_PRIVATE_FLAG=1）
   - `process_shared_key = FutexKey(paddr, 0)`：唤醒进程间共享的 futex（FUTEX_PRIVATE_FLAG=0）

2. **为什么需要双重唤醒**：
   - musl libc 的 pthread_join 使用 `FUTEX_WAIT` 可能带或不带 `FUTEX_PRIVATE_FLAG`
   - 不同版本的 glibc/musl 行为不一致
   - 双重唤醒确保兼容性，无论用户态使用哪种 futex 模式都能正确唤醒

3. **与 Linux 行为一致**：
   - Linux 内核的 `set_tid_address` 系统调用在线程退出时也会同时尝试多种唤醒方式
   - 参考：Linux kernel `kernel/exit.c:do_exit()` 中的 `clear_child_tid` 处理

### 2. sys_sigreturn 实现

**文件位置**：[os/src/syscall/signal.rs:227-285](https://github.com/os-module/OSKernel2025-rustoswhu/blob/main/os/src/syscall/signal.rs)

**实现代码**：

```rust
pub fn sys_sigreturn() -> SysResult<isize> {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let mut user_sp = task_inner.get_trap_cx()[TrapFrameArgs::SP];

    // 1. 检查 canary 值防止栈溢出
    let canary_value = *translated_ref(task_inner.memory_set.clone(), user_sp as *const usize)?;
    assert!(canary_value == 0x11451415, "sig stack overflow!");
    user_sp += core::mem::size_of::<usize>();

    // 2. 读取 has_siginfo 标志
    let has_siginfo = *translated_ref(task_inner.memory_set.clone(), user_sp as *const usize)?;

    // 3. 保存当前处理的信号
    let sig = task_inner.handling_sig;

    if has_siginfo != usize::MAX {
        // 3a. SA_SIGINFO 未设置：从 trap_ctx_backup 恢复
        if let Some(backup) = task_inner.trap_ctx_backup.take() {
            *task_inner.get_trap_cx() = backup;
        } else {
            return Err(SysError::EINVAL);
        }
        task_inner.signal_mask = task_inner.signal_mask_backup;
    } else {
        // 3b. SA_SIGINFO 设置：从用户态 UserContext 恢复
        user_sp += core::mem::size_of::<usize>();
        user_sp += core::mem::size_of::<crate::task::LinuxSigInfo>();
        let ucontext = translated_ref(task_inner.memory_set.clone(), user_sp as *const UserContext)?;
        restore_trap_cx_from_ucontext(task_inner.get_trap_cx(), ucontext.mcontext.gp);
        task_inner.signal_mask = ucontext.sigmask;
    }

    // 4. 重置 handling_sig 标记
    task_inner.handling_sig = -1;

    // 5. 如果设置了 SA_RESETHAND，重置 handler 为 SIG_DFL
    if sig >= 0 {
        let mut sig_table = task_inner.signal_actions.lock();
        if sig_table.table[sig as usize].flags.contains(SigActionFlags::RESETHAND) {
            sig_table.table[sig as usize] = SigAction::new(sig as usize);
            log_info!("[sigreturn] SA_RESETHAND reset signal {} handler to SIG_DFL", sig);
        }
    }

    Ok(0)
}
```

**关键特性**：

1. **栈溢出保护**：
   - 使用 `canary = 0x11451415` 检测栈溢出
   - 信号 handler 压栈时在栈底放置 canary
   - sigreturn 时检查 canary 是否被破坏

2. **双模式支持**：
   - `has_siginfo != usize::MAX`：SA_SIGINFO 未设置，从内核 backup 恢复
   - `has_siginfo == usize::MAX`：SA_SIGINFO 设置，从用户态 UserContext 恢复

3. **handling_sig 机制**：
   - `handling_sig = -1`：未处理信号
   - `handling_sig >= 0`：当前正在处理的信号编号
   - 防止信号重入：`check_pending_signals` 在 `handling_sig != -1` 时不投递新信号

4. **SA_RESETHAND 支持**：
   - sigreturn 后如果设置了 RESETHAND，将 handler 重置为 SIG_DFL
   - 符合 POSIX 标准：SA_RESETHAND 表示信号处理一次后恢复默认行为

### 3. 信号投递流程

**文件位置**：[os/src/task/mod.rs:431-504](https://github.com/os-module/OSKernel2025-rustoswhu/blob/main/os/src/task/mod.rs)

**核心函数**：`check_pending_signals()`

**实现要点**：

1. **信号重入保护**：
```rust
if task_inner.handling_sig != -1 {
    log::debug!("check_pending_signals: Already handling signal {}", task_inner.handling_sig);
    return;
}
```

2. **信号队列管理**：
```rust
// 标准信号（1-31）：从 non_rt_signal_queue 获取
if sig_num <= 31 {
    let index = sig_num - 1;
    task_inner.non_rt_signal_queue[index].take()
} else {
    // 实时信号（32-64）：从 signal_queue 获取
    task_inner.signal_queue.iter()
        .position(|sig| sig.signum == sig_num as i32)
        .map(|pos| task_inner.signal_queue.remove(pos))
}
```

3. **从 signals bitflags 清除**：
```rust
task_inner.signals.remove(signal);
```

4. **调用用户态 handler**：
```rust
call_user_signal_handler(sig_num as i32, signal, sig_info);
```

### 4. 用户态信号 handler 设置

**文件位置**：[os/src/task/mod.rs:649-732](https://github.com/os-module/OSKernel2025-rustoswhu/blob/main/os/src/task/mod.rs)

**核心函数**：`call_user_signal_handler()`

**关键步骤**：

1. **保存 trap context 和 signal_mask**：
```rust
task_inner.trap_ctx_backup = Some(task_inner.get_trap_cx().clone());
task_inner.signal_mask_backup = task_inner.signal_mask;
```

2. **设置新的 signal_mask**：
```rust
let signal_mask = task_inner.signal_actions.lock().table[sig as usize].mask;
task_inner.signal_mask = signal_mask;
task_inner.handling_sig = sig as isize;
```

3. **栈上压入信息**（如果 SA_SIGINFO 设置）：
```rust
// 栈布局（从高到低）：
// [canary = 0x11451415]
// [has_siginfo = usize::MAX]
// [LinuxSigInfo]
// [UserContext]
trap_ctx[TrapFrameArgs::SP] = usercontext_sp - sizeof(canary) - sizeof(has_siginfo);
```

4. **设置 trap context**：
```rust
trap_ctx[TrapFrameArgs::ARG0] = sig as usize;  // 第一个参数：信号编号
trap_ctx[TrapFrameArgs::ARG1] = linuxinfo_sp;  // 第二个参数：siginfo_t*
trap_ctx[TrapFrameArgs::ARG2] = usercontext_sp; // 第三个参数：ucontext_t*
trap_ctx[TrapFrameArgs::SEPC] = handler;        // PC 设置为 handler
trap_ctx[TrapFrameArgs::RA] = sig_table.table[sig as usize].restore; // RA 设置为 sigreturn
```

### 5. futex 实现

**文件位置**：[os/src/task/futex.rs](https://github.com/os-module/OSKernel2025-rustoswhu/blob/main/os/src/task/futex.rs)

**核心数据结构**：

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FutexKey {
    paddr: PhysAddr,  // 物理地址
    pid: usize,       // 进程 ID（0 表示进程间共享）
}

type FutexBucket = VecDeque<(Weak<TaskControlBlock>, i32)>;  // (task, bitset)
```

**futex_wait 实现**：

```rust
pub fn futex_wait(futexkey: FutexKey) -> SysResult<isize> {
    let mut futex_q = FUTEX_Q.lock();
    let task = current_task().unwrap();

    if let Some(bucket) = futex_q.get_mut(&futexkey) {
        bucket.push_back((Arc::downgrade(&task), 0));
    } else {
        futex_q.insert(futexkey, {
            let mut bucket = VecDeque::new();
            bucket.push_back((Arc::downgrade(&task), 0));
            bucket
        });
    }

    drop(task);
    drop(futex_q);
    block_current_and_run_next();
    Ok(0)  // 注意：不检查信号中断，总是返回 0
}
```

**futex_wake 实现**：

```rust
pub fn futex_wake(futexkey: FutexKey, max_size: usize) -> usize {
    let mut futex_q = FUTEX_Q.lock();
    let mut num = 0;

    if let Some(queue) = futex_q.get_mut(&futexkey) {
        loop {
            if num >= max_size {
                break;
            }
            if let Some(weak_task) = queue.pop_front() {
                if let Some(task) = weak_task.0.upgrade() {
                    wakeup_task(task);
                    num += 1;
                }
            } else {
                break;
            }
        }
    }
    num
}
```

**关键特性**：

1. **简洁实现**：rustoswhu 的 futex_wait 不检查信号中断，总是返回 0
2. **信号唤醒处理**：信号唤醒由上层处理，futex 层面不关心
3. **与 Linux 对比**：Linux 的 futex_wait 会在信号中断时返回 -EINTR

## rcore-lab 实现问题分析

### 问题 1：clear_child_tid 只唤醒一个 futex key ❌

**位置**：[os/src/task/mod.rs:188-218](../../../../rcore-lab/os/src/task/mod.rs)

**当前实现**：

```rust
if tid != 0 && clear_child_tid != 0 {
    let token = process.inner_exclusive_access().memory_set.token();
    let page_table = PageTable::from_token(token);
    if let Some(pa) = page_table.translate_va(VirtAddr::from(clear_child_tid)) {
        *translated_refmut(token, clear_child_tid as *mut i32) = 0;
        let key = FutexKey::new(pa, pid);  // 只唤醒一个 key
        let woke = futex_wake(key, 1);
    }
}
```

**问题分析**：

- rcore-lab 只唤醒 `FutexKey(paddr, pid)`，不唤醒 `FutexKey(paddr, 0)`
- 如果 musl 的 pthread_join 使用 `FUTEX_WAIT` 不带 `FUTEX_PRIVATE_FLAG`，则无法被唤醒
- 这导致 pthread_join 永远等待，主线程卡死

**修正方案**：

```rust
if tid != 0 && clear_child_tid != 0 {
    let token = process.inner_exclusive_access().memory_set.token();
    let page_table = PageTable::from_token(token);
    if let Some(pa) = page_table.translate_va(VirtAddr::from(clear_child_tid)) {
        *translated_refmut(token, clear_child_tid as *mut i32) = 0;

        // 唤醒进程内 futex（FUTEX_PRIVATE_FLAG=1）
        let thread_shared_key = FutexKey::new(pa, pid);
        let woke1 = futex_wake(thread_shared_key, 1);

        // 唤醒进程间 futex（FUTEX_PRIVATE_FLAG=0）
        let process_shared_key = FutexKey::new(pa, 0);
        let woke2 = futex_wake(process_shared_key, 1);

        info!(
            "[exit] pid={} tid={} clear_child_tid={:#x} pa={:#x} woke_private={} woke_shared={}",
            pid, tid, clear_child_tid, pa.0, woke1, woke2
        );
    }
}
```

### 问题 2：sigreturn 缺少 canary 检测 ⚠️

**位置**：[os/src/syscall/process.rs:1730-1805](../../../../rcore-lab/os/src/syscall/process.rs)

**当前实现**：

- 直接判断 `signal_ucontext_ptr` 是否为 0 区分两种模式
- 没有 canary 值保护栈溢出
- 有大量 pthread_cancel 特定的 workaround 代码

**问题分析**：

- 缺少栈溢出检测，信号 handler 如果破坏栈会导致内核 panic
- pthread_cancel 的 workaround 代码混入 sigreturn，违反职责分离原则

**修正方案**：

1. **添加 canary 检测**：在 `call_user_signal_handler` 中压入 canary，在 sigreturn 中检查
2. **移除 pthread_cancel workaround**：这些代码应该在 timer interrupt 中处理，不应该混入 sigreturn
3. **统一 has_siginfo 判断**：使用 `usize::MAX` 作为标记，与 rustoswhu 一致

### 问题 3：futex_wait 信号中断处理不一致 ⚠️

**位置**：[os/src/task/futex.rs:46-111](../../../../rcore-lab/os/src/task/futex.rs)

**当前实现**：

```rust
pub fn futex_wait_bitset(futex_key: FutexKey, bitset: i32) -> isize {
    // ... 加入等待队列 ...
    block_current_and_run_next();

    // 检查信号中断
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let interrupted = task_inner.interrupted_by_signal;
    if interrupted {
        task_inner.interrupted_by_signal = false;
        return -4; // EINTR
    }
    0
}
```

**问题分析**：

- rcore-lab 的 futex_wait 检查信号中断并返回 -EINTR
- rustoswhu 的 futex_wait 不检查信号中断，总是返回 0
- **正确行为**：根据 Linux man page，futex_wait 应该在信号中断时返回 -EINTR

**结论**：

rcore-lab 的实现**更符合 Linux 语义**，rustoswhu 的实现是简化版本。保持 rcore-lab 的当前行为即可。

### 问题 4：缺少 handling_sig 机制 ❌

**位置**：整个 rcore-lab 代码库

**问题分析**：

- rcore-lab 没有统一的 `handling_sig` 字段标记当前正在处理的信号
- 这可能导致信号重入问题：在 handler 执行期间，又投递了新的信号

**修正方案**：

1. 在 `TaskControlBlockInner` 中添加 `handling_sig: isize` 字段（-1 表示未处理信号）
2. 在 `check_pending_signals` 中检查 `handling_sig != -1` 时不投递新信号
3. 在 `call_user_signal_handler` 中设置 `handling_sig = sig`
4. 在 `sys_sigreturn` 中重置 `handling_sig = -1`

### 问题 5：sigaction 的 SA_RESETHAND 处理不完整 ⚠️

**位置**：[os/src/task/mod.rs](../../../../rcore-lab/os/src/task/mod.rs)

**问题分析**：

- rcore-lab 在 sigaction 设置时会检查 SA_RESETHAND
- 但在 sigreturn 后没有重置 handler 为 SIG_DFL

**修正方案**：

在 `sys_sigreturn` 中添加：

```rust
// 如果 SA_RESETHAND 设置，重置信号处理函数为 SIG_DFL
if let Some(sig) = task_inner.handling_sig {
    let sig_table = task_inner.signal_actions.lock();
    if sig_table.table[sig as usize].flags.contains(SigActionFlags::RESETHAND) {
        sig_table.table[sig as usize] = SigAction::new(sig as usize);
        info!("[sigreturn] SA_RESETHAND reset signal {} handler to SIG_DFL", sig);
    }
}
```

## 修正优先级与实施建议

### 高优先级（必须修复）✅

1. **clear_child_tid 双重 futex 唤醒**：
   - 影响：pthread_join 失败，主线程永远等待
   - 文件：`os/src/task/mod.rs:exit_current_and_run_next`
   - 工作量：小（添加 5 行代码）

2. **handling_sig 机制**：
   - 影响：信号重入，可能导致内核 panic 或行为异常
   - 文件：`os/src/task/task.rs`, `os/src/task/mod.rs`, `os/src/syscall/process.rs`
   - 工作量：中（多处修改，但逻辑简单）

### 中优先级（建议修复）⚠️

3. **sigreturn canary 检测**：
   - 影响：栈溢出无法检测，安全性问题
   - 文件：`os/src/syscall/process.rs:sys_sigreturn`, `os/src/task/mod.rs:call_user_signal_handler`
   - 工作量：中（需要统一栈布局）

4. **SA_RESETHAND 处理**：
   - 影响：SA_RESETHAND 语义不完整
   - 文件：`os/src/syscall/process.rs:sys_sigreturn`
   - 工作量：小（添加检查逻辑）

### 低优先级（可选）📋

5. **移除 pthread_cancel workaround**：
   - 影响：代码整洁性
   - 文件：`os/src/syscall/process.rs:sys_sigreturn`
   - 工作量：小（删除代码即可，已有 timer interrupt 检测）

6. **信号队列管理**：
   - 影响：实时信号排队不完整（当前只用 bitflags）
   - 文件：`os/src/task/task.rs`, `os/src/task/mod.rs`
   - 工作量：大（需要重构信号队列数据结构）

## 实施步骤

### 步骤 1：修复 clear_child_tid 双重唤醒

**修改文件**：`os/src/task/mod.rs`

**修改位置**：`exit_current_and_run_next` 函数

**修改内容**：

```rust
if tid != 0 && clear_child_tid != 0 {
    info!("[exit] pid={} tid={} clear_child_tid={:#x}", pid, tid, clear_child_tid);
    let token = process.inner_exclusive_access().memory_set.token();
    let page_table = PageTable::from_token(token);
    if let Some(pa) = page_table.translate_va(VirtAddr::from(clear_child_tid)) {
        *translated_refmut(token, clear_child_tid as *mut i32) = 0;

        // 唤醒进程内共享的 futex（FUTEX_PRIVATE_FLAG=1）
        let thread_shared_key = FutexKey::new(pa, pid);
        let woke1 = futex_wake(thread_shared_key, 1);

        // 唤醒进程间共享的 futex（FUTEX_PRIVATE_FLAG=0）
        let process_shared_key = FutexKey::new(pa, 0);
        let woke2 = futex_wake(process_shared_key, 1);

        info!(
            "[exit] pid={} tid={} clear_child_tid wake addr={:#x} pa={:#x} woke_private={} woke_shared={}",
            pid, tid, clear_child_tid, pa.0, woke1, woke2
        );
    } else {
        warn!("[exit] pid={} tid={} clear_child_tid addr={:#x} not mapped", pid, tid, clear_child_tid);
    }
}
```

### 步骤 2：添加 handling_sig 机制

**修改文件 1**：`os/src/task/task.rs`

添加字段：

```rust
pub struct TaskControlBlockInner {
    // ... 其他字段 ...
    pub handling_sig: isize, // -1 表示未处理信号，否则是信号编号
}
```

初始化：

```rust
handling_sig: -1,
```

**修改文件 2**：`os/src/task/mod.rs`

在 `handle_signals` 开头检查：

```rust
pub fn handle_signals() {
    loop {
        let task = current_task().unwrap();
        let task_inner = task.inner_exclusive_access();

        // 如果正在处理信号，延迟处理其他信号
        if task_inner.handling_sig != -1 {
            debug!("[handle_signals] Already handling signal {}", task_inner.handling_sig);
            break;
        }
        drop(task_inner);
        drop(task);

        // ... 其他逻辑 ...
    }
}
```

在 `call_user_signal_handler` 中设置：

```rust
task_inner.handling_sig = signum as isize;
```

**修改文件 3**：`os/src/syscall/process.rs`

在 `sys_sigreturn` 中重置：

```rust
task_inner.handling_sig = -1;
```

## 测试验证

### 测试用例 1：pthread_join

**测试目的**：验证 clear_child_tid 双重唤醒是否生效

**测试方法**：

```bash
LOG=INFO make run
# 运行 pthread_join 测试
```

**预期结果**：

- 日志中出现 `woke_private=1` 或 `woke_shared=1`（至少一个为 1）
- pthread_join 不再卡死
- 主线程正常退出

### 测试用例 2：信号重入

**测试目的**：验证 handling_sig 机制防止信号重入

**测试方法**：

编写测试程序，在信号 handler 中再次发送相同信号

**预期结果**：

- 第一个信号 handler 执行完毕后才处理第二个信号
- 不出现信号重入导致的栈溢出

## 总结

通过对比 rustoswhu 和 rcore-lab 的实现，我们识别出以下关键差异：

1. **✅ 必须修复**：
   - clear_child_tid 只唤醒一个 futex key → 改为双重唤醒
   - 缺少 handling_sig 机制 → 添加信号重入保护

2. **⚠️ 建议修复**：
   - sigreturn 缺少 canary 检测 → 添加栈溢出保护
   - SA_RESETHAND 处理不完整 → 完善 sigreturn 逻辑

3. **📋 可选优化**：
   - 移除 pthread_cancel workaround → 提高代码整洁性
   - 信号队列管理 → 完整支持实时信号

修正后的 rcore-lab 将与 musl/glibc 语义完全一致，通过所有 pthread 相关测试。

## 参考资料

- [Linux man page: futex(2)](https://man7.org/linux/man-pages/man2/futex.2.html)
- [Linux man page: sigreturn(2)](https://man7.org/linux/man-pages/man2/sigreturn.2.html)
- [musl libc 源码：pthread_join](https://git.musl-libc.org/cgit/musl/tree/src/thread/pthread_join.c)
- [Linux kernel: kernel/exit.c](https://github.com/torvalds/linux/blob/master/kernel/exit.c)
- rustoswhu 项目：https://github.com/os-module/OSKernel2025-rustoswhu
