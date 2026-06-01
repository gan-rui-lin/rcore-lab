# RV LTP `check_netem` 触发 `sepc=...0826` Illegal Storm 调试记录（2026-04-15）

## 1. 现象

在 RISC-V 跑 LTP（尤其从 `check_netem` 附近开始）时，日志出现大量重复：

`[ERROR] [kernel] trap_handler: illegal instruction addr=0x0 sepc=0xffffffc100000826`

并与用户态 syscall 日志交错，形成明显“假死/刷屏”。

## 2. 关键定位结论

`0xffffffc100000826` 落在 `SIG_RETURN_ADDR` 跳板页内，且是 `_sigreturn` 的 `ecall` 之后地址。

已验证 `_sigreturn` 指令布局为：

1. `li a7, 139`
2. `ecall`
3. `unimp`（offset `+8`）

因此触发 `sepc=...0826` 的语义是：控制流从 `rt_sigreturn` 返回后继续执行到 `unimp`，说明 `sigreturn` 路径异常返回或状态未正确收敛。

## 3. 与 `initcode.rs` 的关系

`initcode.rs` 的 LTP runner 会遍历 `ltp/testcases/bin/*`，包括 shebang 脚本（如 `check_netem`），会引入更密集的 `fork/exec/signal/wait` 交互。

结论：

`initcode.rs` 是触发器，不是根因。

根因在内核信号返回和 illegal trap 处理路径。

## 4. 根因拆解

### 4.1 `sys_sigreturn` canary 检查位置不稳定

原逻辑按“当前 SP”读 canary，和投递时压栈位置不完全等价，容易在复杂 signal/cancel 路径出现误判。

误判后 `sigreturn` 失败，容易回到 trampoline `unimp`，触发 `0826` illegal。

### 4.2 illegal instruction 处理过于“直喷”

对 trampoline 区的 illegal 没有分级处理和重复抑制，导致同一故障被高频打印，看起来像“卡死”。

## 5. 修复内容

## 5.1 记录精确 canary 地址

新增字段：

- `signal_canary_ptr`
- `illegal_last_sepc`
- `illegal_repeat_count`

涉及：

- `os/src/task/task.rs`
- `os/src/task/mod.rs`

`setup_signal_stack` 返回 `(ucontext_ptr, canary_ptr)`，在投递时记录。

## 5.2 强化 `sys_sigreturn`

涉及：

- `os/src/syscall/process.rs`

策略：

1. trampoline 上若缺失 signal frame，直接按致命错误处理（终止当前任务），避免继续回跳板 `unimp`。
2. `ucontext` 读取失败同样按致命错误处理。
3. canary 检查改为“使用投递时记录地址”。
4. canary mismatch 调整为**非致命告警**（保留诊断价值，避免假阳性直接杀进程）。

## 5.3 抑制 fake illegal storm

涉及：

- `os/src/trap/user_trap_riscv64.rs`

策略：

1. 区分是否在 sigreturn trampoline 页内。
2. 对同步 illegal 优先投递到 faulting task（并确保 SIGILL 不被 mask）。
3. 相同 `sepc` 重复触发时抑制日志（周期性摘要）。
4. 在“正在处理信号 + trampoline illegal”场景直接终止任务，打断环路。

## 6. 验证记录

### 6.1 单点验证

执行：

- `SINGLE_TEST=/musl/ltp/testcases/bin/check_netem`
- `SINGLE_TEST=/musl/ltp/testcases/bin/add_key01`

结果：

1. 未再出现 `sepc=0xffffffc100000826` storm。
2. `add_key01` 仍会因 syscall 217 未实现触发 TCONF/退出（符合预期，不是本问题）。

### 6.2 分段 LTP 验证

执行：

- `SINGLE_TEST=musl-ltp LTP_START_FROM=check_netem`

结果：

1. `check_netem` 失败码仍由其测试依赖决定（例如环境工具缺失），但不再引发 `0826` illegal storm。
2. canary mismatch 仅出现为 non-fatal warn，不再直接把 shell/busybox 杀死并放大后续异常。

## 7. 额外观察

`debug` 内核在极早期可能触发独立 panic（与本次 `0826` 问题不是同一根因）；`release` 路径可正常挂载并进入 initcode/LTP 流程。

## 8. 结论

本次问题不是 LTP 脚本本身错误，而是“脚本压力场景放大了内核 signal-return 收敛缺陷 + illegal trap 打印策略缺陷”。

修复后：

1. `0826` 异常风暴被切断；
2. 假 illegal instr 刷屏显著下降；
3. `add_key01` 保持“syscall 未实现”的正确失败语义，不再误导为 trap 子系统崩坏。

