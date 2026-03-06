# pthread_cancel EINTR 修复进展

日期：2026/2/27

## 已完成的修复

### 1. 添加 interrupted_by_signal 标志（[os/src/task/task.rs](os/src/task/task.rs:41)）

在 `TaskControlBlockInner` 中添加了 `interrupted_by_signal: bool` 字段，用于显式标记任务是被信号唤醒的。

```rust
pub struct TaskControlBlockInner {
    // ...
    pub interrupted_by_signal: bool,
    // ...
}
```

### 2. 在 handle_signals 中设置标志（[os/src/task/mod.rs](os/src/task/mod.rs:362)）

当信号唤醒被阻塞的任务时，设置 `interrupted_by_signal = true`：

```rust
if task_inner.task_status == TaskStatus::Blocked {
    futex_remove_waiter_any(&task);
    task_inner.task_status = TaskStatus::Ready;
    task_inner.interrupted_by_signal = true; // 设置中断标志
    // ...
}
```

### 3. 在 futex_wait 中检查标志并返回 -EINTR（[os/src/task/futex.rs](os/src/task/futex.rs:78-98)）

```rust
let task = current_task().unwrap();
let mut task_inner = task.inner_exclusive_access();
let interrupted = task_inner.interrupted_by_signal;
if interrupted {
    task_inner.interrupted_by_signal = false; // 清除标志
}
// ...
if interrupted {
    return -4; // EINTR
} else {
    return 0;
}
```

### 4. **修复sys_futex忽略返回值的关键bug**（[os/src/syscall/process.rs](os/src/syscall/process.rs:251,324)）

**这是最关键的修复！** sys_futex之前忽略了futex_wait的返回值，总是返回0：

```rust
// 修复前
futex_wait(key.clone());  // 返回值被忽略
// ...
0  // 总是返回0

// 修复后
let ret = futex_wait(key.clone());
// ...
ret  // 返回实际值（可能是-EINTR）
```

对 `futex_wait_bitset` 也做了同样的修复。

## 测试结果

### pthread_cancel_points
- ✅ **不再卡死**：测试在1-2秒内完成
- ❌ **仍然失败**：线程退出码仍是0而不是PTHREAD_CANCELED
- 错误信息：`res != PTHREAD_CANCELED failed (shm_open, canceled thread exit status)`

### pthread_cancel
- ❌ **仍然超时卡住**

## 问题分析

尽管实现了所有EINTR机制，但日志显示：

```
[exit] pid=34 tid=1 name=entry-static.exe code=0
[exit] pid=34 tid=2 name=entry-static.exe code=0
```

线程仍然以exit code=0退出，而不是PTHREAD_CANCELED（-1）。

### 可能的原因

1. **线程不在futex_wait中阻塞**：被取消的线程可能在其他取消点（如read/write）或者根本不在取消点

2. **interrupted_by_signal标志未生效**：
   - 日志显示 `wait_resume interrupted=false`
   - 可能是timing问题或标志被清除了

3. **musl的pthread_cancel实现问题**：
   - musl可能期望某些我们未实现的取消点函数返回-EINTR
   - 或者取消状态检查依赖于某些我们未实现的机制

4. **信号投递时序问题**：
   - 信号可能在线程未处于Blocked状态时到达
   - handle_signals检查 `task_status == Blocked` 可能漏掉一些场景

## 后续调试方向

1. **追踪被取消线程的具体syscall**：
   - 添加日志记录被取消线程在收到SIGCANCEL时正在执行的syscall
   - 确认它们是否真的在futex_wait或其他取消点中

2. **检查所有取消点函数**：
   - read, write, accept, connect 等
   - 确保它们都能被信号中断并返回-EINTR

3. **理解musl的pthread_cancel handler**：
   - 研究musl源码中SIGCANCEL handler的实现
   - 确认它期望的行为和我们实现的行为是否一致

4. **检查信号投递时机**：
   - 线程可能在Running状态而不是Blocked状态
   - 需要确认handle_signals的检查逻辑是否覆盖所有场景

## 关键代码位置

- [os/src/task/task.rs:41](os/src/task/task.rs:41) - `interrupted_by_signal` 字段定义
- [os/src/task/mod.rs:362](os/src/task/mod.rs:362) - 设置中断标志
- [os/src/task/futex.rs:78-98](os/src/task/futex.rs:78-98) - 检查中断标志并返回-EINTR
- [os/src/syscall/process.rs:251,324](os/src/syscall/process.rs:251,324) - 修复sys_futex返回值

## 相关文档

- [pthread_cancel_sigreturn_loop_fix_2026-02-26.md](pthread_cancel_sigreturn_loop_fix_2026-02-26.md) - 之前的sigreturn循环修复（结论不正确）
- [pthread_cancel_points_shm_debug_2026-02-23.md](../GRLDocs/pthread_cancel_points_shm_debug_2026-02-23.md) - 最早的调试记录
