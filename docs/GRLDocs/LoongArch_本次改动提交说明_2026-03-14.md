# 建议提交说明（中文）

## 提交标题

`fix(loongarch): 修正 pthread 取消链路中的 ucontext/siginfo ABI 与取消点语义`

## 提交正文

LoongArch 路线在 `pthread_cancel` / `pthread_cancel_points` / `tls_*` 用例中出现了“信号到达但取消不生效”与异常风暴问题。根因涉及三层：

1. 用户态 `cancel_handler` 对 `ucontext` 的读取偏移有严格假设（`uc_mcontext` 需位于 `ucontext + 176`）；
2. `SIGCANCEL`（SIG33）的 `siginfo` 内容不完整会影响 libc 取消处理路径判断；
3. 取消点相关 syscall（`tgkill/clock_nanosleep/nanosleep`）语义缺失会导致线程无法在预期时机被打断。

同时，LoongArch 未对齐地址异常在用户态路径上会触发高频 trap 循环，放大“卡死/泄露”表象。

### 主要改动

- 修正 LoongArch 信号栈入参的 ABI 细节：
  - 调整 `LinuxSigInfo` 内部布局，确保 payload 偏移与用户态预期一致；
  - 对 `SIG32/SIG33` 填充 `SI_TKILL` 与 `si_pid`；
  - 增加 `UserContext` 编译期布局断言，保证 `uc_mcontext` 偏移为 176。
- 补齐取消链路系统调用：
  - 新增 `sys_tgkill`；
  - 新增 `sys_clock_nanosleep`；
  - 在 `sys_nanosleep` 中处理 `SIG32/SIG33` 取消信号的可中断返回（`EINTR`）。
- 修复 LoongArch trap 分类缺口：
  - 为 `AddressNotAligned` 增加模拟处理并继续执行，避免异常循环。

### 效果

- `pthread_cancel` 路径由“信号到达但不跳转”恢复为可生效状态；
- 早期“卡在 [pci]/日志刷屏”的异常风暴明显缓解；
- 为后续 `tls_get_new_dtv` 等剩余动态/TLS尾部问题提供稳定调试基线。

### 影响范围

- `os/src/task/mod.rs`
- `os/src/syscall/process.rs`
- `os/src/trap/mod.rs`

### 备注

- RV 路线未观察到本轮改动导致的明确功能回归（用户手动回归结果）。
- 若后续继续收敛尾部失败，建议优先跟进 `dlopen/tls_get_new_dtv` 的动态加载与 TLS 动态重定位路径。
