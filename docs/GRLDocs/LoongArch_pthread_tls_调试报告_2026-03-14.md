# LoongArch pthread/TLS 调试报告（2026-03-14）

## 1. 问题背景

在 LoongArch 路线执行全量测试时，出现过以下现象：

- `run-la.sh -t all` 早期看起来“卡在 [pci]”或无新增输出。
- `pthread_cancel` / `pthread_cancel_points` / `tls_*` 等用例不稳定失败。
- GDB 现场曾停在 `virtio_drivers::queue::VirtQueue::add_notify_wait_pop`，初看像 I/O 死锁。

与之对比，RISC-V 路线同批测试可通过，因此重点怀疑 LoongArch 的 ABI 适配与信号返回路径。

## 2. 调试路径与关键证据

### 2.1 先排除“假卡死”

- 通过 `LOG=INFO/DEBUG` 与分段日志比对，确认多数场景不是内核硬死锁，而是异常/信号路径重复触发导致进展极慢。
- `pkill qemu-system-loongarch64` 后重跑可复现，说明是逻辑问题而非偶发环境噪声。

### 2.2 修复 LoongArch 未对齐访问异常风暴

- 发现 LoongArch 用户态出现大量 `AddressNotAligned` 相关陷入，导致反复 trap 循环和日志刷屏。
- 在 `os/src/trap/mod.rs` 的 LoongArch trap 分发中补上 `AddressNotAligned` 处理，调用未对齐访存模拟并直接继续执行。
- 结果：`unknown trap` 风暴消失，最初“卡住”症状显著缓解。

### 2.3 锁定 `ucontext` 布局问题（核心）

用户给出“RV 曾因 ucontext 结构体出错”的线索后，采用“反汇编用户态 libc + 对齐内核结构体”的方式验证。

关键证据：

1. 从镜像提取 LoongArch `libc.so`（`/musl/lib/libc.so`）。
2. 反汇编 `cancel_handler`，观察到其固定使用 `ucontext + 176` 读写返回 PC。 
3. 这要求内核传给用户态的 `UserContext` 必须满足：`uc_mcontext` 偏移为 `176`。

结论：若 `UserContext` 在 `uc_sigmask` 后额外填充不当，会导致 `cancel_handler` 改写错误地址，表现为：

- `sigreturn` 时 `return_pc == saved_pc`，取消跳转未生效；
- `pthread_cancel*` 出现“信号到了但取消没发生”的失败模式。

### 2.4 针对取消点的系统调用行为补齐

为兼容 musl 取消点行为，还补充了：

- `tgkill(131)` 映射到 `sys_tgkill`；
- `clock_nanosleep(115)` 映射到 `sys_clock_nanosleep`；
- `sys_nanosleep` 中对取消信号（`SIG32/SIG33`）的可中断语义处理。

并做了对照实验：

- 去掉 `sys_nanosleep` 中取消信号中断逻辑后，`pthread_cancel` 明显回退失败；
- 恢复该逻辑后，`pthread_cancel` 结果恢复，说明这是必要条件。

## 3. 本轮关键改动

### 3.1 `os/src/task/mod.rs`

- 调整 `LinuxSigInfo` 内部填充，确保 `si_pid` 等 payload 偏移与用户态预期一致。
- `setup_signal_stack` 对 `SIG32/SIG33` 填充 `SI_TKILL` 和 `si_pid` 信息。
- 新增编译期断言，确保 `UserContext.uc_mcontext` 偏移固定为 `176`（与 musl cancel handler 约定一致）。

### 3.2 `os/src/syscall/process.rs`

- 增加 `sys_tgkill`。
- 增加 `sys_clock_nanosleep` 并复用 `sys_nanosleep`。
- 在 `sys_nanosleep` 中处理 `SIG32/SIG33` 触发的取消中断语义。

### 3.3 `os/src/trap/mod.rs`

- LoongArch trap 分发补齐 `AddressNotAligned` 分支，避免用户态未对齐访问导致异常循环。

## 4. 结果与当前状态

- LoongArch 路线最初“卡住/刷屏”主因已定位并显著缓解。
- `pthread_cancel` 相关行为较初始状态明显改善。
- 动态加载/TLS 相关仍有残余失败（如 `tls_get_new_dtv`），表现为后续路径问题，不再是最初的同类根因。

## 5. 经验总结

- LoongArch 与 RV 在 `ucontext/sigcontext` ABI 细节上不能直接复用“看起来相近”的布局，必须以目标 libc/内核头文件和反汇编结果为准。
- 对于信号+线程取消问题，单看内核日志容易误判，最好结合：
  - `tkill/tgkill` 投递链路，
  - `handle_signals` 入栈参数，
  - `sigreturn` 的 `saved_pc/return_pc` 对比。
- “像死锁”的现象常由异常风暴引起，先排异常分类是否完整，收益很高。
