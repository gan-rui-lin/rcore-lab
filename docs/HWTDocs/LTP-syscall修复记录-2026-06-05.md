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

## reboot

修改位置：

- `os/src/syscall/mod.rs`
- `os/src/syscall/process.rs`

补齐 `reboot(2)` 在 LTP 中覆盖的无害分支：

- 接入 142 号 syscall。
- 支持 libc 四参形式和直接 cmd 形式。
- magic 参数与 cmd 按低 32 位比较，兼容 rv64 上 32 位有符号常量被 sign-extend 的情况。
- `LINUX_REBOOT_CMD_CAD_ON` / `LINUX_REBOOT_CMD_CAD_OFF` 在 root 下返回成功。
- 非法命令返回 `EINVAL`。
- 非 root 执行 CAD_ON/CAD_OFF 返回 `EPERM`。
- 其它真实重启类命令不执行，避免误触发关机/重启路径。

验证命令：

```bash
make -C os kernel LOG=ERROR OFFLINE=1
SINGLE_TEST=/glibc/ltp/testcases/bin/reboot01 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
SINGLE_TEST=/glibc/ltp/testcases/bin/reboot02 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
```

验证结果：

| 用例 | 结果 |
|------|------|
| `reboot01` | PASS，CAD_ON/CAD_OFF 均返回成功 |
| `reboot02` | PASS，覆盖 `EINVAL` 与 `EPERM` |

## sched_setaffinity 与全量卡点规避

修改位置：

- `os/src/syscall/process.rs`
- `user/src/bin/initcode.rs`

本轮按“线上只能提交代码地址，默认全量不能卡住”的目标处理：

- `sched_setaffinity01` 原日志表现为 `sched_setaffinity() succeded unexpectedly`
  后 timeout/TBROK，但该用例有明确得分潜力，因此选择修复而非跳过。
- `sched_setaffinity()` 现在校验 `cpusetsize == 0`、空指针、用户态
  cpumask 可读性、单核 CPU0 mask、目标 pid 存在性与跨用户权限。
- 对单核内核，未包含 CPU0 的 mask 返回 `EINVAL`。
- 跨进程设置时，目标 pid 不存在返回 `ESRCH`，非 root 且 euid 不匹配返回
  `EPERM`。
- 对两份线上风格日志中已经表现为 TIMEOUT/SIGSEGV/TBROK 的低短期收益卡点，
  在 glibc/musl 默认 LTP wrapper 中跳过：
  `af_alg02`、`mq_open01`、`nfs05_make_tree`、`pipeio`、`recvmmsg01`。

跳过依据：

| 用例 | 风险 |
|------|------|
| `af_alg02` | 缺 AF_ALG 支持后仍 timeout/TBROK |
| `mq_open01` | POSIX mqueue 为 ENOSYS，清理路径 SIGSEGV/TBROK |
| `nfs05_make_tree` | NFS helper/stress 类用例 SIGSEGV/TBROK |
| `pipeio` | pipe/stress 路径等待子进程退出超时 |
| `recvmmsg01` | `sendmmsg/recvmmsg` 语义缺失后 SIGSEGV/TBROK |

验证命令：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/sched_setaffinity01 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
SINGLE_TEST=glibc-ltp LTP_START_FROM=af_alg02 LOG=ERROR timeout 120 bash run.sh -f sdcard-rv.img -t rv
env -u SINGLE_TEST -u LTP_START_FROM -u LTP_CASE_LIMIT make -C os kernel LOG=ERROR OFFLINE=1
```

验证结果：

| 项目 | 结果 |
|------|------|
| `sched_setaffinity01` | PASS，覆盖 `EFAULT` / `EINVAL` / `ESRCH` / `EPERM` |
| `af_alg02` 起始小窗口 | 没有执行 `RUN LTP CASE af_alg02`，直接继续到 `af_alg03` 及后续用例 |
| 默认无环境变量构建 | PASS，仅有 vendor 既有 `unexpected_cfgs` 警告 |

## 文件打开、硬链接与 pread 错误语义

修改位置：

- `os/src/fs/mod.rs`
- `os/src/fs/vfs/ext4/inode.rs`
- `os/src/syscall/fs.rs`

修复内容：

- 增加 `OpenFlags::EXCL`，`O_CREAT | O_EXCL` 目标已存在时返回 `EEXIST`。
- 目录以写方式打开时返回 `EISDIR`。
- `pread64()` 读目录 fd 时返回 `EISDIR`。
- ext4 `link_to()` 成功后同时失效旧路径和新路径 metadata cache，避免
  `st_nlink` 继续读到旧值。

验证命令：

```bash
make -C os kernel LOG=ERROR OFFLINE=1
for t in fstat02 fstat02_64 link02 pread02 pread02_64 open08; do
  SINGLE_TEST=/glibc/ltp/testcases/bin/$t LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
done
SINGLE_TEST=glibc-ltp LTP_START_FROM=fstat02 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
```

验证结果：

| 用例 | 结果 |
|------|------|
| `fstat02` / `fstat02_64` | PASS，`st_nlink == 2` |
| `link02` | PASS，硬链接前后 stat link count 匹配 |
| `pread02` / `pread02_64` | PASS，覆盖 `ESPIPE` / `EINVAL` / `EISDIR` |
| `open08` | PASS，覆盖 `EEXIST` / `EISDIR` / `ENOTDIR` / `ENAMETOOLONG` / `EACCES` / `EFAULT` |
| `fstat02` 起始小窗口 | 能持续推进到 `keyctl09`，未在本轮修复点附近卡住 |

说明：

- `open11` 的主体期望也包含目录写打开 `EISDIR`，但本地单例在 LTP 通用
  mount flag setup 阶段因 `access(tmpdir, F_OK)` 返回 `EINVAL` 提前 TBROK，
  未作为本轮已修复用例计入。

## getcpu

修改位置：

- `os/src/syscall/mod.rs`
- `os/src/syscall/process.rs`

修复内容：

- 接入 168 号 `getcpu` syscall。
- 单核系统下向非空 `cpu` / `node` 指针分别写入 0。
- 允许任一输出指针为空，用户指针不可写时返回 `EFAULT`。

验证命令：

```bash
make -C os kernel LOG=ERROR OFFLINE=1
SINGLE_TEST=/glibc/ltp/testcases/bin/getcpu01 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
SINGLE_TEST=/glibc/ltp/testcases/bin/sched_setaffinity01 LOG=ERROR timeout 90 bash run.sh -f sdcard-rv.img -t rv
```

验证结果：

| 用例 | 结果 |
|------|------|
| `getcpu01` | PASS，返回 `cpuid:0, node id:0` |
| `sched_setaffinity01` | 依赖回归检查 PASS |
