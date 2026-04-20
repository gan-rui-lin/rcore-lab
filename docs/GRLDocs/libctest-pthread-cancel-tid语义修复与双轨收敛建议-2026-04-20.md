# libctest pthread_cancel TID 语义修复与双轨收敛建议（2026-04-20）

## 1. 背景与目标

本次工作起点是 `SINGLE_TEST=libctest bash run.sh | tee rv-libctest.log` 在 RV 平台上出现 `pthread_cancel` 系列失败。失败现象集中、可重复，且对 musl 结果影响显著。工作目标分为两层：

1. 修复当前可复现失败，确保 `libctest` 回归通过。
2. 在修复后继续做语义一致性审查，避免出现“局部可用但系统口径分裂”的隐藏风险。

本文档记录完整调查链路、关键证据、修复策略、验证结果，以及后续的双轨收敛建议。

---

## 2. 现象与复现

### 2.1 复现命令

```bash
cd /home/grl/codeRepo/rcore-lab
SINGLE_TEST=libctest bash run.sh | tee rv-libctest.log
```

### 2.2 初始症状（修复前）

日志中反复出现：

- `pthread_cancel(td) failed: No such process`
- `FAIL pthread_cancel_points [status 1]`
- `FAIL pthread_cancel [timed out]`
- `FAIL pthread_cancel_sem_wait [status 1]`

并且当时 musl 段结束状态为非零（早期复现为 `status=0x21`），说明 cancel 相关失败会直接拖垮该组结果。

从测试聚类看，失败高度集中于线程定向信号路径（tkill/tgkill），而非广泛调度/文件系统/内存子系统问题。

---

## 3. 调查过程与证据链

### 3.1 第一阶段：确认失败集中于 cancel 路径

对日志进行关键字提取，发现失败几乎全部围绕 `pthread_cancel*`，并且错误码表现为 `No such process`，对应 Linux `ESRCH` 语义。

这意味着“信号目标线程找不到”比“信号处理逻辑错误”更可疑。

### 3.2 第二阶段：定位 tkill/tgkill 目标匹配路径

核查链路：

- `sys_tkill`
- `sys_tgkill`
- `send_signal_to_task_from_list`
- `task_matches_linux_tid`

最终在 `os/src/syscall/process.rs` 中确认目标匹配函数存在用户态 TID 与内核内部 TID 的映射问题。

### 3.3 第三阶段：对照 clone 返回值口径

在 `sys_clone` 线程分支中可见：

```rust
let system_tid = (new_task_tid + 1) as i32;
...
system_tid as isize
```

也就是用户态拿到的是“系统 TID（internal+1）”。

而旧版 `task_matches_linux_tid` 核心判定是：

```rust
internal_tid == target_tid || (internal_tid == 0 && process_pid == target_tid)
```

这会导致非主线程场景下，用户传入 `internal+1` 时，内核按 `internal` 去匹配，出现错位，进而 `ESRCH`。

---

## 4. 根因（一句话）

根因是：**线程定向信号投递（tkill/tgkill）的目标匹配仍按 internal_tid 口径查找，而用户态拿到的线程 ID 来自 clone 的 system_tid（internal_tid+1），两者语义不一致，导致 `pthread_cancel` 在非主线程场景命中 `ESRCH`。**

---

## 5. 修复策略与实现

### 5.1 修复原则

- 不扩大改动面，仅修复目标匹配语义。
- 保持线程组 leader 仍可用 `process_pid` 命中。
- 非 leader 线程按用户可见口径匹配。

### 5.2 实际改动

文件：`os/src/syscall/process.rs`

函数：`task_matches_linux_tid`

修复后逻辑：

- `internal_tid == 0`（主线程）时，要求 `target_tid == process_pid`。
- 非主线程时，要求 `target_tid == internal_tid + 1`。

即：

```rust
if internal_tid == 0 {
    return process_pid == target_tid;
}
(internal_tid + 1) == target_tid
```

该改动直接对齐了 clone 返回给用户态的 TID 口径。

### 5.3 未来方向

Linux 的核心语义是这三条：

每个线程有一个在 pid namespace 内唯一的 TID（不是进程内局部编号）。
线程组组长满足 TID = TGID = PID。
clone 的子线程返回值、gettid、set_tid_address、tkill/tgkill 目标匹配，必须使用同一套 TID 值。
所以你们现在这两种映射：

internal_tid + 1
process_pid + internal_tid
都只是工程近似，不是完整 Linux 语义（因为都没有真正全局唯一 TID 分配）。

---

## 6. 验证矩阵

### 6.1 目标验证（单测回归）

命令：

```bash
cd /home/grl/codeRepo/rcore-lab
SINGLE_TEST=libctest bash run.sh | tee rv-libctest.log
```

关键结果：

- `pthread_cancel_points` 由 FAIL 转为 Pass。
- `pthread_cancel` 由 timed out 转为 Pass。
- `pthread_cancel_sem_wait` 由 FAIL 转为 Pass。
- `musl` 组状态恢复为 `status=0x0`。
- `glibc` 组保持 `status=0x0`。

日志证据（修复后）：

- `#### OS COMP TEST GROUP START libctest-musl ####`
- `#### OS COMP TEST GROUP END libctest-musl ####`
- `=== /musl/libctest_testcode.sh completed (status=0x0) ===`
- `#### OS COMP TEST GROUP START libctest-glibc ####`
- `#### OS COMP TEST GROUP END libctest-glibc ####`
- `=== /glibc/libctest_testcode.sh completed (status=0x0) ===`

### 6.2 评测脚本验证（按抬头分段）

根据实际运行命令与日志抬头，先切段再分别喂官方 judge：

```bash
cd /home/grl/codeRepo/autotest-for-oskernel
awk '/#### OS COMP TEST GROUP START libctest-musl ####/{f=1} f{print} /#### OS COMP TEST GROUP END libctest-musl ####/{f=0}' /home/grl/codeRepo/rcore-lab/rv-libctest.log > /tmp/libctest-musl-section.log
awk '/#### OS COMP TEST GROUP START libctest-glibc ####/{f=1} f{print} /#### OS COMP TEST GROUP END libctest-glibc ####/{f=0}' /home/grl/codeRepo/rcore-lab/rv-libctest.log > /tmp/libctest-glibc-section.log
tr -d '\r' < /tmp/libctest-musl-section.log | python3 kernel/judge/judge_libctest-musl.py > /tmp/libctest-musl-score.json
tr -d '\r' < /tmp/libctest-glibc-section.log | python3 kernel/judge/judge_libctest-glibc.py > /tmp/libctest-glibc-score.json
```

统计结果：

- musl：`217/220`
- glibc：`217/220`
- 缺失项一致：`libctest static crypt`、`libctest static pleval`、`libctest dynamic crypt`

该结果说明本次修复显著改善了 cancel 路径问题，剩余扣分不在本次改动触达范围。

### 6.3 扩展验证（接口语义一致性审计）

除 tkill/tgkill 调用链外，还检查了 TID 生产接口：

- `sys_clone` 当前返回 `internal_tid + 1`
- `sys_gettid` 当前对非主线程返回 `process_pid + internal_tid`
- `sys_set_tid_address` 与 `sys_gettid` 保持同口径

结论：局部修复已生效，但系统层仍存在 TID 口径双轨风险（见下一节）。

---

## 7. 双轨问题说明（当前残留风险）

### 7.1 双轨定义

当前代码中，线程 ID 的“用户可见语义”存在两套规则：

1. **clone/tkill 轨**：非主线程按 `internal_tid + 1`
2. **gettid 轨**：非主线程按 `process_pid + internal_tid`

两套口径并非严格同构，导致“一个线程可对应两个不同用户态 TID 表达”。

### 7.2 风险场景

- 用户态先通过 `gettid()` 取得 tid，再用 `tkill(tid, sig)`，可能与 clone/settid 返回值不一致。
- 未来接入更多线程相关测试时，可能出现“某路径通过、某路径偶发 ESRCH/EINVAL”的隐蔽兼容问题。
- 跨进程场景下，现有简化映射规则不保证严格全局唯一，后续扩展代价会增大。

---

## 8. 改进建议（分阶段）

### 8.1 短期建议：统一内核内部转换入口

新增统一 helper（建议命名示例）：

- `to_user_tid(process_pid, internal_tid)`
- `match_user_tid(process_pid, internal_tid, target_tid)`

并让以下路径统一依赖 helper：

- `sys_clone` 返回值
- `PARENT_SETTID/CHILD_SETTID` 写值
- `sys_gettid`
- `sys_set_tid_address`
- `task_matches_linux_tid`

这样可立即消除“多处手写映射公式”导致的再次分裂。

### 8.2 中期建议：定义并落地单一 TID 语义规范

在 `docs/` 增加线程 ID 语义规范文档，明确：

- leader 与 non-leader 的用户可见表示
- 与 Linux 目标语义的偏差边界
- tkill/tgkill/waittid/clone/gettid 的一致性要求

所有线程相关 syscall 在 CR/评审中对照此规范。

### 8.3 长期建议：引入全局线程 ID 分配/索引

若目标是更强 Linux 兼容（尤其跨进程定向信号），建议引入：

- 全局 tid allocator
- tid -> task 的全局映射
- 线程退出时回收策略

这样可以避免“由局部公式拼凑全局唯一性”的结构性风险。

---

## 9. 被否定的假设（为何不是别的问题）

1. **不是 signal handler 本身错误**：
因为失败前置错误是 `pthread_cancel(td) failed: No such process`，属于投递阶段找不到目标，而非 handler 执行崩溃。

2. **不是 futex 唤醒主因**：
futex 相关问题通常表现为长期挂起或 EINTR 时序异常；本次首因是 ESRCH，且修复 tid 匹配后 cancel 系列立即回归。

3. **不是评测脚本误判主因**：
初次全 0 是 CRLF 与脚本精确匹配 `Pass!` 的输入格式问题；归一化后评分与日志实际通过情况一致。

---

## 10. 本次结论

- 本次改动已修复 `pthread_cancel` 核心失败路径，`libctest` 运行状态恢复到 musl/glibc 均 `status=0x0`。
- 官方 judge 按抬头分段评分后，得到 `217/220 + 217/220`，剩余 3 项缺失与本次信号匹配修复无直接关联。
- 系统层仍存在 TID 双轨残留，需要后续统一映射入口与语义规范，建议优先做短期收敛，避免未来回归。

---

## 11. 后续行动清单（可执行）

1. 提交一个小型重构 PR：抽 `to_user_tid/match_user_tid` helper，并替换 clone/gettid/set_tid_address/task_matches 的散落实现。
2. 增加一个线程 ID 一致性自测：同线程内对比 clone 返回值、gettid、set_tid_address 返回值，并验证 tkill 命中。
3. 在 CI 中加入“按日志抬头分段 + judge_libctest-*.py”评分脚本，固定 `tr -d '\r'` 预处理，避免格式噪声导致误判。
4. 针对剩余 `crypt/pleval` 缺失项做单独追踪文档，避免和本次 cancel 修复混在同一问题面。

---

## 12. 附录：本次关键命令汇总

```bash
# 1) 复现
cd /home/grl/codeRepo/rcore-lab
SINGLE_TEST=libctest bash run.sh | tee rv-libctest.log

# 2) 核查信号路径
rg -n "tgkill|tkill|pthread_cancel|sys_tkill|sys_tgkill" os/src/syscall/process.rs user/src/bin/initcode.rs

# 3) 核查改动来源
cd /home/grl/codeRepo/rcore-lab
git --no-pager blame -L 3299,3316 os/src/syscall/process.rs
git --no-pager show 7b05bfe1 -- os/src/syscall/process.rs | sed -n '260,420p'

# 4) 官方 judge 分段评分
cd /home/grl/codeRepo/autotest-for-oskernel
awk '/#### OS COMP TEST GROUP START libctest-musl ####/{f=1} f{print} /#### OS COMP TEST GROUP END libctest-musl ####/{f=0}' /home/grl/codeRepo/rcore-lab/rv-libctest.log > /tmp/libctest-musl-section.log
awk '/#### OS COMP TEST GROUP START libctest-glibc ####/{f=1} f{print} /#### OS COMP TEST GROUP END libctest-glibc ####/{f=0}' /home/grl/codeRepo/rcore-lab/rv-libctest.log > /tmp/libctest-glibc-section.log
tr -d '\r' < /tmp/libctest-musl-section.log | python3 kernel/judge/judge_libctest-musl.py > /tmp/libctest-musl-score.json
tr -d '\r' < /tmp/libctest-glibc-section.log | python3 kernel/judge/judge_libctest-glibc.py > /tmp/libctest-glibc-score.json
```
