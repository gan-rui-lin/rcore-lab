# futex/SIGCANCEL 进展记录（2026/2/23）

## 背景
近期在 musl 测试集中，`pthread_cancel_points` 一直卡住或异常退出。之前通过扩展信号范围、实现 `tkill`、补齐 `SA_SIGINFO` 基础栈帧等方式，解决了早期的 `LoadPageFault` 与 SIGCANCEL 投递线程错误的问题，但仍存在死循环或异常终止的情况。

## 今日观察与结论
1. **futex 等待/唤醒路径已打通**
   - 日志显示在 `pthread_cancel_points` 中出现：
     - `tid=1` 的 futex 等待
     - `tid=0` 的 futex 唤醒，且 `woke=1`
   - 说明之前的“永远不唤醒”问题已经修复，不再是死锁。

2. **新的错误是 InstructionPageFault (sepc=0x0)**
   - `futex` 被唤醒后，紧接着发生 `InstructionPageFault`，且 `sepc=0x0`、`stval=0x0`。
   - 这意味着线程返回到用户态时试图从地址 0 执行，属于“TrapContext 的 PC 被置为 0”的问题。

3. **潜在原因推断**
   - SIGCANCEL 仍有可能在该线程上投递，但 handler 可能为 0 或被错误覆盖。
   - 如果信号处理路径把 `sepc` 设置为 `action.handler`，而 handler 取值为 0，就会直接导致 `sepc=0`。
   - 目前日志中缺少 SIGCANCEL handler 的安装与投递细节，无法确认是 `sys_sigaction` 未成功写入，还是 `handle_signals` 读取到了错误的 `SignalAction`。

## 已落地的修复
- **clear_child_tid 唤醒**
  - 在线程退出时补齐 `clear_child_tid` 写 0 + `futex_wake(…, 1)`，并输出详细日志。
  - 这解决了等待线程永远睡眠的问题。

## 下一步计划
1. **补充 SIGCANCEL handler 相关日志**
   - 在 `sys_sigaction(signum=33)` 打印 handler/flags/restorer/mask，确认是否成功写入。
   - 在 `handle_signals` 投递 SIGCANCEL 时打印 handler/flags 以及当前 `sepc/sp`。
   - 目标是判断：handler 为 0 还是被覆盖、signum 是否正确。

2. **根据日志决定修复方向**
   - 如果 handler 为 0：检查 musl 对 `sigaction` 结构布局与内核结构是否一致。
   - 如果 handler 非 0 但 `sepc` 仍变 0：追踪 TrapContext 被覆写的路径。

## 当前状态总结
- 死锁已解除，转变为 **SIGCANCEL 路径导致的 0 地址执行异常**。
- 需要进一步确认 `sigaction` 的写入是否正确，以及 `handle_signals` 是否拿到正确 handler。
