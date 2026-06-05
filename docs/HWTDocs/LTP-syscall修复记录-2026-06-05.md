# LTP syscall 修复记录（2026-06-05）

## sched_rr_get_interval

修改位置：

- `os/src/syscall/mod.rs`
- `os/src/syscall/process.rs`

本轮补齐 `sched_rr_get_interval(2)` 的基础语义，主要面向 LTP 的
`sched_rr_get_interval01`、`sched_rr_get_interval02`、
`sched_rr_get_interval03`：

- 在 syscall 表中接入 127 号调用，并加入 syscall 名称映射。
- 复用已有 `sched_setscheduler()` 记录的进程调度策略。
- `SCHED_RR` 返回 100ms 时间片。
- `SCHED_FIFO` 返回 0。
- `pid == -1` 返回 `EINVAL`。
- 不存在 pid 返回 `ESRCH`。
- 无效 timespec 用户指针返回 `EFAULT`。

验证命令：

```bash
make -C os kernel LOG=ERROR OFFLINE=1
SINGLE_TEST=/glibc/ltp/testcases/bin/sched_rr_get_interval01 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
SINGLE_TEST=/glibc/ltp/testcases/bin/sched_rr_get_interval02 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
SINGLE_TEST=/glibc/ltp/testcases/bin/sched_rr_get_interval03 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
```

验证结果：

| 用例 | 结果 |
|------|------|
| `sched_rr_get_interval01` | PASS，libc 与 syscall old spec 变体均返回 0s 100000000ns |
| `sched_rr_get_interval02` | PASS，FIFO 策略返回 0s 0ns |
| `sched_rr_get_interval03` | PASS，覆盖 `EINVAL` / `ESRCH` / `EFAULT` |

说明：

- 未运行从 `sched_rr_get_interval01` 开始的连续小窗口，因为当前日志中紧随其后的
  `sched_setaffinity01` 是已知 timeout 风险点，本轮按单用例验证避免拖住内核。
- 构建仅出现 vendor 目录既有 `unexpected_cfgs` 警告。
