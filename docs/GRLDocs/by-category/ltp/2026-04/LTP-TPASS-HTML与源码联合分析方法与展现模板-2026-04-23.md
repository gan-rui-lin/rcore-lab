# LTP TPASS：基于 HTML 与源码的联合分析方法与展现模板（2026-04-23）

## 1. 目标

本文档用于统一以下工作流：

1. 基于评测 HTML 快速识别 TPASS 缺口。
2. 结合 LTP 与内核源码定位“为什么没过”。
3. 形成可执行的后续改进方向与优先级。
4. 以固定格式展示分析结果，便于跨轮次对比。

适用场景：

- `htmls-9000/开放课程.html` 一类离线评测快照。
- 需要在 `glibc/musl`、`rv/la` 之间做横向差分。
- 需要把“统计结论”落到“具体 syscall 语义缺口”。

---

## 2. 输入与产出

### 2.1 输入

- HTML 快照：`/home/grl/codeRepo/rcore-lab/htmls-9000/开放课程.html`
- LTP 源码：`/home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524/testcases`
- 内核源码：`/home/grl/codeRepo/rcore-lab/os/src`

### 2.2 产出

必须至少包含 4 份信息：

1. `case` 级别差分清单（按 pass/all 差值排序）。
2. 方向级聚合结论（如 `sched_*`、`preadv*`、`msg*`）。
3. 根因分类（stub/缺分发/errno 不匹配/权限语义/状态管理）。
4. 改进路线图（短中期优先级 + 回归计划）。

---

## 3. 分析流程（标准版）

### 步骤 A：从 HTML 提取统计差分

使用现有脚本：

```bash
python3 scripts/analyze_ltp_html.py \
  --html htmls-9000/开放课程.html \
  --ltp-root /home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524/testcases \
  --output ltp_glibc_musl_comparison.txt
```

脚本会输出：

- `ltp-glibc` 与 `ltp-musl` 总体 `pass/all` 差值。
- `case` TopN（按 `pass_diff/all_diff`）。
- 方向聚合（按路径自动分类）。

### 步骤 B：定义问题集合

建议固定分三类：

1. 双 libc 都完全未过：`pass = 0 && all > 0`
2. 共享 case 的差分缺口：`glibc` 明显落后 `musl`
3. musl-only 高价值点：可能是覆盖链路问题，也可能是语义缺失

说明：

- 第 1 类最适合优先做“内核共性语义补齐”。
- 第 2 类适合做“同 case 下 glibc 行为对齐”。
- 第 3 类先判断是否为“未跑到”而不是“内核不会”。

### 步骤 C：源码反查（LTP -> syscall -> 内核实现）

对每个高价值 case，按下面顺序定位：

1. 打开 LTP case 源码，确认断言点（预期 errno、返回值、权限条件）。
2. 找 syscall 号与分发：`os/src/syscall/mod.rs`
3. 找 syscall 实现：`os/src/syscall/fs.rs` / `process.rs` / `net/syscall.rs`
4. 判断根因类型：
   - 未实现（`ENOSYS`/stub）
   - 已实现但 errno 错
   - 参数校验缺失
   - 权限/凭证语义缺失
   - 进程状态或生命周期污染（PID 复用、残留状态）

### 步骤 D：估算投入产出比（ROI）

建议给每个方向打 3 个分：

- 价值分 `V`：可回收 `all` 槽位规模（可参考 `all_sum`）
- 成本分 `C`：实现复杂度（1-5，5 最复杂）
- 风险分 `R`：引发回归概率（1-5，5 最高）

可用简化指标：

`ROI = V / (C + R)`

排序时优先：

1. `V` 高、`C` 中低、`R` 中低
2. 语义收敛（一个补丁覆盖多个 case）
3. 可快速定向回归验证

### 步骤 E：定向回归并闭环

每次改动后至少做两层验证：

1. 目标 family 的单测回归（如 `sched_*`）
2. 相邻 family 的冒烟回归（防止 errno/权限改动外溢）

对每个 case 记录：

- `TPASS/TFAIL/TBROK/TCONF`
- 退出码（`status=0x...`）
- 是否是“内核语义问题”还是“用户态/环境问题”

---

## 4. 根因分类字典（推荐）

建议统一使用以下标签，避免每轮重造术语：

- `DISPATCH_MISSING`：号表/分发链路缺失
- `STUB_IMPL`：实现存在但仅返回固定值
- `ERRNO_MISMATCH`：行为接近但错误码不符合 Linux 语义
- `ARG_VALIDATION_GAP`：参数范围/空指针/地址校验不完整
- `PERMISSION_GAP`：euid/capability/特权语义不完整
- `STATE_LIFECYCLE_GAP`：进程状态在 fork/exit/reap 生命周期污染
- `LIBC_ENV_GAP`：glibc/musl、NSS、运行环境导致的非内核 TBROK

---

## 5. 后续改进方向（建议分层）

### 5.1 短期（高 ROI，1-3 天一轮）

- `sched_*` 参数与 errno 家族
- `preadv/preadv2` 与 iovec/flags/error-path
- `syslog`/`sethostname`/`setdomainname` 等单点语义缺口

目标：

- 快速回收“分数大 + 语义集中”的 case。

### 5.2 中期（中等复杂度，按专题推进）

- SysV IPC 系列（`msg*`、`shm*`）边界语义
- 文件权限与路径细分语义（`access/stat/open/chown/chmod`）
- wait/waitid/times 等框架依赖路径

目标：

- 降低 glibc 与 musl 的系统性差距。

### 5.3 长期（高复杂度能力建设）

- `name_to_handle_at/open_by_handle_at`、namespace、pidfd、mqueue 等
- 需要 VFS/namespace/安全模型深层支撑的接口

目标：

- 从“补兼容点”转向“补内核能力面”。

---

## 6. HTML 分析结果的展现形式（推荐）

建议每轮输出固定 4 层视图，避免“只有一张大表”：

### 视图 1：一页摘要（管理视图）

字段建议：

- 总 `pass/all`（glibc、musl、总差值）
- Top 3 改进方向
- 本轮新增 TPASS
- 本轮回归风险点（有无新增 TBROK/TFAIL）

### 视图 2：方向看板（决策视图）

| 方向 | pass增量潜力 | all潜力 | 成本 | 风险 | 当前状态 | 下一步 |
|---|---:|---:|---:|---:|---|---|
| sched_* | 高 | 高 | 中 | 低 | 进行中 | 补齐剩余 case |
| preadv* | 高 | 高 | 中 | 中 | 已完成/回归 | 稳定性回归 |

### 视图 3：Case TopN（执行视图）

| case | glibc(pass/all) | musl(pass/all) | 差值 | LTP路径 | 根因标签 | owner |
|---|---|---|---|---|---|---|
| sched_setparam03 | 0/8 | 8/8 | +8 | kernel/syscalls/... | STATE_LIFECYCLE_GAP | xxx |

### 视图 4：单 case 语义卡（调试视图）

模板建议：

```text
Case: sched_setparam03
Expectation (LTP): sched_setscheduler/setparam/getparam 行为与 errno
Observed: syscall 变体 TBROK, libc 变体 TPASS
Root cause: PID 复用后调度状态残留
Fix: 进程 reap 时清理 per-pid sched state
Regression: musl/glibc 均 TPASS，无新增 TFAIL
```

---

## 7. 周报/里程碑展示模板（可直接复制）

```markdown
## 本周 LTP 进展

- 数据源：htmls-9000/开放课程.html（日期：YYYY-MM-DD）
- 总体：glibc pass X/Y，musl pass A/B，差值 +N
- 本周新增 TPASS：+K

### 已完成
1. 方向：sched_*
2. 关键修复：补 syscall 分发 + errno 对齐 + 进程状态回收
3. 结果：case1/case2/... 通过（附日志路径）

### 进行中
1. 方向：XXX
2. 当前阻塞：XXX
3. 下周计划：XXX

### 风险
1. glibc `getpwnam(nobody)` 路径出现 EFAULT，导致若干 case TBROK（非本轮 syscall 语义引入）
```

---

## 8. 质量门槛（建议）

当一个方向标记为“完成”时，建议满足：

1. 目标 case 在 musl 与 glibc 至少各回归 1 轮。
2. 无新增 `TFAIL/TBROK`（`TCONF` 需注明原因）。
3. 文档中有“根因 -> 修复点 -> 回归结果”的闭环记录。
4. 下轮优先级看板已更新。

---

## 9. 实操建议

1. 先用 HTML 差分筛选，不要直接从 syscall 列表盲修。
2. 高价值 case 必须先读 LTP 源码断言，再写内核补丁。
3. 每轮只推进 1-2 个 family，确保可回归、可归因。
4. 对 `glibc-only TBROK` 单独归类，避免误判为内核语义回退。

