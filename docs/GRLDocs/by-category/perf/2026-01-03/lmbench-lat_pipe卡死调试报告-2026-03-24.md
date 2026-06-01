# lmbench `lat_pipe` 卡死调试报告（musl-lmbench）

日期：2026/3/24

## 结论先行（罪魁祸首）

本次 `musl-lmbench` 卡死的**直接罪魁祸首**是内核 `pipe` 实现中的阻塞循环没有检查 pending signal：  
`os/src/fs/pipe.rs` 在 `read()` / `write()` 等待分支里反复 `suspend_current_and_run_next()`，但没有在循环内检测「当前线程是否已有未屏蔽信号（尤其 SIGKILL）」。  

结果是：`lat_pipe` 子进程被父进程 `kill(SIGKILL)` 后，可能仍陷在内核读写循环里，无法及时返回到 trap 出口执行 `handle_signals()`，进而不能正确转为 zombie，被父进程 `wait4` 回收；最终表现为 `lat_pipe -P 1` 长时间不结束，看起来像“卡死”。

## 一、问题背景与现象

调试目标来自如下场景：

```bash
SINGLE_TEST=musl-lmbench LOG=INFO OFFLINE=1 CARGO_NET_OFFLINE=true bash run.sh -f sdcard-rv.img -t rv > allrv-lmbench-musl.log
```

当前 `initcode` 已裁剪为只跑两项：
1. `lat_sig -P 1 prot lat_sig`
2. `lat_pipe -P 1`

用户侧观察是：通常打印出 `Protection fault: xx.xxxx microseconds` 后，进入 `lat_pipe` 阶段并长时间不返回。

## 二、复现与调试-测试循环

### 第 1 轮：120s 超时复现

使用：

```bash
timeout 120s env SINGLE_TEST=musl-lmbench LOG=INFO OFFLINE=1 CARGO_NET_OFFLINE=true bash run.sh -f sdcard-rv.img -t rv > allrv-lmbench-musl.log 2>&1
```

结论：
1. `lat_sig` 能完成并返回 `rc=0`。
2. `lat_pipe` 开始后超时退出。
3. 说明问题点集中在 `lat_pipe`，不是 `lat_sig` 本身挂死。

### 第 2 轮：扩大超时窗口确认是否“慢”还是“挂”

将超时扩到 300s，仍然在 `lat_pipe` 阶段超时，说明这不是单纯“慢”，而是存在无界等待/退出路径异常。

### 第 3 轮：定向 syscall trace

使用 `TRACE_NAME=lmbench_all` 抓 `lat_pipe` 末段行为。关键序列如下：

1. `pid=4` 与 `pid=5` 长时间进行 pipe 单字节读写 ping-pong（read/write 成功返回 1）。
2. 随后 `pid=4` 触发 `kill(pid=5, SIGKILL)`。
3. 父进程进入 `wait4` 相关流程后没有及时收尸，整体流程停滞。

这条证据链非常关键：它把问题从“普通 pipe 读写错误”收敛到“**被信号终止后的退出/回收链路**”。

## 三、为什么定位到 `pipe` 阻塞循环

内核信号处理触发点在 trap 返回路径（`handle_signals()`）。如果线程长期困在内核函数内部死循环（不回到 trap 出口），即使 `signal_pending` 已置位，也可能不被及时消费。

`os/src/fs/pipe.rs` 的 `read()` / `write()` 逻辑在无数据或无空间时会：

1. 判断条件不满足；
2. `drop(pipe)`；
3. `suspend_current_and_run_next()`；
4. 回来后继续 loop。

这个 loop 没有「信号中断条件」，即使 `sys_kill` 已将目标线程置为 `interrupted_by_signal` 并唤醒，线程恢复运行后仍可能继续在该 loop 内部兜圈，不退出 syscall，也不进入 `handle_signals()`，最终出现父进程 `wait4` 长时间等不到 zombie。

这与 trace 中“已发送 SIGKILL 但 `lat_pipe` 不结束”的现象完全一致。

## 四、修复方案

### 1. 主修复（根因修复）

在 `os/src/fs/pipe.rs` 新增：

- `has_pending_unmasked_signal()`：检查 `(process_pending | task_pending) & !signal_mask` 是否非空。

并在 `read()` / `write()` 的阻塞分支中增加短路返回：

- 若存在未屏蔽 pending signal，直接返回当前已处理字节数（多数场景为 0），让 syscall 尽快返回到 trap 层，交给统一信号处理逻辑收敛。

### 2. 辅助可观测性修正（非根因）

1. `should_trace_syscall()` 默认从“全开”改为“未指定 TRACE 目标时关闭”，避免 INFO 场景下 syscall 日志风暴干扰调试节奏。
2. RISC-V 用户页故障日志从 `error!` 降到 `debug!`，避免 `lat_sig prot` 这种预期 fault 场景产生海量噪声。

说明：这两项不是 `lat_pipe` 卡死根因，但能显著降低噪声、缩短定位路径。

## 五、修复后验证结果

### 验证命令

```bash
timeout 180s env SINGLE_TEST=musl-lmbench LOG=INFO OFFLINE=1 CARGO_NET_OFFLINE=true bash run.sh -f sdcard-rv.img -t rv > allrv-lmbench-musl-fix.log 2>&1
```

关键输出已出现：

1. `Protection fault: ... microseconds`
2. `START lat_pipe -P 1`
3. `Pipe latency: ... microseconds`
4. `DONE  lat_pipe -P 1 rc=0`
5. `#### OS COMP TEST GROUP END lmbench-musl ####`

并且能观察到：

`[signal] pid=5 name=lmbench_all killed by SIGKILL`  
随后父进程流程正常推进并退出，这与预期一致。

### 关于 120s 的结论

修复后：
1. 150s、180s 复测可稳定完成。
2. 120s 仍可能超时，属于性能边界/波动问题，不再是“SIGKILL 后不可回收”的死锁路径。

因此本次调试目标“定位并修复卡死根因”已完成；若后续追求 120s 稳过，需要单独做性能向优化（例如进一步压缩日志、减少 `sigaction` 热路径开销、降低 `lat_pipe` 额外系统调用扰动）。

## 六、关键经验与后续建议

1. 对于“发了 SIGKILL 仍像没死”的问题，优先检查：是否有内核内部阻塞 loop 没有 signal escape hatch。  
2. 只看 `=== All tests completed ===` 不足以判定成功，必须结合关键风险日志和测试项显式完成标记（本案即 `DONE lat_pipe rc=0`）。  
3. trace 策略应优先使用 `TRACE_NAME=目标进程`，避免全局 trace 让定位被日志噪声淹没。  
4. 本次修复点具备通用意义：任何会长期阻塞/轮询的内核 IO 路径都应具备可中断性，否则类似问题可在 futex、pipe、socket 等路径重复出现。

