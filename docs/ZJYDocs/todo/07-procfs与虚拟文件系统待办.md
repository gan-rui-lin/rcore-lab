# procfs 与虚拟文件系统待办事项

**日期**: 2026/06/04

---

## 1. 缺失 /proc 条目

以下 /proc 条目在 LTP 测试中被访问但返回 ENOENT，导致 TBROK：

### 1.1 /proc/self/ns/* — 命名空间（6+ 测试）

| 路径 | 影响测试 | 优先级 |
|------|---------|-------|
| /proc/self/ns/pid | ioctl_ns01-06 | P2 |
| /proc/self/ns/uts | ioctl_ns01-06 | P2 |
| /proc/self/ns/user | ioctl_ns01-06 | P2 |
| /proc/self/ns/mnt | ioctl_ns01-06 | P2 |
| /proc/self/ns/net | ioctl_ns01-06 | P2 |

**建议**: 创建虚拟符号链接文件，readlink 返回 `ns:[inode_number]` 格式。不需要实现真正的 namespace 隔离。

### 1.2 /proc/meminfo 完善（2+ 测试）

**现状**: 已实现但部分字段缺失。

**待办**:
- [ ] 添加 VmallocTotal, VmallocUsed, VmallocChunk 字段
- [ ] 添加 Mlocked 字段（mlock 相关测试需要）

### 1.3 /proc/self/status 完善（3+ 测试）

**现状**: 已实现但 `VmLck` 字段缺失。

**待办**:
- [ ] 添加 VmLck 行（当前返回 0 即可）
- [ ] 确保 Uid/Gid 行格式正确（4 个数字 tab 分隔）

### 1.4 /proc/self/fd/N readlink（1+ 测试）

**现状**: `/proc/self/fd/N` 可以打开，但 readlink 返回 ENOENT。

**影响**: openat03, open14 等测试需要通过 readlink 获取 fd 对应的文件路径。

**待办**:
- [ ] 在 procfs 的 fd 子目录中支持 readlink 语义
- [ ] 返回 fd 对应的 File 的 `path()` 方法结果

### 1.5 /proc/sys/fs/ 条目

| 路径 | 影响 |
|------|------|
| /proc/sys/fs/inotify/max_user_instances | inotify06 |
| /proc/sys/fs/inotify/max_user_watches | inotify 相关 |
| /proc/sys/fs/pipe-max-size | pipe 相关 |

---

## 2. /dev 设备文件

### 2.1 /dev/loop* 块设备（22+ 测试）

**现状**: "Failed to acquire device" 影响 lchown03, linkat02, mknod07, mknodat 等。

**说明**: LTP 使用 `tst_acquire_device()` 寻找可用块设备做文件系统测试。没有 loop 设备时会 TBROK。

**建议**: 较复杂，低优先级。可考虑创建一个内存块设备模拟 loop 设备。

### 2.2 /dev/tty（少量测试）

**现状**: 部分 ioctl 测试需要 `/dev/tty`。

**待办**:
- [ ] 确保 `/dev/tty` 存在且可打开
- [ ] 支持基本的 TIOCGWINSZ 等 ioctl

---

## 3. /etc 文件完善

### 3.1 /etc/passwd NSS 问题

**现状**: COW fork 修复后 getpwnam 大部分已工作，但仍有少量失败。

**待办**:
- [ ] 确认所有需要的用户条目都在 /etc/passwd 中（nobody, daemon, bin 等）
- [ ] 检查 glibc NSS 是否在某些场景下仍返回 EFAULT

### 3.2 /etc/protocols 完善

**现状**: 已有基本条目，但 `hopopt` 协议条目格式不对导致 `asapi_01` TFAIL。

**待办**:
- [ ] 检查 /etc/protocols 格式是否完全符合 getprotobyname() 期望
