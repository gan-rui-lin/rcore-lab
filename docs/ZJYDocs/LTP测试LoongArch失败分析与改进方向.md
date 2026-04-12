# LTP 测试 LoongArch 失败分析与改进方向

**日期**：2026/04/13  
**架构**：LoongArch64（qemu-system-loongarch64）  
**测试套件**：LTP 20240524（musl + glibc）

---

## 一、背景

本文档基于 LoongArch 架构下运行 LTP（Linux Test Project）全量测试的输出日志（`LoongArch输出 (1).txt`）进行分析。测试环境为：

```
qemu-system-loongarch64 -kernel kernel-la -m 1G -nographic -smp 1
  -drive file=sdcard-la.img ...
  -device virtio-blk-pci ...
  -device virtio-net-pci ...
```

LTP 测试结果有四种状态：
- **TPASS**：断言通过
- **TFAIL**：断言失败（逻辑错误，内核行为不符预期）
- **TBROK**：测试 setup 失败（依赖条件缺失，导致整个 case 无法运行）
- **TCONF**：测试被跳过（功能未启用，中性）

**总体数据：**

| 状态 | 数量 |
|---|---|
| TPASS | 2675 |
| TFAIL | 788 |
| TBROK | 394 |
| TCONF（跳过） | 919 |
| 超时（exit 124） | 16 个 case |

通过率约 **69.4%**（以 TPASS / (TPASS+TFAIL+TBROK) 计算）。以下按内核层面分类分析所有失败，并给出高优先级改进路径。

---

## 二、失败分析（按内核层面）

### 2.1 信号处理层

#### 2.1.1 `rt_sigaction` 参数校验缺失（150 TFAIL）

这是**单个测试贡献失败数最多**的问题，`rt_sigaction03` 测试共 150 个子 case 全部失败：

```
rt_sigaction03  150  TFAIL  :  rt_sigaction03.c:129:
    rt_sigaction call succeeded: result = 0 got error 22: but expected 22
```

测试逻辑：对无效信号号（如 SIGKILL=9、SIGSTOP=19、超范围的 signum）调用 `rt_sigaction` 时，Linux 规范要求返回 `EINVAL (-22)`。但我们的内核直接返回成功（0），从而 150 个期望失败的子 case 全部判为 TFAIL。

**修复方向**：在 `sys_rt_sigaction` 入口处加 signum 合法性校验：
- `signum <= 0` 或 `signum > 64` → EINVAL
- `signum == SIGKILL (9)` 或 `signum == SIGSTOP (19)` → EINVAL（不可捕获/忽略）

这是性价比最高的单点修复，一行判断可修复 150 个 TFAIL。

#### 2.1.2 `sigwait` 信息字段不匹配（10 TFAIL）

```
sigwait.c:90: TFAIL: struct siginfo mismatch
sigwait.c:29: TFAIL: Expected error number EINTR, got: EINVAL (22)
```

`sigwaitinfo`/`sigtimedwait` 返回的 `siginfo_t` 结构体字段（如 `si_code`、`si_pid`）与 Linux 标准不一致，且某些超时场景返回 EINVAL 而非 EINTR。

---

### 2.2 procfs / VFS 层

#### 2.2.1 缺失 procfs 条目（~30 TBROK）

大量测试在 setup 阶段就因访问 `/proc` 下的虚拟文件失败而 TBROK，无法运行任何子 case：

| 缺失路径 | TBROK 数 | 受影响测试 |
|---|---|---|
| `/proc/sys/kernel/tainted` | 12 | tst_taint（影响所有依赖 tainted 检测的测试） |
| `/proc/sys/vm/overcommit_memory` | 4 | tst_sys_conf |
| `/proc/cmdline` | 2 | tst_kconfig |
| `/proc/self/fd/<N>`（readlink） | 2 | openat03, open14 |
| `/proc/sys/kernel/printk` | 1 | tst_sys_conf |
| `/proc/sysvipc/shm` | 1 | shmget03 |
| `/proc/<pid>/stat` | 3 | msgsnd06, msgrcv05, msgrcv06 |
| `/proc/self/ns/{pid,uts,user}` | 6 | ioctl_ns01~06 |
| `/proc/sys/fs/inotify/max_user_instances` | 1 | inotify06 |
| `/proc/kallsyms` | 1 | kallsyms |

**重点说明：**

- `/proc/self/fd/<N>` 的 `readlink` 失败（返回 ENOENT）说明 procfs 的 `fd` 子目录虽然支持 `open`，但尚未实现 `readlink` 语义（即通过 fd 反查文件路径）。Linux 下 `/proc/self/fd/N` 是指向实际文件路径的符号链接，readlink 应返回对应路径。

- `/proc/<pid>/stat` 缺失说明进程的 stat 文件只对 `self` 有效，对 other pid 未实现。

- `/proc/sys/kernel/tainted` 影响面极大，LTP 的测试框架（`tst_taint.c`）在每个测试前后都会检查内核是否被 taint，缺失此文件导致 12 个 TBROK。

#### 2.2.2 VFS 路径错误码不完整（~30 TFAIL）

Linux 规范要求路径解析在以下情况返回特定错误：
- **ELOOP**：符号链接嵌套超过阈值（通常 40 层）
- **ENAMETOOLONG**：路径或单个分量超过 NAME_MAX/PATH_MAX
- **ENOTDIR**：路径中某个分量不是目录

当前内核对这些情况统一返回 ENOENT，导致：

| 测试 | 具体问题 | TFAIL 数 |
|---|---|---|
| `lstat02` | ELOOP/ENAMETOOLONG/ENOTDIR 应返回但得到 ENOENT；lstat 不应成功时成功 | 8 |
| `readlink03` | 同上 + EINVAL 缺失 | 7 |
| `rmdir02/03` | ELOOP/ENAMETOOLONG/ENOTDIR 缺失；rmdir 不应成功时成功 | 7 |
| `statfs02/03` | ENOTDIR/ENAMETOOLONG 缺失；statfs 不应成功时成功 | 6 |
| `mkdirat02` | ELOOP 应返回但得到 EIO | 2 |

**根本原因**：
1. **ELOOP**：符号链接跟踪时没有计数器，缺少循环检测
2. **ENAMETOOLONG**：路径各分量长度未做 ≤ NAME_MAX（255）检查
3. **ENOTDIR**：路径中间分量若非目录，应在 lookup 时提前返回 ENOTDIR 而非继续查找

---

### 2.3 未实现的系统调用（ENOSYS）

#### 2.3.1 向量 I/O：`preadv` / `preadv2` / `pwritev2`（~16 TFAIL）

```
preadv01.c:55: TFAIL: Preadv(2) failed: ENOSYS (38)
preadv201.c:64: TFAIL: preadv2() failed: ENOSYS (38)
pwritev202.c:85: TFAIL: pwritev2() failed unexpectedly, expected EOPNOTSUPP: ENOSYS (38)
```

`preadv`/`pwritev` 是 POSIX 规定的向量 I/O 接口，`preadv2`/`pwritev2` 是 Linux 扩展版本（支持 RWF_HIPRI 等 flags）。当前完全未实现，导致所有子 case 失败。

#### 2.3.2 `mknod` / `mknodat`（4 TBROK + 7 TFAIL）

```
statx01.c:210: TBROK: mknod() failed: ENOSYS (38)
mknodat01.c: mknodat() returned -1: TEST_ERRNO=ENOSYS
```

`mknod` 用于创建设备文件、FIFO 等特殊文件。缺失此调用不仅导致 mknodat 系列测试直接失败，还导致 statx01 等依赖创建特殊文件的测试 TBROK。

#### 2.3.3 内存锁定：`mlock` / `munlock`（5 TFAIL + 2 TBROK）

```
mlock02.c: mlock() failed: ENOSYS (38)
munlock01.c:34: TBROK: mlock() failed: ENOSYS (38)
mmap14: TFAIL: Expected 1024K locked, get 0K locked
```

`mlock` 阻止内存页被换出，测试中用于验证 `mmap` 的锁定语义。

#### 2.3.4 其他缺失 syscall（按影响排序）

| syscall | 影响 | TFAIL/TBROK |
|---|---|---|
| `sethostname` / `setdomainname` | 5+5 TFAIL | 实现简单，可做 stub |
| `name_to_handle_at` / `open_by_handle_at` | ~27+10 TFAIL | 复杂 FS 接口 |
| `sched_rr_get_interval` | 2 TFAIL | |
| `removexattr` / `setxattr` | 4 TFAIL | xattr 支持 |
| `setgroups` | 1 TBROK | |
| `recvmmsg` | 1 TFAIL | |
| `personality` | 18 TBROK | |
| `remap_file_pages` | 8 TFAIL | 已废弃接口 |

---

### 2.4 prctl 层

`prctl` 系统调用实现不完整，多个 option 返回 EINVAL：

| prctl option | 问题 | TFAIL 数 |
|---|---|---|
| `PR_SET_PDEATHSIG` | 返回 EINVAL，未实现 | 1 |
| `PR_SET_TIMERSLACK` | 返回 EINVAL（所有值包括 0/1/70000 均失败） | 4 |
| `PR_GET_NAME` | 返回空字符串，线程名未持久化 | 2 |
| `PR_SET/GET_SECCOMP` | seccomp 机制未实现 | 4 |

`PR_SET_PDEATHSIG` 设置"父进程死亡时向子进程发送的信号"，是多进程程序的常见需求。`PR_SET_TIMERSLACK` 设置定时器精度裕量，影响 prctl08/09。

---

### 2.5 管道 / splice 层

#### 2.5.1 `fcntl F_GETPIPE_SZ` 返回 EINVAL（3 TBROK）

```
splice02.c:126: TBROK: fcntl(5,F_GETPIPE_SZ,...) failed: EINVAL (22)
pipe12.c:97: TBROK: fcntl(5,F_GETPIPE_SZ,...) failed: EINVAL (22)
splice01.c:64: TBROK: splice(fd_in, pipe) failed: EINVAL (22)
```

`F_GETPIPE_SZ` / `F_SETPIPE_SZ` 是 Linux 特有的 fcntl 命令，用于获取/设置管道缓冲区大小。未实现导致依赖此 feature 的 splice 测试在 setup 阶段就 TBROK。

#### 2.5.2 `pipe2` 返回 fd 不携带标志位（3 TFAIL）

```
pipe2_01.c:66: TFAIL: pipe2 fds[1] doesn't get expected flag(524288), get flag(0)
pipe2_01.c:66: TFAIL: pipe2 fds[0] doesn't get expected flag(2048), get flag(0)
```

`pipe2(fds, O_NONBLOCK)` 应让返回的 fd 带 `O_NONBLOCK`（2048）标志，`pipe2(fds, O_DIRECT)` 应带 `O_DIRECT`（524288）标志。当前 `pipe2` 创建管道后未将 flags 设置到 fd 上。

#### 2.5.3 管道最大数量（1 TFAIL）

```
pipe07.c:78: TFAIL: exp_num_pipes (1024) != num_pipe_fds (1020)
```

期望能创建 1024 个管道，实际只能创建 1020 个，差 4 个。可能是 fd 限制或内部计数问题。

---

### 2.6 内存管理层

| 问题 | TFAIL/TBROK | 分析 |
|---|---|---|
| `mmap06`：无效 prot/flags 不应成功 | 2 TFAIL | mmap 参数校验不足 |
| `mmap15`：高地址映射不该成功 | 1 TFAIL | 用户地址空间上限未检查 |
| `mmap18`：子进程 SIGSEGV | 2 TFAIL | 可能是 COW 或权限位问题 |
| `mmap20`：意外 EINVAL | 1 TFAIL | 合法调用被拒 |
| `mmap17`：EEXIST 而非 ENOENT | 1 TFAIL | 文件映射错误码 |
| `munmap01/02`：写 mmap 文件 errno=0 | 2 TBROK | mmap 写回路径 bug |
| `mmap14`：mlock 锁定量为 0 | 1 TFAIL | 依赖 mlock |

`mem02` 超时说明某些内存压力测试（OOM 场景）在当前内核中无法正常退出，可能是内存回收路径阻塞。

---

### 2.7 SysV IPC 层

所有 System V 信号量接口（`semget`）返回 ENOSYS，导致级联失败：

```
semctl01.c:269: TBROK: semget(0, 10, 780) failed: ENOSYS (38)
pipeio.c: Couldn't allocate semaphore: errno=ENOSYS(38)
sendmsg01.c: ip/ifconfig failed to bring up loop back device
```

- **semaphore 系列**：semget01~09、semop、semctl01~09 全部 TBROK
- **级联影响**：sendmsg02、pipeio 等依赖 semaphore 做进程同步的测试也 TBROK

`shmctl02` 的 EFAULT/EPERM 校验缺失（10 TFAIL）：
- `shmctl(id, IPC_STAT, invalid_ptr)` 不应成功但成功了
- `shmctl(id, IPC_SET, ...)` 无权限时应返回 EPERM 但返回 EINVAL

---

### 2.8 Socket / 网络层

| 问题 | 数量 | 分析 |
|---|---|---|
| `accept4` EFAULT | 1 TBROK | accept4 未验证 sockaddr 指针 |
| `getpeername` 返 ENOTSOCK 而非 EFAULT | 3 TFAIL | fd 类型检查在指针校验之前 |
| `getsockopt`：无效 level/optname 未拒绝 | 6 TFAIL | 参数校验缺失 |
| `setsockopt`：不该成功时成功 | 6 TFAIL | 同上 |
| `send02` EINVAL | 1 TBROK | connected socket 发送失败 |
| `getsockopt02`：listen 返回 ENOTSOCK | 1 TBROK | fd 管理问题 |

**注**：大量网络测试（tcp4-uni-*、tcp6-*）因 `command cut not found` 全部 TCONF（跳过），这是测试环境问题（busybox 缺少 cut 命令），不是内核 bug。

---

### 2.9 定时器精度

```
tst_timer_test.c:292: TFAIL: select() woken up early 418 times range: [999,461]
tst_timer_test.c:292: TFAIL: pselect() woken up early 278 times range: [995,411]
tst_timer_test.c:314: TFAIL: nanosleep() slept for too long
```

`select`/`pselect`/`poll` 大量提前唤醒（每轮超时 1ms 时有 418 次提前唤醒），且 `nanosleep` 睡眠过长。这是由于：
1. QEMU 的时钟模拟精度不足
2. 内核 timer wheel 分辨率可能只有 10ms（CONFIG_HZ=100）

此问题涉及时钟中断频率和高精度定时器（HRTIMER）支持，修复难度较高，直接导致 **16 个 case 超时** + **32 个 TFAIL**。

---

## 三、高优先级改进路径

按**性价比（修复成本 vs 得分提升）**排序：

### 🔴 P0 - 快速高回报（修改量小，分数提升大）

#### 3.1 `rt_sigaction` 参数校验（修复 150 TFAIL）

在 `sys_rt_sigaction` 入口加入：
```rust
if signum <= 0 || signum > 64 || signum == SIGKILL || signum == SIGSTOP {
    return -EINVAL;
}
```

一行判断，修复 150 个 TFAIL，是全部改进中 ROI 最高的。

#### 3.2 procfs 路径补全（修复 ~30 TBROK）

优先级从高到低：
1. `/proc/sys/kernel/tainted` → 返回 `0\n`（未 tainted）
2. `/proc/sys/vm/overcommit_memory` → 返回 `0\n`
3. `/proc/cmdline` → 返回内核命令行参数
4. `/proc/self/fd/<N>` → 实现 readlink，返回 fd 对应的文件路径
5. `/proc/<pid>/stat` → 实现对任意 pid 的 stat 文件

#### 3.3 VFS 路径错误码完善（修复 ~30 TFAIL）

- **ELOOP**：在 path lookup 时增加符号链接跟踪计数器，超过 40 层返回 ELOOP
- **ENAMETOOLONG**：每个路径分量 > 255 字节时返回 ENAMETOOLONG
- **ENOTDIR**：中间分量不是目录时返回 ENOTDIR 而非继续查找

---

### 🟠 P1 - 中等投入，较大回报

#### 3.4 `preadv` / `pwritev2` 实现（修复 ~16 TFAIL）

`preadv(fd, iov, iovcnt, offset)` 语义是"在 offset 处执行向量读"，可在 `pread` + `readv` 基础上组合实现。`preadv2` 增加 flags 参数（RWF_HIPRI/RWF_SYNC 等），初期可忽略 flags 实现基础功能。

#### 3.5 管道层修复（修复 ~6 TFAIL + 3 TBROK）

- `fcntl F_GETPIPE_SZ`：返回管道缓冲区大小（默认 65536）
- `fcntl F_SETPIPE_SZ`：设置管道缓冲区大小（需权限校验）
- `pipe2` flags 传递：创建 fd 后将 `O_NONBLOCK`/`O_DIRECT` 写入 fd flags

#### 3.6 `prctl` 补全（修复 ~7 TFAIL）

- `PR_SET_PDEATHSIG`：在 TCB 中存储 pdeathsig，父进程退出时发送
- `PR_SET/GET_TIMERSLACK`：在 TCB 中存储 timer_slack 值（nanoseconds）
- `PR_SET/GET_NAME`：在 TCB 中存储线程名（最多 16 字节）

#### 3.7 `mknod` / `mknodat` 实现（修复 ~11 TFAIL + TBROK）

- 对于设备文件（S_IFCHR/S_IFBLK），在 VFS 中创建特殊 inode
- FIFO（S_IFIFO）可基于现有 pipe 机制实现

#### 3.8 `mlock` / `munlock` 基础实现（修复 ~7 TFAIL + TBROK）

至少实现 `mlock` 的参数校验和地址范围检查，在 VMA 上打 `VM_LOCKED` 标志，即使没有真正阻止换出也能通过基础测试。

---

### 🟡 P2 - 较高投入，中等回报

#### 3.9 `sethostname` / `setdomainname` stub（修复 ~10 TFAIL）

可在内核中维护一个全局字符串，不需要真正配置网络：
```rust
static HOSTNAME: Mutex<[u8; 64]> = ...;
```

#### 3.10 socket 参数校验（修复 ~12 TFAIL）

- `getsockopt`/`setsockopt`：对无效 level（如非 SOL_SOCKET/IPPROTO_TCP）返回 ENOPROTOOPT
- `accept4`：在 socket 操作前先验证 sockaddr 指针（EFAULT 检查应在 ENOTSOCK 之前）

#### 3.11 `shmctl` 指针校验（修复 ~10 TFAIL）

- `shmctl(id, IPC_STAT, NULL)` 应返回 EFAULT
- `shmctl(id, IPC_SET, ...)` 无 root 权限时应返回 EPERM
- 无效 shmid 应返回 EINVAL

---

## 四、不建议优先投入的方向

以下问题虽然有失败，但修复成本过高或受环境限制：

| 问题 | 原因 |
|---|---|
| 定时器精度（select/poll wakeup early） | 需要改 HRTIMER + QEMU 时钟，影响面广 |
| `personality` syscall | 涉及进程模拟模式，极少使用 |
| `name_to_handle_at` / `open_by_handle_at` | 需要 FS 层大改，测试 case 本身也不多 |
| `remap_file_pages` | Linux 已废弃该接口 |
| ptrace 完整实现 | 极复杂，且与调试器耦合深 |
| SysV semaphore 完整实现 | 涉及 IPC namespace 等，工作量大 |
| tcp 网络测试 | TCONF 是因为 busybox 缺 `cut` 命令，属于用户态环境问题 |

---

## 五、总结

当前内核最集中的问题分布在：

1. **信号校验缺失**：`rt_sigaction` 不拒绝无效 signum → 150 TFAIL（一行修复）
2. **procfs 不完整**：大量 `/proc/sys/...` 路径不存在 → 30 TBROK（级联阻断测试）
3. **VFS 错误码不规范**：ELOOP/ENAMETOOLONG/ENOTDIR 三类错误被吞 → 30 TFAIL
4. **关键 syscall 缺失**：preadv、mknod、mlock 等 → 30+ TFAIL

若完成上述 P0+P1 所有改进，预计可将 TFAIL 从 788 降低到 500 以下，通过率从 69.4% 提升至 80%+。
