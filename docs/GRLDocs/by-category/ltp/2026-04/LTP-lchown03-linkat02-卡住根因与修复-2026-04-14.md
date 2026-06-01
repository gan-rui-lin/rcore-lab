# LTP `lchown03/lchown03_16/linkat02` 卡住根因与修复（2026-04-14）

## 1. 现象

在 LTP 过滤脚本中运行以下 case 时，日志长期停在：

- `RUN LTP CASE lchown03`
- `RUN LTP CASE lchown03_16`
- `RUN LTP CASE linkat02`

早期现象是卡住后无法继续统计；后续加了 case 超时后，仍能看到部分 case 在 `openat` 后无进展。

## 2. 复现方式

典型复现命令（RISC-V）：

```bash
SINGLE_TEST=all LTP_START_FROM=lchown03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh
SINGLE_TEST=all LTP_START_FROM=linkat02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh
```

## 3. 根因分析

### 3.1 主根因（内核 ext4 seek 语义不兼容）

`lwext4` 的 `ext4_fseek(SEEK_SET)` 对 `offset > fsize` 返回 `EINVAL(22)`；
而 Linux 语义允许文件偏移定位到 EOF 之后，再由后续 write 扩展文件。

这会导致 LTP 中依赖“先 seek 到大偏移、再写入/分配”的路径进入异常分支，表现为：

- `openat(..., 0x8241, ...) ret=3` 后出现 `ext4_fseek: rc = 22`
- case 无法按预期完成，部分场景表现为“看起来卡住”

### 3.2 次要放大因素（测试脚本超时回收策略）

`run_case_with_timeout()` 在 `kill -9` 后立即 `wait pid`，当子进程处于不易回收状态时，父脚本也可能二次阻塞。

## 4. 修复方案

### 4.1 ext4 seek 兼容修复（关键）

文件：`vendor/lwext4_rust/src/file.rs`

在 `file_seek()` 中对以下情况做兼容：

- `ext4_fseek` 返回 `EINVAL`
- `seek_type == SEEK_SET`
- `offset >= 0`

则直接更新 `file_desc.fpos = offset` 并返回成功，保持 Linux 用户态可见语义（允许定位到 EOF 后）。

### 4.2 ext4 write_at 兜底扩展优化

文件：`os/src/fs/vfs/ext4/inode.rs`

`write_at()` 在 seek 失败（越 EOF）时，改为从当前 size 到目标 offset 的大块补零（1 MiB chunk），降低大文件扩展时的极端慢路径风险。

### 4.3 LTP case 超时回收防阻塞

文件：`user/src/bin/initcode.rs`

`run_case_with_timeout()` 在超时后：

- `kill -9` 后先短轮询 `kill -0`
- 仅在确认进程已退出时再 `wait`
- 避免父脚本在 `wait` 上无限挂起

## 5. 验证结果

修复后关键行为变化：

1. `linkat02` 不再卡在 `openat` 后，`fallocate` 可返回：
   - `num=56(openat) ... ret=3`
   - `num=47(fallocate) ... ret=0`
2. `lchown03/lchown03_16` 不再出现原先“openat 后卡死”现象，能继续执行并给出 `FAIL LTP CASE ... : 2`。

结论：

- **“卡住”主问题已被解除**。
- 当前剩余为功能兼容失败（返回码/语义差异），属于下一阶段对齐问题，不再是本次 hang 根因。

## 6. 后续建议

- 针对 `FAIL ... : 2` 的 case，分别按 syscall 语义补齐（权限、errno、边界条件）。
- 若继续做大规模 LTP 扫描，建议保留 case 级 timeout 与非阻塞回收逻辑，避免单点异常拖垮整轮统计。
