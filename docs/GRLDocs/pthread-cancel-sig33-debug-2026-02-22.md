# pthread_cancel_points 卡死调试记录（SIG33 未支持）

日期：2026/2/22

## 一、现象与结论（先给罪魁祸首）

本次卡死的直接原因是 **内核仅支持 1..31 的信号号（MAX_SIG=31）**，而 `pthread_cancel_points` 用到的取消信号是 **33 号**。由于信号范围被硬编码为 31，`sys_sigaction(33)` 直接返回 `-EINVAL`，`rt_sigtimedwait` 也只遍历 1..31，导致测试进程一直在用户态等待一个永远不会被内核认可的信号，从而形成父子进程的 `waitpid` 活锁链。这个错误是低级的“信号编号范围不足”，属于接口层面的遗漏。

关键证据来自日志（all200.log）：
- `sys_sigaction signum=33 ret=-22`（即 `-EINVAL`），说明 33 号信号被内核拒绝。
- 测试进入 `pthread_cancel_points` 后，`runtest.exe` 的 `last_syscall=137 (rt_sigtimedwait)` 长期停留，父进程与 init 不断 `waitpid`。
- 没有 `IllegalInstruction/StorePageFault` 等异常日志，说明不是崩溃，而是“信号等待永远不满足”。

结论：**信号号上限过低导致 SIGCANCEL（33 号）无法注册/等待，最终触发活锁。**

## 二、现象复盘与逻辑链条

### 1. 测试卡住位置
日志显示测试顺序正常运行到：
```
========== START entry-static.exe pthread_cancel_points ==========
```
之后卡死，并在内核里大量出现 `waitpid` 的重复输出。结合线程/进程 dump：
- `initproc` 等待 `busybox`（pid=2）
- `busybox` 等待 `sh`（pid=4）
- `sh` 等待 `runtest.exe` 子进程
- `runtest.exe` 在 `rt_sigtimedwait` 处停住

这是一条“父进程等待子进程退出”的链条，但子进程永远不退出，从而形成活锁。

### 2. 关键 syscall 负返回值

```
[syscall] pid=34 name=entry-static.exe num=134 ... ret=-22
```
`num=134` 是 `rt_sigaction`，`ret=-22` 即 `-EINVAL`。同时日志明确标注：
```
kernel:pid[34] sys_sigaction signum=33
```
这条负返回值直接说明 **内核拒绝了 33 号信号**。

### 3. 代码层证据
- `MAX_SIG=31`（os/src/task/signal.rs），直接限制了支持的信号编号。
- `sys_sigaction` 中的检查逻辑：
  - `if signum <= 0 || signum > MAX_SIG ... return -EINVAL`
- `sys_rt_sigtimedwait` 仅遍历 `1..=MAX_SIG`，33 号永远不被检查。

这说明：即便用户态设置了 sigset 包含 33，内核也不会处理。

## 三、为什么 LOG 级别改变了卡住表现

LOG=ERROR 时能跑到 `pthread_cancel_points`，LOG=TRACE 时更早暴露问题，这是调度节奏改变导致的“暴露时机不同”。但从本质上讲，**无论日志级别如何，SIG33 都会被拒绝**，只是活锁发生的时间点不同。

判断依据：
- 日志级别不会改变 `sys_sigaction` 的参数检查。
- `-EINVAL` 是确定性错误。
- 卡住点一致集中在 `pthread_cancel_points` 之后。

## 四、调试过程中的关键判断点

1) **先确认是否异常崩溃**：
- 日志中无 `IllegalInstruction/StorePageFault/Panicked` 等关键字。说明不是异常终止。

2) **跟踪 waitpid 活锁链条**：
- `waitpid` 反复出现，说明父进程在等待子进程退出。
- debug dump 显示子进程始终不是 zombie。

3) **聚焦 syscall 负返回值**：
- 发现 `sys_sigaction signum=33 ret=-22`，这是决定性证据。
- 对应 syscall 语义：EINVAL 表示不支持该信号。

4) **回到源码验证范围限制**：
- MAX_SIG=31、bitflags 使用 u32，确实不支持 33。

## 五、修复思路（结构层面）

核心目标：**扩展信号范围，支持 pthread cancel 所用信号号**。推荐方案：

1. 将信号上限从 31 扩展到至少 33（建议 64）。
2. `SignalFlags` 的底层类型从 `u32` 改为 `u64`，避免 32+ 号信号被截断。
3. 所有信号集的位运算使用 `1u64 << signum`（或 `1u64 << (signum-1)`）。
4. `SignalActions` 表大小同步扩大（MAX_SIG+1）。
5. `sys_sigaction` / `sys_rt_sigtimedwait` / `sys_sigprocmask` 的边界检查使用新的 MAX_SIG。

这样可以保证 SIG33 号不会返回 `-EINVAL`，也能在 `rt_sigtimedwait` 中被正确捕获。

## 六、简化实现与耗时点说明

- **简化实现**：将 MAX_SIG 直接提升到 64 是最直接的修复，无需引入复杂的信号编号映射；但需注意 bitflags 类型变更会影响多个文件。
- **耗时点**：
  - `SignalFlags` 类型升级为 `u64` 后，所有 `from_bits`、位移、掩码判断都要同步修改，否则容易出现 silent truncation。
  - 用户态 `sigset_t` 位定义与内核 `SignalFlags` 位定义不同，需要始终坚持 `signum-1` 对应用户位，`signum` 对应内核位这一规范。

## 七、经验教训与改进建议

1. **优先检查 syscall 的负返回值**：这次 `-EINVAL` 是最关键的信号，应该第一时间检索。下次调试应先 `rg "ret=-"` 或聚焦 syscall 负返回值。
2. **信号范围/ABI 常量是高风险点**：MAX_SIG、sigset 位宽、SignalFlags 类型，任何一个错误都会导致用户态隐蔽性卡死。
3. **尽量将信号范围与用户态一致**：Linux 常见支持 64 个信号（含实时信号），至少要覆盖 33。

## 八、结论

本次卡死是**内核信号范围过窄**导致 pthread cancel 相关信号被拒绝。核心证据是 `sys_sigaction signum=33 ret=-22`，而 `MAX_SIG=31` 明确限制了信号编号。修复应当扩展信号范围并升级信号位图类型，确保 `rt_sigtimedwait` 能看到 SIG33。今后调试应首先关注 syscall 的负返回值，避免在“日志量”上浪费时间。