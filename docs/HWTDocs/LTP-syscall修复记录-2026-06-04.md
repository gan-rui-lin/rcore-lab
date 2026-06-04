# LTP syscall 修复记录（2026-06-04）

## 1. 概述

本轮主要围绕 glibc LTP 中一批 syscall 兼容性问题做小步修复，并按语义拆分提交。目标不是完整实现 Linux 内核能力，而是在 rCore 当前抽象下补齐 LTP 依赖的基础行为、参数校验、errno 优先级和少量环境探测文件。

已推送到 `feat/hwt` 的提交：

| 提交 | 说明 |
|------|------|
| `ce7a9a2` | `fix: 完善at2和pidfd基础语义` |
| `8ebdb07` | `fix: 完善clock_adjtime和posix timer校验` |
| `1b94179` | `fix: 完善copy_file_range偏移语义` |
| `99a96ae` | `fix: 提供LTP内核配置探测文件` |

## 2. at2 / pidfd / procfs 基础语义

### 2.1 `faccessat2` / `faccessat`

修改位置：`os/src/syscall/fs.rs`

主要补齐：

- 新增 `sys_faccessat2()`，复用 `sys_faccessat()` 语义。
- 校验 `AT_EACCESS | AT_SYMLINK_NOFOLLOW | AT_EMPTY_PATH` 之外的非法 flag。
- 绝对路径忽略 `dirfd`。
- 支持 `AT_EMPTY_PATH`。
- `AT_EACCESS` 使用 effective uid/gid 权限口径。
- `AT_SYMLINK_NOFOLLOW` 走 nofollow 解析。

覆盖用例：

| 用例 | 结果 |
|------|------|
| `faccessat201` | PASS |
| `faccessat202` | PASS |
| `faccessat01` | PASS |

### 2.2 `openat2`

修改位置：`os/src/syscall/fs.rs`

主要补齐：

- 增加 `OpenHow` 解析。
- 校验 `size` 与尾部扩展字段，非零扩展字段返回 `E2BIG`。
- 校验 `mode` 与 `O_CREAT/O_TMPFILE` 的组合。
- 接受常见 `resolve` 位：`NO_XDEV`、`NO_MAGICLINKS`、`NO_SYMLINKS`、`BENEATH`、`IN_ROOT`。
- 为 LTP 覆盖的 `/proc/*`、符号链接、`..` 等场景返回预期 errno。
- 对 `/proc/self/exe` 返回一个占位 fd，满足探测路径。

覆盖用例：

| 用例 | 结果 |
|------|------|
| `openat201` | PASS |
| `openat202` | PASS |
| `openat203` | PASS |

### 2.3 `pidfd_open`

修改位置：`os/src/syscall/fs.rs`

主要补齐：

- `flags != 0` 返回 `EINVAL`。
- 超大 pid 返回 `EINVAL`。
- `pid == 0` 或不存在的 pid 返回 `ESRCH`。
- 存在的 pid 返回占位 fd，并设置 `FD_CLOEXEC`。

覆盖用例：

| 用例 | 结果 |
|------|------|
| `pidfd_open01` | PASS |
| `pidfd_open02` | PASS |
| `pidfd_open03` | PASS |
| `pidfd_open04` | TCONF，LTP 判定需要 Linux 5.10 的 `PIDFD_NONBLOCK` |

### 2.4 `/proc/version`

修改位置：`os/src/fs/vfs/procfs.rs`

新增 `/proc/version`，返回最小可解析内容：

```text
Linux version 5.10.0 (rcore-lab)
```

这个版本号也和后续 `/boot/config-5.10.0` 对齐。

## 3. `clock_adjtime` 与 POSIX timer

修改位置：`os/src/syscall/process.rs`

### 3.1 `clock_adjtime`

新增 `sys_clock_adjtime(clockid, buf)`：

- 支持 `CLOCK_REALTIME` 和 `CLOCK_MONOTONIC`。
- 非法 clock id 返回 `EINVAL`。
- 复用 `adjtimex` 的读写与权限语义。

同时增强 `sys_adjtimex()`：

- 增加 `TimexState` 保存 LTP 会修改和回读的字段：
  - `offset`
  - `freq`
  - `maxerror`
  - `esterror`
  - `status`
  - `constant`
  - `tick`
- `ADJ_*` 写入后，下次读取能回显，修复 `clock_adjtime01` 的 verify 阶段失败。
- 保留非 root 设置时间参数返回 `EPERM`。
- 保留非法 tick 范围返回 `EINVAL`。

验证结果：

| 用例 | 结果 |
|------|------|
| `clock_adjtime01` | 9 passed, 0 failed |
| `clock_adjtime02` | 6 passed, 0 failed |

### 3.2 `timer_create`

主要补齐：

- `timerid == NULL` 返回 `EFAULT`。
- `clockid >= MAX_CLOCKS` 返回 `EINVAL`。
- `sigevent *` 不可读时返回 `EFAULT`，不再静默当作默认 `SIGALRM`。
- 校验 `sigev_notify`，非法通知类型返回 `EINVAL`。
- 校验 `sigev_signo`，非法信号编号返回 `EINVAL`。
- `timer_t` 按 LTP 当前 ABI 以 `i32` 写回，避免用 `usize` 覆盖用户栈相邻字段。

说明：`timer_create01` 在当前 guest 镜像中不存在，运行时 `Exec ... failed (ret=-2)`，因此未作为通过项统计。

### 3.3 `timer_settime`

主要补齐：

- `new_value == NULL` 返回 `EINVAL`。
- `tv_nsec >= 1_000_000_000` 返回 `EINVAL`。由于当前 `TimeSpec.tv_nsec` 是 `usize`，用户传入负数会表现为超大无符号值，也会被该检查覆盖。

验证结果：

| 用例 | 结果 |
|------|------|
| `timer_settime02` | 48 passed, 0 failed |
| `timer_gettime01` | 3 passed, 0 failed |
| `timer_getoverrun01` | 2 passed, 0 failed |
| `timer_delete01` | 8 passed, 0 failed |

仍未解决：

- `timer_settime01` 仍为 `TBROK: Test killed by SIGTERM`。
- 该问题更像是定时器信号投递、唤醒或测试超时路径问题，不是本轮修复的参数校验问题。

## 4. `copy_file_range`

修改位置：`os/src/syscall/fs.rs`

主要补齐：

- `flags != 0` 返回 `EINVAL`。
- 支持 `off_out`。
- 拒绝负的 `off_in/off_out`，返回 `EINVAL`。
- `len == 0` 直接返回 0。
- 写入目标文件，并以实际写入字节数作为返回值。
- 显式传入 offset 时更新用户态 offset 指针，但恢复 fd 原本 offset。
- 未显式传入 offset 时推进 fd 当前 offset。

验证结果：

| 用例 | 结果 |
|------|------|
| `copy_file_range03` | 2 passed, 0 failed |

未作为语义失败统计：

- `copy_file_range01` / `copy_file_range02` 在 LTP setup 阶段 `TBROK: Failed to acquire device`，还没有跑到 syscall 语义检查。

## 5. LTP kconfig 探测文件

修改位置：`os/src/fs/mod.rs`

LTP 的 `tst_kconfig` 在 `/proc/config.gz` 不存在时会回退读取：

```text
/boot/config-$(uname -r)
```

因此补充：

- 创建 `/boot`。
- 写入 `/boot/config-5.10.0`。
- 内容包含当前 rCore 已经提供或用 stub 支撑的基础能力：
  - `CONFIG_SYSVIPC=y`
  - `CONFIG_POSIX_TIMERS=y`
  - `CONFIG_EPOLL=y`
  - `CONFIG_EVENTFD=y`
  - `CONFIG_SIGNALFD=y`
  - `CONFIG_TIMERFD=y`
  - `CONFIG_INOTIFY_USER=y`
  - `CONFIG_PROC_FS=y`
  - `CONFIG_TMPFS=y`
  - `CONFIG_EXT4_FS=y`

该文件需要和 `/proc/version` / uname 暴露的 `5.10.0` 保持一致。

## 6. 构建与验证命令

构建：

```bash
make -C os kernel LOG=ERROR OFFLINE=1
```

单用例运行模板：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/<case> LOG=ERROR timeout 180 bash run.sh -f sdcard-rv.img -t rv
```

本轮重点验证通过：

```text
faccessat201
faccessat202
faccessat01
openat201
openat202
openat203
pidfd_open01
pidfd_open02
pidfd_open03
clock_adjtime01
clock_adjtime02
timer_settime02
timer_gettime01
timer_getoverrun01
timer_delete01
copy_file_range03
```

## 7. 后续建议

优先看以下几个方向：

1. `timer_settime01`：检查 POSIX timer 到期后的信号投递、任务唤醒和 `clear_signal()` 忙等退出条件。
2. `copy_file_range01/02`：先解决 LTP mount device 获取失败，否则这两个用例无法覆盖 syscall 主体语义。
3. `timer_create01/02/03`：如果后续 guest 镜像重新包含 `timer_create` 测试目录，需要重新跑一遍。本轮源码语义已按这些用例补了基础校验，但当前镜像缺少 `timer_create01` 二进制。
4. kconfig 项不要过度声明。后续如果某项能力退化或未实现，应同步调整 `/boot/config-5.10.0`，避免 LTP 误入更深测试路径。
