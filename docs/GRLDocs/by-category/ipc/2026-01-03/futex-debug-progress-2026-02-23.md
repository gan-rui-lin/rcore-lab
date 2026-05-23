# futex 调试进展记录

日期：2026/02/23

## 背景与目标
近期在 rCore 内核中实现了 futex 系统调用，并进入联调阶段。目标是在 musl 测试集中确保 futex 相关用例正常结束，且系统整体测试不会因线程/信号/等待逻辑进入活锁或死锁。本次调试重点是：

1. 验证 futex 等待/唤醒路径是否能够匹配同一 key。
2. 处理 futex 等待与信号投递之间的关系，避免阻塞任务无法被唤醒。
3. 扩展信号范围以覆盖测试用例（特别是 pthread_cancel 相关路径）。
4. 记录当前发现的问题与下一步定位方向。

## 已完成工作与关键修改

### 1) futex 基本实现与 syscall 接入
完成了 futex 队列与基础操作，包含 wait/wake/requeue 与 bitset 变体，并在 syscall dispatch 中添加 `sys_futex` 路由与 `SYSCALL_FUTEX=98`。同时补充了 `ETIMEDOUT` 以处理带超时的等待路径。

实现要点：
- 使用 `FutexKey { paddr, pid }` 作为队列 key。
- `FUTEX_PRIVATE_FLAG` 时 key 中 pid 使用当前进程 pid，否则 pid=0。
- `FUTEX_WAIT` / `FUTEX_WAIT_BITSET`：先比对用户内存值，不匹配直接 `-EAGAIN`，否则入队阻塞。
- `FUTEX_WAKE` / `FUTEX_WAKE_BITSET`：按 key 唤醒队列中的等待者。
- `FUTEX_REQUEUE`：从 old_key 迁移部分等待者到 new_key。

### 2) mprotect/clone 权限复制修复
在 clone 场景中，发现栈页权限在 `MemorySet::from_existed_user()` 时丢失，导致 LoadPageFault。已调整为按页复制 PTE flags，从而保留 `mprotect` 产生的权限变化。该问题已被确认与修复。

### 3) 信号范围扩展与 SIG33 兼容
pthread_cancel 测试中使用了 33 号信号，原先 `MAX_SIG=31` 直接返回 `-EINVAL`，导致取消点挂起。已将信号范围扩展到 64，`SignalFlags` 改为 `u64`，并修正 `checked_shl` + `from_bits_truncate` 的位移路径。

### 4) futex 与信号唤醒的交互
之前出现 SIGKILL pending 但任务仍阻塞在 futex 的情况。为避免阻塞任务无法被 signal 路径打断，在 `handle_signals` 中加入：
- 若当前任务为 `Blocked`，从 futex 队列移除该等待者并重新入队为 Ready。
- 新增 `futex_remove_waiter_any` 支持按 task 直接扫描/移除等待队列。

### 5) 加强调试日志
针对定位 hang，引入了：
- `sys_futex` 详细跟踪，包含 op/cmd/private/uaddr/pa/val/bitset/timeout 和 wake 计数。
- 进程/任务状态 dump（ready 队列、任务 last_syscall、sepc/sp/ra）。
- 时钟中断采样（后续发现采样在无当前任务时可能 panic，已规避）。

## 现象与结论

### 现象：futex wait 后无 wake
多份日志中可见如下模式：
- entry-static.exe 进入 `FUTEX_WAIT`，`private=true`，无超时，随后任务状态为 `Blocked`，`last_syscall=98`。
- 日志中没有任何 `FUTEX_WAKE/REQUEUE` 记录；`sys_waitpid` 在父进程循环 yield。

核心证据：
- `sys_futex` 跟踪输出只出现 `WAIT`，没有 `WAKE`。
- 进程树中等待的 entry-static.exe 始终阻塞，父进程一直 waitpid。

结论：
- 目前的 hang 并非崩溃，而是没有任何唤醒发生。
- 即便后续有 wake，如果跨 pid 且 `private=true`，key 也不会匹配。
- 需要确认：用户态是否确实调用 `FUTEX_WAKE`；若调用，是否因 `private` key 或 paddr 不一致导致无法匹配。

## 当前风险点

1. futex key 的匹配范围
- `private` 将 key 绑定到 pid；若线程不是同 pid/tgid，则 wake 永远无法命中。
- 如果共享内存映射不一致（不同物理页），即使 pid 相同也无法匹配。

2. 线程模型与 futex 语义不一致
- 如果 clone/线程未共享地址空间或 pid 语义不符合 pthread 预期，futex private 的行为会与用户态库不一致。

3. 信号路径与 futex 的解阻塞
- 已加入 `handle_signals` 中的移除队列逻辑，但前提是确实存在 pending 信号。
- 若用户态依赖 futex 唤醒而未发信号，此路径不会触发。

## 下一步定位建议

1) 明确 wake 来源
- 继续保持 `sys_futex` 无条件日志，确认是否有 `FUTEX_WAKE` 调用。
- 若没有，说明用户态未发起 wake 或卡在信号路径。

2) 核对线程/进程语义
- 检查 pthread 相关 clone 路径是否共享 tgid 或地址空间。
- 若实际是多 pid 进程而用户态使用 `FUTEX_PRIVATE_FLAG`，需纠正语义或调整 key 设计。

3) 核对 futex 共享内存物理页
- 关键 `uaddr` 的 `pa` 需要确认在 wait/wake 之间一致。

4) 如需进一步验证
- 可加入只在 `entry-static.exe` 上的 futex 日志过滤，减轻日志量。
- 或增加一次性的 futex key 统计（pid + paddr + waiters 数）。

## 小结
本轮调试已经从“无 futex 实现”推进到“完整 syscall 与队列逻辑”，并修复了 clone 权限、信号范围等关键问题。当前问题表现为 futex wait 无 wake，核心风险集中在线程语义与 key 匹配范围。下一步需要结合用户态行为确认 wake 是否发生，并验证 private futex 的使用是否与内核线程模型一致。
