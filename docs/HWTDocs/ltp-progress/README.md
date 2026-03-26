# musl-LTP 测试提分进度总结

## 分数变化

| 版本 | 分数(TPASS) | 测试数 | 最后一个测试 | 备注 |
|------|------------|--------|-------------|------|
| allrv07 (基线) | 307 | 107 | cgroup_fj_proc | 旧版，首次完整跑分 |
| allrv00 (当前) | 314 | 107 | cgroup_fj_proc | 代码修改后，重新解压镜像跑分 |

**提升: +7 TPASS**

## 已完成的代码修改

### 1. procfs 完善 (`os/src/fs/vfs/procfs.rs`)

**问题**: 多个测试因缺少 `/proc/self/mounts` 等文件而 TBROK 或 TFAIL。

**修改内容**:
- `/proc/self/mounts` — cgroup_core01-03 从 TBROK 变为 TCONF (不再卡住报错)
- `/proc/self/mountinfo` — 提供 mountinfo 格式的挂载信息
- `/proc/self/mountstats` — 基础挂载统计
- `/proc/self/cgroup` — 返回 `0::/\n`，cgroup 测试需要
- `/proc/self/status` — 包含 Name/Pid/PPid/Uid/Gid/VmRSS/Threads
- `/proc/self/fd` — 空目录占位
- `/proc/self/cmdline`, `/proc/self/environ` — 基础占位
- `/proc/cgroups` — 空的 cgroup 控制器列表头部
- `/proc/filesystems` — 列出 sysfs/tmpfs/proc/ext4/devtmpfs
- `/proc/mountinfo` — 根级别 mountinfo 文件
- `proc_mounts()` 格式修正为 `rootfs / rootfs rw 0 0` 开头

### 2. 网络: AF_UNIX 域套接字骨架 (`os/src/net/unix_socket.rs`, `os/src/net/syscall.rs`, `os/src/fs/mod.rs`)

**问题**: bind01/bind03/bind04/bind05 因 `socket(AF_UNIX, ...)` 返回 EAFNOSUPPORT 而全部失败。

**修改内容**:
- 新建 `unix_socket.rs`: `UnixSocketFile` 结构体 + 全局 path->socket 注册表
- `sys_socket` 支持 `AF_UNIX` 域，创建并返回 `UnixSocketFile`
- `sys_bind` 支持 AF_UNIX 地址解析 (pathname + abstract socket)
  - EADDRINUSE 检测 (注册表查重)
  - EAFNOSUPPORT (inet socket 绑定 AF_UNIX 地址)
  - EINVAL (已绑定的 socket 再次绑定)
- `File` trait 新增 unix socket 系列方法 (bind/listen/accept/connect/read/write/poll)
- `read_sockaddr_family()` / `read_unix_sockaddr()` 地址解析辅助函数

**当前状态**: 编译通过，但尚未实际测试。bind 基本逻辑完整，connect/listen/accept 框架已有但未完善。

### 3. 系统调用扩展 (`os/src/syscall/mod.rs`, `os/src/syscall/process.rs`)

**新增 syscall**:
- `rt_sigsuspend` (133) — cgroup_fj_proc 等需要
- `capget/capset` (90/91) — 能力管理，返回基本的 root 全能力
- `setreuid/setregid` (145/143) — UID/GID 切换
- `setresuid/getresuid/setresgid/getresgid` (147-150) — 完整的 UID/GID 管理
- `prctl` (167) — 基础实现 (PR_SET_NAME/PR_GET_NAME 等)
- `adjtimex` (171) — 时钟调整 stub
- `sys_accept` 新增 flags 参数支持 (SOCK_CLOEXEC/SOCK_NONBLOCK)

### 4. 进程管理增强 (`os/src/task/process.rs`)

- `ProcessControlBlockInner` 新增字段: `saved_uid`, `saved_gid`, `name` (进程名)
- 支持完整的 UID/GID 保存/恢复语义

## 已发现但未解决的问题

### 1. cgroup_fj_proc 卡住 (180s 超时)

**现象**: 二进制调用 `sigsuspend` 等待 SIGUSR1，但信号永远不会到达，导致无限阻塞。
**影响**: 测试执行到第 107/108 个时卡住，消耗剩余全部时间。
**尝试过的方案**:
- 脚本层 15s per-test 后台超时 -> **失败**: 内核 `wait`/`kill` 对后台子进程的支持不够完善，导致每个测试都要等 15s，9 个测试就耗尽 180s。
- 需要从内核层面解决: 要么修复 wait4 的 PID 过滤，要么在内核中实现信号超时机制。

### 2. cgroup_regression_*.sh 无限循环

**现象**: shell 脚本中 `while true; do mkdir ...; rmdir ...; done`，永不退出。
**当前**: 排在 cgroup_fj_proc 之后，目前不影响分数（被前一个卡住挡住了）。

### 3. AF_UNIX connect/accept 未完成

**缺少**: `sys_connect` 的 AF_UNIX 路径、peer 链接、`sys_listen`/`sys_accept` 的 AF_UNIX 分支。
**预计收益**: bind01 (+2)、bind04、bind05、accept03 (+1) 等可能提升。

### 4. sdcard 镜像老化

**现象**: 多次 debugfs 写入后镜像运行速度下降。
**解决**: 重新解压原始镜像恢复正常速度。以后修改后如果速度异常考虑重新解压。

## 潜在提分方向 (优先级排序)

1. **修复 cgroup_fj_proc 卡住** — 释放后续测试执行时间，可能多跑几个 cgroup 测试
2. **完善 AF_UNIX connect/accept** — 影响 bind01 (+2)、bind04/05、accept03 (+1)，预计 +3~5
3. **修复 wait4 PID 过滤** — 让 per-test 超时脚本可用，作为通用防卡方案
4. **添加更多 /proc 文件** — 按测试需求逐步补全
