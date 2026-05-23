# UserContext 字段适配调试记录

日期：2026/03/05

## 背景

近期在 rcore-lab 上验证 musl 的 pthread_cancel 小测试时出现稳定卡死。内核日志显示 SIGCANCEL 已经递送并完成 handler 调用，但线程依然回到用户态自旋，pthread_join 永远无法返回。通过进一步比对用户态打印与内核日志，确定这是一个“信号到达了，但用户态取消路径没有被触发”的问题。

在这类问题中，关键路径是：内核构造 siginfo/ucontext 传入用户态 handler → handler 可能会修改 ucontext 中的 PC（跳转到 libc 的 __cancel）→ sigreturn 从 ucontext 中恢复上下文。只要 ucontext 字段不符合 Linux 语义，handler 对 ucontext 的修改就可能被内核错误读取，从而导致 PC 无法跳转，线程回到原来的忙循环。

因此，这次调试的核心怀疑点是：UserContext（ucontext_t）/MContext（mcontext_t）的字段布局与 Linux 语义不一致，导致 sigreturn 读取的 PC 和 sigmask 都不正确。

## 现象与直接证据

1. 用户态在异步线程中显式设置了 PTHREAD_CANCEL_ASYNCHRONOUS，且打印显示已生效。
2. SIGCANCEL 信号确实被递送（trap → handle_signals → sigreturn 完整执行）。
3. sigreturn 日志显示 ucontext_pc 与 saved_pc 完全一致，且 SIGCANCEL 被放入 mask，之后线程继续执行原先的自旋代码。

这意味着：用户态 handler 虽然执行了，但它对 ucontext 的修改并没有被内核正确理解或恢复。最容易出错的位置就是“ucontext 中 PC 的字段位置”和“sigmask 的布局”。

## 罪魁祸首判断

罪魁祸首是 UserContext 字段布局与 Linux 语义不一致，具体表现在两个方面：

- **sigmask 字段大小**：原实现用 `u64` 表示 sigmask，而 Linux 下的 `sigset_t` 是 128 字节（16 * u64）。musl 在 SA_SIGINFO 下使用 `ucontext_t`，会认为 sigmask 是完整的 128 字节结构。如果内核只保存 8 字节，用户态对 mask 的写入会覆盖后续字段或被内核误读。
- **mcontext/fpregs 结构布局**：原实现用一块固定大小的字节数组表示 FPU 状态，这种布局不一定匹配 Linux RISC-V 的 mcontext 定义。musl/ABI 假定 fpregs 的对齐和大小与 Linux 一致。如果内核提供的结构不匹配，用户态对 mcontext 的写入就可能偏移，从而导致 PC 写入位置与内核读取位置不一致。

这两个问题直接导致：

- sigreturn 从 ucontext 中读到的 PC 没有变化（实际上 handler 已经写了，但写在了内核没有读取的位置），所以线程回到原自旋位置。
- SIGCANCEL 被写入到 mask 后，内核恢复了错误的 mask，导致 SIGCANCEL 进一步被屏蔽，取消逻辑无法再触发。

这就是“信号已经处理，但线程继续卡死”的根因。

## 调试思路与分析过程

1. **验证用户态状态是否正确**：在小测试中打印 cancel state/type 与 sigmask，确认 SIGCANCEL 未被屏蔽且 canceltype 已切换为异步。该阶段排除了“用户态没设置成功”的可能性。
2. **确认信号递送是否完整**：内核日志显示 tkill、handle_signals、sigreturn 都被调用，说明信号确实投递且 handler 被触发。
3. **检查 ucontext 恢复路径**：sigreturn 打印的 ucontext_pc 始终等于 saved_pc，提示内核未读取到 handler 修改的 PC。
4. **定位结构体布局问题**：回到 UserContext/MContext 定义，发现 sigmask 用 u64，且 fpregs 是一段字节数组，明显与 Linux RISC-V 的 ucontext_t/mcontext_t 不对齐。
5. **对照 Linux 语义**：Linux 下 sigset_t 为 128 字节；mcontext 包含通用寄存器、浮点寄存器及 fcsr，且对齐要求较严格。结合其他仓库（同为 RISC-V 的 Linux 语义实现）比对，确认当前实现偏差较大。

最终结论：UserContext 字段布局是导致 cancel handler 无法生效的直接原因。

## 适配思路与架构说明

为了对齐 Linux 语义，需要从结构布局和读写路径两方面一起修正：

1. **UserContext 的 sigmask**
   - 从单个 `u64` 扩展为 `[u64; 16]`，对应 Linux 的 128 字节 sigset_t。
   - 内核恢复时只取 `sigmask[0]` 映射为 SignalFlags（兼容现有 1..64 信号位）。
   - 这种方式不会影响现有 signal mask 的逻辑，但保证结构大小和布局正确。

2. **MContext / FPU 字段布局**
   - 用显式的 `RiscvFpRegs { f[32], fcsr }` 替代裸字节数组。
   - 确保对齐和大小符合 Linux RISC-V 预期，让 musl 写入的 PC 和寄存器能被正确读取。

3. **sigreturn 恢复路径**
   - 从 `ucontext.uc_sigmask[0]` 恢复 mask，避免误读。
   - PC 仍从 `uc_mcontext.gregs[0]` 读取，这是现有 ABI 约定，但前提是 mcontext 布局正确。

这样可以保证：
- handler 修改 ucontext 的 PC 能被内核恢复；
- sigmask 不会覆盖 mcontext 等后续字段；
- musl 的 cancel handler 能正确推进取消流程。

## 修改重点与难点

### 修改重点

- **UserContext 的 sigmask 字段**：改为 `[u64; 16]`，并在构造与恢复时只使用第 0 个 u64。
- **MContext 的 fpregs 字段**：改为显式结构 `RiscvFpRegs`，对齐 Linux 语义。
- **sigreturn 读取**：日志和恢复逻辑改为读取 `uc_sigmask[0]`。

### 难点

- **结构布局的 ABI 兼容性**：这类结构可能被用户态 libc 直接访问，任何字段大小/顺序不一致都会造成隐蔽错误。
- **对齐和填充**：RISC-V mcontext 涉及寄存器、浮点寄存器和控制寄存器，结构内存对齐必须正确，否则用户态读写会错位。
- **调试难度高**：问题表现为“信号处理后仍回到旧 PC”，无法直接从功能层面定位，需要通过字段布局推断。

## 结论

本次卡死的真正原因不是调度，也不是信号递送失败，而是 UserContext/MContext 的字段布局与 Linux 语义不一致，导致用户态取消 handler 修改的 PC 与 sigmask 无法被内核正确恢复。

修正 UserContext 的 sigmask 宽度以及 mcontext/fpregs 的布局，可以使 sigreturn 恢复出的 PC 与用户态 handler 的预期一致，从而让 pthread_cancel 的异步取消生效，避免线程继续自旋。

## 后续建议

1. **验证结构大小**：在内核侧增加静态断言或日志，验证 UserContext/MContext 的 size/align 是否符合 Linux RISC-V 语义。
2. **验证 PC 恢复**：在取消 handler 中临时输出 ucontext 中 PC 的字段值，确认内核恢复位置一致。
3. **补充测试**：新增一个小测试，在 handler 中显式修改 ucontext PC，验证 sigreturn 后是否跳转成功。
4. **逐步清理兼容代码**：如果后续对齐了完整 ABI，可清理之前用于兜底的 SIGCANCEL 日志/特殊处理。

通过上述修正与验证，信号与取消路径可以更贴近 Linux 语义，并消除当前的 pthread_cancel 卡死问题。