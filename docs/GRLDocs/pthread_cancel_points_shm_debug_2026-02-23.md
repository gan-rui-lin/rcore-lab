# pthread_cancel_points 调试记录（SIGCANCEL 的 tkill 循环）

日期：2026/2/23

## 结论先行（罪魁祸首）

本次 `pthread_cancel_points` 卡住的直接罪魁祸首是 **SIGCANCEL 在用户态被反复自我投递（tkill）而未能终止线程，导致信号处理进入循环**。该循环的触发点是：

- 线程在 SIGCANCEL 处理函数中调用 `tkill(tid, SIGCANCEL)`，用于触发取消语义；
- 内核在 `sigreturn` 时恢复了相同的 `pc` 和不正确的信号屏蔽语义，导致 SIGCANCEL 立刻再次被投递；
- 结果形成 “deliver sig=33 -> handler -> tkill -> sigreturn -> deliver sig=33” 的循环。

这一点从新增日志中清晰可见：

- 连续多次出现：
  - `[signal] sig=33 pid=34 tid=1 ... sepc=0x3f03c`
  - `[signal] deliver sig=33 ... a0=0x21 ... a2=0x40022760`
  - `sys_tkill tid=1 signum=33`
  - `[sigreturn] ... sigmask=0x100020000 pc=0x3f03c`

这里的 `pc=0x3f03c` 代表被中断的用户态指令地址，重复出现说明控制流一直被拉回到同一个点，符合“循环”特征。进一步结合 `sigmask` 的取值可以定位到 sigset 布局和恢复逻辑的问题。

注意：本次日志同时出现了 `/dev/shm` 的解析失败，但这只是测试用例的一个小分支错误，并非导致 “SIGCANCEL tkill 循环” 的主因。本报告以 tkill 循环为主，/dev/shm 仅作为附带现象提及。

## 背景与问题复盘

`pthread_cancel_points` 的目标是验证多种取消点是否正确生效，典型路径是：

1. 主线程创建子线程。
2. 子线程进入某个取消点（例如阻塞型系统调用）。
3. 主线程调用 `pthread_cancel` 对子线程发出取消请求。
4. 子线程收到 SIGCANCEL，执行取消处理并退出，返回 `PTHREAD_CANCELED`。

在本次环境中，musl 的取消实现依赖 SIGCANCEL 信号：

- SIGCANCEL 被设置为 `SA_SIGINFO` 信号；
- 取消处理函数内部会对自身执行 `tkill(tid, SIGCANCEL)`，用来推进取消路径；
- 处理完成后通过 `sigreturn` 恢复上下文。

因此，内核对 `sigreturn` 的上下文恢复、对 ucontext 的 sigmask 编解码、以及对重入信号的处理策略都会直接影响取消流程是否能最终结束。

## 关键日志与判断依据

以下日志片段（来自 all205.log）构成循环证据链：

1. **信号投递**
   - `[signal] sig=33 pid=34 tid=1 ... sepc=0x3f03c sp=0x40022aa0`

2. **信号进入处理函数**
   - `[signal] deliver sig=33 pid=34 tid=1 a0=0x21 a1=0x400226e0 a2=0x40022760 sp=0x400226e0`

3. **处理函数内部的自我 tkill**
   - `kernel:pid[34] sys_tkill tid=1 signum=33`

4. **从用户态 ucontext 恢复**
   - `[sigreturn] pid=34 ucontext_ptr=0x40022760 sigmask=0x100020000 pc=0x3f03c`

5. **再次投递同一信号**
   - 再次出现第 1～4 步完整序列

该序列成组重复，是典型的“信号处理返回后立即再次进入信号处理”的循环特征。只要信号屏蔽或恢复逻辑不让 SIGCANCEL 正确阻塞，循环就不会结束。

## 深入分析：为何形成 tkill 循环

### 1) `pc` 恢复是否正确

在 RISC-V 中，`sepc` 记录的是“触发异常或信号时的指令地址”，`sigreturn` 将 `sepc` 恢复为相同值是合理的行为。日志中 `pc=0x3f03c` 重复出现并不一定代表 `pc` 恢复错误，而更可能说明 **信号屏蔽或待处理信号未清除**，导致返回后马上又被 SIGCANCEL 打断。

因此，单靠 `pc` 重复并不能证明 `sigreturn` 错误，但它是循环现象的一个侧面证据。

### 2) sigmask 的关键问题

日志里 `sigmask=0x100020000` 很关键。它是一个用户态 ucontext 的 sigset 值，在 musl 语义中应当表达“当前信号（33）被阻塞”。但内核内部使用的是 `SignalFlags` 布局，其 bit 位置是 **bit = signum**，而用户态 sigset 是 **bit = signum - 1**。

如果内核直接用用户态的 sigmask 作为内部 `SignalFlags`，就会出现：

- 用户态认为 “SIGCANCEL 被屏蔽”
- 内核解码后实际屏蔽的是 SIG32，而 SIGCANCEL 仍然是未屏蔽状态
- SIGCANCEL 返回后立即再次被投递

这恰好解释了日志中的循环。

### 3) 与 musl 取消语义的匹配

musl 在取消处理函数中调用 `tkill(tid, SIGCANCEL)`，并依赖 `sigreturn` 依据 ucontext 恢复正确的 sigmask。如果内核恢复的 mask 不正确，就会产生循环。

因此问题核心不在 “tkill 被调用” 这件事本身，而在 **tkill 后返回的屏蔽语义无法阻止下一次投递**。

## 修复方向与验证思路

### 1) 修复方向（已定位到内核层）

核心修复点是：

- 在写入用户态 ucontext 时，使用用户态布局（bit = signum - 1）；
- 在 `sigreturn` 读取 ucontext 时，转换回内核 `SignalFlags` 布局；
- 这样才能保证 SIGCANCEL 在 handler 期间保持被屏蔽，返回后不会立刻重入。

### 2) 验证思路

验证时应观察以下日志特征：

- `sigreturn` 恢复的 `sigmask` 应该对应 “用户态 SIGCANCEL 被屏蔽” 的值（通常只包含 `1 << (33-1)`）；
- 在 `sigreturn` 之后不应再次出现连续的 `sig=33` 投递序列；
- 线程应当走到取消路径并退出，而不是长期停留在信号处理循环中。

这些观测点可以直接通过 TRACE 日志确认，不需要额外的工具。

## 附带问题：/dev/shm 解析失败（仅简要提及）

在 `pthread_cancel_points` 的运行过程中还出现了：

- `[ERROR] vfs: resolve failed at shm for /dev/shm`

这会导致 `shm_open` 相关子用例提前失败，但它与 SIGCANCEL 的 tkill 循环并非同一问题链路。该问题需要单独的 VFS 支持或挂载补齐，不应混入本次 tkill 循环的根因分析中。

## 调试过程回顾

1. 运行 `LOG=TRACE` 观察 SIGCANCEL 相关日志，确认 `sig=33` 序列重复。
2. 对比 `sigreturn` 日志中的 `sigmask` 与用户态/内核态布局差异。
3. 识别循环由 “mask 编解码不一致” 导致。
4. 将 `/dev/shm` 报错作为附带现象记录，而非主要结论。

## 结论与后续工作（本次不修复，仅记录）

结论：`pthread_cancel_points` 的核心卡点是 **SIGCANCEL 的 tkill 循环**，根因是 **ucontext sigmask 的用户态布局与内核 `SignalFlags` 布局不一致**，导致 SIGCANCEL 未被正确屏蔽，信号返回后立即重入。

后续工作建议：

- 在 `UserContext` 写入与 `sigreturn` 读取处添加 sigmask 布局转换；
- 复测 `pthread_cancel_points`，确保不再出现连续 `sig=33` 投递循环；
- 再单独处理 `/dev/shm` 的 VFS 挂载或路径解析支持。

## 关键日志片段（节选）

```
[signal] sig=33 pid=34 tid=1 ... sepc=0x3f03c sp=0x40022aa0
[signal] deliver sig=33 pid=34 tid=1 a0=0x21 a1=0x400226e0 a2=0x40022760 sp=0x400226e0
kernel:pid[34] sys_tkill tid=1 signum=33
[sigreturn] pid=34 ucontext_ptr=0x40022760 sigmask=0x100020000 pc=0x3f03c
```

该片段重复出现，即是 tkill 循环的直接证据。
