# LTP `sigtimedwait01` 兼容方案与 `getrusage02`“卡死”误判调试记录（2026-04-16）

## 1. 背景

本轮问题有两条线：

1. LoongArch 上 `sigtimedwait01` 出现“日志持续输出且不退出”的卡死现象。
2. RISC-V 上 `getrusage02` 已打印 `Summary passed`，但后面仍看到 `FAIL LTP CASE getrusage02 : 0`，容易误判为“最后阶段没返回（wait 卡住）”。

另外，`run.sh` 默认跑的是 RV（`BUILD_TYPE="rv"`），这一点在排查路径里需要明确。

## 2. 现象复盘

### 2.1 LA: `sigtimedwait01`

现象（用户日志）：

- `sigwait.c:29: TFAIL: Expected error number EINTR, got: EINVAL (22)`
- 多处 `struct siginfo mismatch`
- 曾出现测试不退出/看起来一直在跑

### 2.2 RV: `getrusage02`

在 `rv-ltp.log` 中，`getrusage02` 已经是：

- `getrusage02.c:64: TPASS: getrusage(0, 0xffffffffffffffff) : EFAULT (14)`
- `Summary: passed 3 failed 0 broken 0 skipped 1 warnings 0`

但随后又打印：

- `FAIL LTP CASE getrusage02 : 0`

这条 `FAIL` 和 `ret=0` 同时出现，已经说明它更像脚本输出错误，而不是内核没有返回。

## 3. 调试过程（详细）

### 3.1 先确认 RV 侧“是否真卡住”

先看 `rv-ltp.log` 的 case 粒度输出，重点是 `getrusage02` 前后：

- 有完整 Summary
- 且 Summary 后继续进入下一个 case

结论：`getrusage02` 本身并未卡死；流水线继续推进。

### 3.2 追脚本：为什么会出现 `FAIL ... : 0`

定位到 `user/src/bin/initcode.rs` 中生成 LTP 脚本的逻辑：

- 旧逻辑是每个 case 执行完后无条件 `echo "FAIL LTP CASE $case_name : $ret"`
- 所以即便 `ret=0` 也会打印 FAIL

这就是 RV 侧误判的直接根因。

### 3.3 LA 侧 `sigtimedwait01` 不退出的内核/用户态交互

排查时观察到：

- `sigtimedwait01` 场景里，用户态（musl）对 `sigtimedwait` 的错误处理会对某些错误码进行重试；
- 在“空等待集 + 无 timeout + 持续被其他信号打断”的组合下，容易进入反复等待/重试，表现为“日志一直刷不退出”。

这里不是 COW 问题，根因在信号等待语义和 libc 重试行为的耦合。

## 4. 兼容方案与代码改动

### 4.1 `sys_rt_sigtimedwait` 兼容防卡死

文件：`os/src/syscall/process.rs`

新增兼容分支：

- 当 `sigset == 0 && timeout == NULL` 时，直接返回 `EINVAL`。

目的：

- 避免 musl 在该组合下陷入“被打断-重试-再等待”的长循环，从“无限挂住”降级为“可退出但有一项语义差异（line 29）”。

这是有意识的兼容取舍：优先保证测试集可推进、系统不被单例卡死。

### 4.2 LTP 脚本输出修正（RV getrusage 误判根因修复）

文件：`user/src/bin/initcode.rs`

将 case 结束后的输出改为按返回码分支：

- `ret == 0` -> `PASS LTP CASE ...`
- `ret != 0` -> `FAIL LTP CASE ... : ret`

修复效果：

- 不再出现 `FAIL LTP CASE xxx : 0` 这种误导信息；
- 可以直接从 case 行判断是否失败，减少误判“卡死/没返回”。

## 5. 关联改动（信号元数据一致性）

为让 `sigwaitinfo/sigtimedwait` 的 `siginfo` 更接近 Linux 行为，本轮还整理了信号元数据路径：

- 进程级 pending 信号补充 sender pid / si_code 记录与清理；
- 发送信号、SIGCHLD、定时器信号等路径同步写入元数据；
- 消费 pending 信号时同步清理元数据，避免陈旧信息复用。

相关文件：

- `os/src/task/process.rs`
- `os/src/task/mod.rs`
- `os/src/syscall/process.rs`