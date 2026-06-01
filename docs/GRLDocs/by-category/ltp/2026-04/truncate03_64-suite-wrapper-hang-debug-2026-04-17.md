这是一份针对该调试报告的中文翻译，采用了专业且符合中文技术文档习惯的表达方式：

---

# truncate03_64 测试套件包装器挂起调试报告 (2026-04-17)

## 1. 问题陈述

在 rv 上执行完整的 LTP 套件期间，运行偶尔会在 `truncate03_64` 附近挂起。单例执行 `truncate03_64` 可以正常完成并打印总结，但在套件模式下，运行可能会停止推进，并最终以 QEMU 进程终止告终。

本报告重点说明如何定位该问题、排除了哪些假设、修改了哪些代码路径，以及已完成或待处理的验证工作。

## 2. 环境与范围

- **仓库**: rcore-lab
- **架构**: riscv64 (`run.sh` 路径)
- **调查的主要日志**: `ltp-rv01.log`
- **相关代码**: `user/src/bin/initcode.rs`，以及 `run_ltp_suite` 中生成的 LTP 包装脚本。

## 3. 调查时间线（如何定位问题）

### 阶段 A：确认真实日志与实际失败位置

最初由于使用不同的日志导致了混淆。纠正后，分析锚定在 `ltp-rv01.log`。

**关键命令模式：**
`rg -a -n "truncate03_64|truncate03|RUN LTP CASE|FAIL LTP CASE|TIMEOUT LTP CASE|QEMU: Terminated|All tests completed" ltp-rv01.log`

**关键发现：**
- 第 228579 行：`RUN LTP CASE truncate03`
- 第 228649 行：`FAIL LTP CASE truncate03 : 1`
- 第 228671 行：`RUN LTP CASE truncate03_64`
- 第 228783 行：`QEMU: Terminated`
- **不存在** `FAIL LTP CASE truncate03_64` 相关行。

**分析结论：**
停滞点并非在进入 `truncate03_64` 之前。套件已到达并运行了 `truncate03_64`，但在打印该用例的包装层（wrapper-level）FAIL 行之前卡住了。

### 阶段 B：验证 truncate03_64 自身是否实际退出

对日志末尾窗口的提取显示：
- `truncate03_64` 测试用例输出了完整的 Summary（总结）统计块。
- 出现了 `truncate03_64` 进程的 `exit_group`：
    - `pid=225 name=truncate03_64 code=0`
    - `pid=226 name=truncate03_64 code=1`

这证明测试用例进程**确实已经退出**。因此，挂起很可能发生在子进程退出后的套件包装器控制流中，而非 `truncate` 系统调用的执行过程中。

### 阶段 C：检查末尾附近的内核恐慌或即时崩溃迹象

末尾附近的信号显示：
- 定时器心跳日志仍在继续。
- 末尾附近出现了一个 `itimer` 警告：
    - 第 228765 行：`[itimer] pid=229 SIGALRM fired`
- 随后出现 `QEMU: Terminated`（第 228783 行）。

在最后窗口中没有出现新的 Panic 或 Trap 爆发。因此，观察到的运行结束行为更符合“包装层等待路径未完成，随后被外部终止”的情况。

### 阶段 D：排除 PID 耗尽假设

对 `ltp-rv01.log` 进行 PID 扫描：
- `rg -a -o 'pid=[0-9]+' ltp-rv01.log`
- `rg -a -o 'pid\[[0-9]+\]' ltp-rv01.log`

**结果摘要：**
- 样本数：94677
- 唯一 PID 数：235
- 最大 PID：235

检查 `os/src/task/id.rs` 中的内核分配器显示，其为带有回收列表的单调分配器，且该路径中没有极小的硬上限常量。最大 PID 为 235 并不代表 PID 空间耗尽。

### 阶段 E：将症状映射到包装脚本逻辑

调查转向 `user/src/bin/initcode.rs` 中生成的 `run_ltp_suite` Shell 脚本。

在修复前，`run_case_with_timeout` 使用以下模式：
1. 在后台启动用例。
2. 在循环中轮询 `kill -0 case_pid`。
3. 超时后，执行 `kill -9 case_pid`。
4. **仅在**循环退出后才执行 `wait case_pid`。

这种模式在长时间套件运行中极易受时序竞态（timing races）影响，特别是在进程生命周期和回收时机方面。症状吻合点：
- 测试用例已退出（可见 `exit_group`）。
- 包装器未打印 FAIL 行。
- 定时器/信号噪声仍在持续。

这有力地表明，包装器控制流卡在了超时/轮询路径附近，而非 `truncate03_64` 的系统调用语义问题。

### 阶段 F：解释修复包装器后的核心转储（Core-dump）信号

应用“监视器 + 等待”（watchdog + wait）包装器更改后，从 `truncate03` 开始的窄缩运行到达了：
- `RUN LTP CASE tst_fs_has_free`
- 二进制文件打印了：
    - `Set variables TCID, TST_TOTAL, and TST_COUNT before each test`
- 随后因信号 6 (`SIGABRT`) 退出，Shell 输出显示 `Aborted (core dumped)`。

**关键分析细节：**
- 在 `SINGLE_TEST` 模式下，`initcode` 会打印原始等待状态，例如 `status=0x86`。
- `0x86` 解码为信号 6 并带有 core 标志位，因此这也是一个中止路径（即便在该模式下不一定总是打印 Shell 风格的 "Aborted" 文本）。

**本阶段结论：**
- 此核心转储不是超时包装器重构的副作用。
- 运行进度更进一步，并暴露了一个单独的过滤缺口：带有 `tst_*` 前缀的辅助二进制文件仍被当作独立用例启动。

## 4. 根因陈述

套件层面的停滞是由 `run_case_with_timeout` 中脆弱的超时包装逻辑引起的：它依赖于重复的 `kill -0` 轮询和延迟的 `wait` 处理，这可能导致包装器在测试用例退出后的边缘时序条件下卡住，从而阻止输出最终的 `FAIL LTP CASE` 行。

简而言之：是**包装器控制流竞态**，而非 `truncate03_64` 系统调用语义不匹配。

## 5. 为什么其他假设被拒绝

1. **truncate03_64 内部无限挂起**
    - 被拒绝，因为测试用例打印了完整的总结并输出了 `exit_group` 行。
2. **达到 PID 限制**
    - 被拒绝，因为观察到的最大 PID 仅为 235，且分配器路径没有硬性的微小上限。
3. **末尾发生即时内核恐慌**
    - 被拒绝，因为末尾窗口未显示新的 Panic 级联，而是显示定时器心跳直到外部终止。

## 6. 修复策略与实现

### 策略

将重度轮询的超时循环替换为**以等待为中心**的监视器设计：
- 父路径直接对用例 PID 进行 `wait`。
- 独立的监视器进程休眠 `case_timeout` 时间，如果用例仍存活则将其杀死。
- 使用超时标记文件来决定返回码是否应强制为 124。
- 用例 `wait` 返回后清理监视器。

这减少了竞态发生的可能性，并使用户例完成的决策由 `wait` 驱动，而非重复的 `kill -0` 轮询。

### 修改的文件

- `user/src/bin/initcode.rs`
    - glibc 生成脚本分支：修改 `run_case_with_timeout`
    - musl 生成脚本分支：修改 `run_case_with_timeout`
    - glibc 生成脚本分支：`is_skip_case` 现在会跳过 `tst_*` 辅助二进制文件
    - musl 生成脚本分支：`is_skip_case` 现在会跳过 `tst_*` 辅助二进制文件

### 关键新控制流

1. 创建超时标记文件路径。
2. 在后台启动用例进程。
3. 启动监视器：
    - 休眠 `case_timeout`
    - 如果用例仍存活，写入超时标记并执行 `kill -9`。
4. `wait case_pid` 并捕获返回码（ret）。
5. 停止监视器并清理环境。
6. 如果存在超时标记，打印 TIMEOUT 并返回 124；否则返回原始返回码。

## 7. 验证状态

### 已完成的证据验证

- **确认了 `ltp-rv01.log` 中的旧症状**：
    - `truncate03_64` 进入并退出。
    - 缺失 `FAIL LTP CASE truncate03_64` 行。
    - 最终 QEMU 终止。
- **确认了 `user/src/bin/initcode.rs` 中两个脚本分支的代码替换**。
- **确认了窄缩运行日志中关于 `tst_fs_has_free` 的辅助程序中止特征**：
    - 标记位在 `truncate03` 处启用并匹配。
    - 出现了辅助程序使用横幅（`TCID/TST_TOTAL/TST_COUNT`）。
    - 出现了信号 6 (`SIGABRT`) 和核心转储消息。
- **确认了 `SINGLE_TEST` 状态解码**：
    - `status=0x86` 对应信号 6 且带有 core 标志位（中止语义）。
- **确认了跳过列表（skip-list）的加固已应用于两个脚本分支**：
    - `tst_*` 已添加到现有的 `tst_*.sh` 规则旁，以避免直接执行辅助二进制文件。

### 进行中 / 待处理的运行时验证

已启动窄缩复现：
`LTP_START_FROM=truncate03 LTP_CASE_LIMIT=2 LOG=INFO timeout 240 bash run.sh`

观察到的命令退出码：143（由外部 `timeout` 包装器杀死）。由于此运行尚未产生修复后的清理通过信号，运行时验证仍处于待处理（pending）状态。

**关于退出码层级的说明：**
- `143` 来自宿主机外部的 `timeout` 命令（SIGTERM 杀死 `bash run.sh`）。
- `124` 是超时工具在满足超时条件时的语义代码。
- 测试用例级的 `status=0x86` 是客户机侧辅助进程的等待状态（SIGABRT + core 位），不应与宿主机外部的超时代码混淆。

## 8. 建议的下一步验证矩阵

1. **窄域复现窗口**
   `LTP_START_FROM=truncate03 LTP_CASE_LIMIT=2 LTP_CASE_TIMEOUT=30 LOG=INFO timeout 600 bash run.sh`
   **预期：** 出现 `RUN` 和 `FAIL` 行，包装器推进到下一步或按限制退出，无长时间的心跳尾部。

2. **中等规模回归窗口**
   `LTP_START_FROM=timerfd_settime02 LTP_CASE_LIMIT=20 LTP_CASE_TIMEOUT=15 LOG=INFO timeout 900 bash run.sh`
   **预期：** 无包装器层级的用例后停滞，超时行受限且后跟 FAIL 行。

3. **广泛套件稳定性验证**
   `LTP_CASE_TIMEOUT=8 LOG=INFO timeout 1800 bash run.sh`
   **预期：** 套件后期用例附近不再反复出现 FAIL 行缺失的症状。

## 9. 残余风险

- 修复消除了一种已知的竞态模式，但在极长时间的套件运行中，仍可能暴露内核进程/信号处理中其他 `wait/reap` 的边缘情况。
- 如果挂起持续存在，下一步探测应在 Shell 侧和内核侧对用例 PID 和监视器 PID 的 `wait` 转换进行插桩。

## 10. 总结

通过以下证据链定位了故障：
1. 到达了 `truncate03_64`。
2. `truncate03_64` 测试用例退出。
3. 缺失包装器层级的 `truncate03_64` FAIL 行。
4. 日志末尾显示监视器/定时器活动，随后外部终止。

基于此，在两个 LTP 脚本分支中将超时包装逻辑重写为“监视器 + 等待”语义，直接针对控制流竞态进行了修复。