# SIGCHLD 与 sigtimedwait 调试记录（2026/02/22）

## 结论先行：罪魁祸首
本次失败的直接原因是 **rt_sigprocmask/sigtimedwait 对用户信号集的位定义处理错误**，以及 **sys_sigprocmask 只使用了第一个参数，把 how 当成 mask**。这导致测试程序虽然调用了 `sigprocmask(SIG_BLOCK, {SIGCHLD}, ...)`，但内核实际并没有把 SIGCHLD 加入屏蔽或等待集合，随后 `sigtimedwait` 直接返回 `-EAGAIN`，父进程误以为超时而去 `kill`，此时子进程已退出，最终报出 `No such process`。

关键日志（来自 all127.log）能够清晰证明这一点：
- 父进程先执行 `sigtimedwait`：`ret=-11`（EAGAIN），说明没有收到 SIGCHLD。
- 随后立刻 `kill(pid, SIGKILL)`：`ret=-3`（ESRCH），说明子进程已经退出。
- 之后 `waitpid` 仍能回收该子进程，进一步证明子进程已结束但 SIGCHLD 没被正确感知。

这一行为与用户态 `runtest.c` 的逻辑完全吻合：如果 `sigtimedwait` 超时，就会发送 SIGKILL 并报 `kill failed`。因此根因并非进程退出异常，而是 **信号屏蔽与等待集合解析错误**。

## 现象与复现
现象出现在 libc-test 的 runtest 执行序列中，例如：
```
src/common/runtest.c:86: argv kill failed: No such process
```
同类错误会在多个小用例（basename、clock_gettime、dirname 等）重复出现。

对应的 TRACE 片段（简化）：
- 子进程快速正常退出：`sys_exit_group (exit_code=0)`
- 父进程 `sigtimedwait` 返回 `-11`
- 父进程 `kill` 返回 `-3`
- 随后 `waitpid` 成功回收子进程

因此可以确定不是子进程异常崩溃，而是 **父进程没有等到 SIGCHLD**。

## 根因分析（结合源码与日志）
### 1) 用户态信号集的位定义
在 libc-test 的 `runtest.c` 中，信号集合通过 `sigaddset(&set, SIGCHLD)` 构造。在 Linux ABI 中，`sigset_t` 使用 **signum-1** 作为位索引。例如：
- SIGCHLD = 17，对应位应为 `1 << 16`

### 2) 内核实现的错误点
原内核实现存在两个关键问题：
1. `sys_sigprocmask` 只读取 `args[0]`，将 `how` 误当成 mask；
2. `sys_rt_sigtimedwait` 在判断 `sigset` 时使用了 `1 << signum`，而不是 `1 << (signum-1)`。

这直接导致：
- 用户态传入的 SIGCHLD 位（第 16 位）在内核被当成第 17 位判断，判定失败；
- 由于 `sys_sigprocmask` 没正确设置屏蔽与旧集，父进程从未建立正确等待条件；
- `sigtimedwait` 进入循环后找不到匹配信号，最终超时返回 EAGAIN。

### 3) 为什么 kill 会失败
在日志中，子进程执行完目标测试后正常退出，父进程却因为误判超时调用 `kill(pid, SIGKILL)`。此时内核中该 PID 已退出，`pid2process` 失败返回 ESRCH，最终用户态显示 `kill failed: No such process`。

这说明 **kill 失败是症状而非根因**。

## 修复方案与实现
修复包含两部分：
1) 完整实现 `rt_sigprocmask`，正确解析 `how`、`set`、`oldset`，并进行 user mask 与内核 SignalFlags 的转换；
2) 修正 `sigtimedwait` 的信号位判断，从 `1 << signum` 改为 `1 << (signum-1)`。

具体改动要点：
- `sys_sigprocmask(how, set, oldset, sigsetsize)` 按 Linux 语义实现；
- `sigsetsize` 必须匹配 `sizeof(usize)`，否则返回 EINVAL；
- 用户态 `sigset_t` 使用 signum-1 的位定义，内核 SignalFlags 仍使用 signum 位，二者需显式转换；
- `sys_rt_sigtimedwait` 检查集合时使用 `(1 << (signum-1))`。

## 验证结果
修复后重新运行：
- `sigtimedwait` 不再返回 EAGAIN；
- `kill failed: No such process` 消失；
- 用例进入正常 waitpid 回收路径。

这说明 SIGCHLD 已被正确投递与感知，调试闭环完成。

## 后续建议
1. 增加一个内核自测：fork 子进程后 SIGCHLD 是否能被 `sigtimedwait` 捕获，避免回归。
2. 在信号相关 syscall 中统一封装 user sigset <-> SignalFlags 的转换函数，避免再次出现位偏移错误。
3. 如果需要更严格的行为，可实现 `sigaction(SIGCHLD, SIG_IGN)` 下的特殊规则，但当前测试场景无需。

---

本次调试记录至此完毕。