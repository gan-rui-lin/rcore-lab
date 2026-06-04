# LTP glibc-rv Round 8 失败分类分析

**日期**: 2026/06/04

**数据源**: `/private/tmp/syscall/ltp-glibc-round8.log`

## 总体概览

| 指标 | 数值 |
|------|------|
| TPASS | 3363 |
| TFAIL | 232 |
| TBROK | 471 |
| 通过率 | 82.7% (3363/4066) |

---

## 一、TBROK 分类 (共 471 条)

### 1.1 getpwnam / getpwuid 失败 — 133 条 (约 98 个独立用例)

**根因**: 内核未实现完整的 `/etc/passwd` 解析或 NSS (Name Service Switch) 支持。`getpwnam("nobody")` 调用返回 `EFAULT (14)` 而非正确的 passwd 结构体。`getpwuid()` 同理。

这是 **影响面最广** 的单一根因，阻塞了近 98 个独立测试用例。许多 LTP 测试在 setup 阶段需要切换到 `nobody` 用户来测试权限相关行为。

**受影响测试用例列表** (去重后 ~98 个):

| 类别 | 用例 |
|------|------|
| 文件权限 | access01-04, chmod03/05/06/07, chown03/04, creat06, fchmod03/04/06, fchown03, link04, mkdir02/05, mknod02, open02/10, readlink01, rmdir03, stat01, statfs03, truncate03, unlink08, utime06, utimes01 |
| 用户/组管理 | setegid01/02, setfsgid01/02, setfsuid01/03/04/04_16, setgid02/03, setgroups03, setregid02/03, setresgid01/01_16/03/04/04_16, setresuid02/04/05, setreuid02-07, setrlimit02, setuid03/04 |
| 进程/调度 | chroot01/04, execve03, fchdir03, nice04, sched_setaffinity01, sched_setparam05, sched_setscheduler02/03, unshare02, vhangup01 |
| IPC/消息队列 | bind02, mq_open01, mq_unlink01, msgctl04/06, semctl02/09, shmctl02/04, shmget04 |
| 其他 | adjtimex02, getegid02, getgid01, getresgid02/03, getresuid02/03, lchown02/02_16, mlock02, mlock202, pathconf02, reboot02, setdomainname03/sethostname03, setpriority02, stime02, syslog12, utsname04 |
| 老式 API (额外) | mknod03/04/05/08 ("nobody not in /etc/passwd"), setfsuid04/04_16, setresgid01/01_16 ("getpwnam() failed for root") |

**修复建议**: 实现最简 `/etc/passwd` + `/etc/group` 文件解析，或在 `getpwnam`/`getpwuid` 系统调用路径中正确处理用户指针传递（当前返回 EFAULT 说明指针校验可能存在问题）。

---

### 1.2 Cannot parse kernel .config — 64 条 (约 64 个独立用例)

**根因**: LTP 框架在 `tst_kconfig.c:207` 尝试读取 `/proc/config.gz` 或 `/boot/config-*`，内核未提供这些文件。

**受影响用例** (64 个):

acct02, aslr01, bind06, cfs_bandwidth01, clock_gettime03, icmp_rate_limit01, init_module01, io_cancel01, io_destroy02, io_getevents01, io_setup02, io_submit02, io_submit03, keyctl09, kill13, ksm01, ksm03, ksm05, ksm07, madvise11, mountns01-04, mqns_01-04, msgget04/05, msgrcv03, netns_netlink, nft02, pidns01-03/12/20, proc_sched_rt01, process_madvise01, pty03, sendmsg03, sendto03, setsockopt05-10, shmget05/06, statx09, swapping01, sysinfo03, tcindex01, timens01, timerfd04, userns02-05/07/08, vsock01

**修复建议**: 实现 `/proc/config.gz` 或在 sdcard 中提供 `/boot/config-*` 文件。也可以考虑提供一个最简的 `.config` 文件来满足 LTP 框架检查。

---

### 1.3 Failed to acquire device — 73 条 (约 72 个独立用例)

**根因**: `tst_device.c:354` 无法创建 loop 设备或块设备。内核缺少 `/dev/loop*` 设备支持或 `losetup` 工具。

**受影响用例** (72 个):

| 类别 | 用例 |
|------|------|
| 文件系统挂载 | mount01-07, mount_setattr01, move_mount01/02, umount01-03, umount2_01/02 |
| 文件操作 | rename01/03-08/10-13, renameat01, lchown03/03_16, linkat02, mkdir09 |
| 设备节点 | mknod07, mknodat02 |
| 交换分区 | swapoff01/02, swapon01-03 |
| 文件属性 | statfs01/01_64, statvfs01, statx04/06/08/10-12, setxattr01, lremovexattr01 |
| I/O | ioctl04-06, msync04, preadv03/03_64, preadv203/03_64, pwritev03/03_64, readahead02, sync01, sync_file_range02, syncfs01 |
| 时间 | utime01-05, utimensat01 |
| 内存 | memcontrol02-04 |
| 其他 | prctl06 |

**修复建议**: 需要实现 loop 设备 (`/dev/loop0` 等) 和相关的 `ioctl`，或在 sdcard 镜像中预创建测试用块设备。

---

### 1.4 getcwd() 失败 (ENOENT) — 27 条 (约 25 个独立用例)

**根因**: `getcwd()` 在某些场景下返回 `ENOENT (2)`。这可能是因为 `/proc/self/cwd` 不可用，或者当前工作目录被删除后 `getcwd` 的实现不正确。

**受影响用例**:

IPC 相关 (依赖 `libnewipc.c`): msgctl01/02, msgget01-03, msgrcv01/02/07/08, msgsnd01/02, semctl03-05/07, semget01/02, shmat04, shmdt01, shmget03

其他: getcwd03, kill05, openat01, unlinkat01

**修复建议**: 检查 `sys_getcwd` 实现，确保在有效工作目录下返回正确路径。当前许多 IPC 测试因 `getcwd` 失败导致 key 生成失败而全部 TBROK。

---

### 1.5 /proc 缺失条目 — 20 条 (约 16 个独立用例)

**根因**: 内核的 procfs 实现不完整，缺少多个 `/proc` 条目。

| 缺失路径 | 涉及用例 |
|----------|----------|
| `/proc/self/ns/pid` | ioctl_ns01 |
| `/proc/self/ns/uts` | ioctl_ns02, ioctl_ns03 |
| `/proc/self/ns/user` | ioctl_ns04, ioctl_ns06 |
| `/proc/self/status` | mlock201, mlock203, munlockall01 |
| `/proc/self/smaps` | mlock05 |
| `/proc/self/maps` | mmap04 |
| `/proc/self/coredump_filter` | madvise08 |
| `/proc/self/fd/3` (readlink) | open14, openat03 |
| `/proc/kallsyms` | kallsyms |
| `/proc/meminfo` | max_map_count, overcommit_memory |
| `/proc/sys/kernel/keys/root_maxkeys` | keyctl02 |
| `/proc/sys/kernel/printk` | syslog (TBROK) |
| `/proc/sys/net/ipv4/conf/default/tag` | clone09 |
| `/proc/sys/vm/min_free_kbytes` | min_free_kbytes |
| `/sys/class/net/lo/mtu` | sctp_big_chunk |

**修复建议**: 按优先级实现 procfs 条目。`/proc/self/status`、`/proc/self/maps`、`/proc/meminfo` 影响较多用例，建议优先实现。

---

### 1.6 clone3 失败 (EINVAL) — 12 条 (11 个独立用例)

**根因**: `clone3` 系统调用返回 `EINVAL`，可能是 `clone_args` 结构体解析不完整或 namespace 相关标志不支持。

**受影响用例**: pidns04, pidns06, pidns10, pidns13, pidns16, pidns17, pidns30, pidns31, pidns32, utsname03, share_ns (common.h)

**修复建议**: 检查 `sys_clone3` 对 `CLONE_NEWPID` 等 namespace 标志的支持。

---

### 1.7 /dev/ptmx 缺失 — 6 条 (5 个独立用例)

**根因**: `/dev/ptmx` 伪终端主设备不存在。

**受影响用例**: hangup01, ptem01, pty01, pty02, pty04, pty05

**修复建议**: 实现 devpts 文件系统或在 sdcard 中创建 `/dev/ptmx` 设备节点。

---

### 1.8 ENOSYS — 直接返回 (11 条)

**根因**: 系统调用本身返回 `ENOSYS`，在 TBROK 检查阶段就失败了。

| 系统调用 | 用例 |
|----------|------|
| `adjtimex` / `clockadjtime` | adjtimex01, adjtimex03 |
| `mq_open` | mq_notify01, mq_notify03, mq_timedreceive01, mq_timedsend01 |
| `semget` | sem_nstest, semctl01, semop04 |
| `semaphore (IPC)` | pipeio, sendmsg02 |

---

### 1.9 SIGSEGV 崩溃 — 12 条 (约 8 个独立用例)

**根因**: 用户程序收到意外的 `SIGSEGV` 信号，表明存在内存访问问题。

| 来源 | 用例 |
|------|------|
| `tst_sig.c` unexpected SIGSEGV | fcntl18, mlockall02, mlockall03, mmap001, remap_file_pages01, vma01 |
| `tst_test.c` Test killed by SIGSEGV | clone09 (child), mq_open02, open09, nftw01 |
| Test killed (timeout) | accept03 |
| Test killed by SIGIOT/SIGABRT | timer_create04 |
| Test killed by SIGTERM | timer_delete02 |

**修复建议**: 需要逐一调查这些 SIGSEGV 的根因。可能涉及信号处理栈设置、mmap 权限、或特定系统调用的内存访问问题。

---

### 1.10 其他 TBROK (杂项)

| 问题 | 条数 | 用例 |
|------|------|------|
| `fcntl(F_GETPIPE_SZ/F_SETPIPE_SZ)` EINVAL | 5 | pipe12, pipe15, pipe2_04, splice02, vmsplice04 |
| `splice()` EINVAL | 3 | splice01, tee01, vmsplice01 |
| `short write` (pipe 写入不完整) | 3 | splice04, splice05, vmsplice03 |
| `pivot_root mkdir EEXIST` | 4 | pivot_root01 (多次运行) |
| `execlp()` failed | 3 | open12, openat02 |
| `readlink(/proc/self/fd/*)` ENOENT | 3 | open14, openat03 |
| `ioctl ENOTTY` | 5 | ioctl07, rtc (tst_wallclock), setxattr03, sockioctl01, unlink09 |
| `ioctl_ns05 ltp_clone` EINVAL | 1 | ioctl_ns05 |
| `pread` 返回成功但不该成功 | 2 | truncate02 |
| `send()` EINVAL | 1 | send02 |
| `shmdt(NULL)` EINVAL | 1 | shmat03 |
| `shmctl(IPC_INFO)` EINVAL | 1 | shmctl01 |
| `prctl(PR_SET_TIMERSLACK)` EINVAL | 1 | prctl09 |
| `msgstress01 msgget` ENOSPC | 1 | msgstress01 |
| `getaddrinfo` 失败 | 1 | getaddrinfo_01 |
| `getuid()` 返回异常 | 1 | getgid03 |
| `Failed to copy resource` | 2 | getrusage03, pipe2_02 |
| `vfork02 SIGUSR1 not pending` | 1 | vfork02 |
| `recv01/recvfrom01 timeout` | 2 | recv01, recvfrom01 |
| `clock_gettime EINVAL` | 1 | nice05 |
| `overcommit_memory != 2` | 1 | oom01 |
| `LTP_IPC_PATH not defined` | 1 | (waitpid test) |
| usage / missing args | 3 | sendfile6_client, sendfile_server, sendfile6_server |
| `ima_mmap` missing filename | 1 | ima_mmap |
| `read_all` missing -d | 1 | read_all |
| `ioctl02` no -d option | 1 | ioctl02 |

---

## 二、TFAIL 分类 (共 232 条, 118 个独立用例)

### 2.1 ENOSYS — 未实现系统调用导致 (27 条, 14 个独立用例)

测试调用了未实现的系统调用，返回 ENOSYS 而非预期行为。

| 未实现调用 | 涉及用例 | TFAIL 条数 |
|-----------|----------|-----------|
| `remap_file_pages` (234) | remap_file_pages01 | 8 |
| `sendmmsg` / `recvmmsg` (269/243) | sendmmsg01, sendmmsg02, recvmmsg01 | 9 |
| `sched_rr_get_interval` (127) | sched_rr_get_interval01/02/03 | 5 |
| `pivot_root` (41) | pivot_root01 | 1 |
| `reboot` (142) | reboot01, reboot02 | 3 |
| `ptrace` | ptrace03, ptrace11 | 2 |
| `mq_notify` (184) | mq_notify02 | 2 |
| `getcpu` (168) | getcpu01 | 1 |

---

### 2.2 错误的 errno 返回 (约 55 条, ~30 个独立用例)

系统调用返回了错误，但 errno 与预期不符。

| 用例 | 预期 errno | 实际 errno | 说明 |
|------|-----------|-----------|------|
| timer_settime02 | EINVAL | EFAULT(14) 或 SUCCESS(0) | 24 条，timer 参数校验不正确 |
| socket01 | EPROTONOSUPPORT(93) | EINVAL(22) | 协议类型错误码不对 |
| send01 | EMSGSIZE(90)/EOPNOTSUPP(95) | EINVAL(22) | UDP 消息过大、无效 flags 错误码不对 |
| sendto01 | EPIPE(32)/EOPNOTSUPP(95) | 0/EINVAL(22) | shutdown 后发送应失败 |
| sockioctl01 | EFAULT(14) | ENOTTY(25) | ioctl 错误码不对 |
| wait403 | ESRCH | ECHILD(10) | wait4 错误码不对 |
| waitpid04 | EINVAL/ESRCH | ECHILD(10) | waitpid 错误码不对 |
| renameat201 | EEXIST/ENOENT | EINVAL(22) | renameat2 flags 不支持 |
| renameat202 | 成功 | EINVAL(22) | renameat2 RENAME_EXCHANGE 不支持 |
| mmap08 | EBADF | EINVAL(22) | mmap 对无效 fd 错误码不对 |
| gethostbyname_r01 | ERANGE(34) | retval=14 | gethostbyname_r 缓冲区检查 |
| linkat01 | EXDEV(18) | EOPNOTSUPP(95) | 跨设备链接 |
| setrlimit01 | errno=10 | errno=26 | setrlimit 错误码 |
| sigwait | EINTR | EINVAL(22) | sigwait 错误码不对 |

---

### 2.3 操作应失败但成功了 (约 41 条, ~25 个独立用例)

系统调用在应该返回错误时意外成功。

| 用例 | 问题描述 |
|------|---------|
| open07 | `O_NOFOLLOW` 打开符号链接应失败 (4条) |
| open11 | 目录以 `O_RDWR`/`O_WRONLY` 打开应失败 (3条) |
| open06 | FIFO 以 `O_NONBLOCK|O_WRONLY` 打开应失败 |
| open13 | `fchmod`/`fchown` 对 `O_PATH` fd 应失败 |
| mmap06 | mmap 非法参数应失败 (2条) |
| mmap15 | mmap 高地址区域应失败 |
| mmap20 | mmap MAP_FIXED_NOREPLACE 应失败 |
| mmap17 | mmap MAP_FIXED_NOREPLACE 应返回 EEXIST |
| memfd_create01 | mmap+mprotect 应拒绝密封违反 |
| munlock02 | munlock 应失败 |
| llseek01 | lseek 超文件限制后 write 应失败 |
| write04 | write 某种情况应失败 |
| read03 | 空管道 read 应阻塞/失败 |
| pread02 | pread 目录 fd 应失败 (2条) |
| sendfile04 | sendfile 映射缓冲区应失败 (4条) |
| clone302 | clone3 extra size 应失败 |
| settimeofday02 | settimeofday 非法值应失败 |
| sigaltstack02 | 非法 flag / 过小栈应失败 (2条) |
| sched_setattr01 | sched_setattr 非法参数应失败 (3条) |
| sched_getattr02 | sched_getattr 非法参数应失败 (2条) |
| setpgid01 | setpgid 应失败 |
| setsid01 | setsid 已是 leader 应失败 (2条) |
| setgroups04 | setgroups 应失败 |
| setrlimit03 | setrlimit 超过 nr_open 应失败 |
| rename09 | rename 某种条件应失败 |
| shmt09 | sbrk 应失败 |

---

### 2.4 prctl 相关问题 (16 条, 4 个独立用例)

| 用例 | 问题 |
|------|------|
| prctl01 | `PR_GET_PDEATHSIG` 返回 EINVAL |
| prctl04 | `PR_SET_SECCOMP` STRICT/FILTER 模式返回 EINVAL/EACCES (8条) |
| prctl05 | `PR_GET_NAME`/`PR_SET_NAME` 不工作 (2条) |
| prctl08 | `PR_SET_TIMERSLACK` / `PR_GET_TIMERSLACK` 返回 EINVAL (5条) |

**修复建议**: 需要实现 `PR_GET_PDEATHSIG`、`PR_SET_NAME`/`PR_GET_NAME`、`PR_SET_TIMERSLACK`、`PR_SET_SECCOMP` 等 prctl 子命令。

---

### 2.5 信号处理问题 (10 条, 7 个独立用例)

| 用例 | 问题 |
|------|------|
| sigpending02 | 初始化后不应有 pending 信号 |
| sigprocmask01 | `sigismember()` 失败 |
| sigsuspend01 | `sigsuspend()` 未解除 SIGALRM 阻塞 |
| rt_sigprocmask01 | `sigismember()` 调用失败 |
| rt_sigqueueinfo01 | rt_sigqueueinfo 返回 ESRCH |
| signalfd4_01 | signalfd4 SFD_CLOEXEC 未设置 close-on-exec 标志 |
| madvise07 | 未收到 SIGBUS (访问 poisoned page) |
| mmap13 | 未收到 SIGBUS |
| mmap18 | 子进程被 SIGSEGV 杀死 (2条) |
| kill02 | 进程未收到信号 |

---

### 2.6 内存管理问题 (约 19 条, ~12 个独立用例)

| 用例 | 问题 |
|------|------|
| madvise03 | 匿名映射无 zero-fill-on-demand 页 |
| madvise10 | MADV_FREE 后内存未被释放 (2条) |
| mmap01 | getcwd 在 mmap 测试中失败 |
| mmap12 | `/proc/self/pagemap` 打开失败 |
| mmap14 | mlock 后检查锁定页数为 0 |
| shmat03 | 共享内存映射到了低 64KB 区域 |
| shmctl03 | `shmctl(IPC_INFO)` 返回 EINVAL |
| shmctl07 | `SHM_LOCK`/`SHM_UNLOCK` 返回 EINVAL (3条) |
| shmctl08 | `shm_perm.mode` 值异常 (4条) |
| lseek02 | lseek 对管道/FIFO 应失败但成功 (6条) |

---

### 2.7 timer_settime 问题 (24 条, 1 个独立用例)

`timer_settime02` 产生了 24 条 TFAIL，是单个用例中失败最多的。

- 8 条: 预期 EINVAL 但得到 EFAULT(14) — 说明参数指针校验优先于参数值校验
- 16 条: 预期 EINVAL 但得到 SUCCESS(0) — 说明非法的 timer 参数未被正确拒绝

**修复建议**: 修复 `timer_settime` 的参数校验逻辑，应在指针校验之后检查时间值的合法性。

---

### 2.8 调度器问题 (13 条, 7 个独立用例)

| 用例 | 问题 |
|------|------|
| sched_getattr01 | 调度属性读回不正确 |
| sched_getattr02 | 非法参数应失败 (2条) |
| sched_setattr01 | 非法参数应失败 (3条) |
| sched_setparam03 | 优先级设置后读回为 0 (2条) |
| sched_rr_get_interval01-03 | ENOSYS (5条) |

---

### 2.9 文件系统语义问题 (约 15 条)

| 用例 | 问题 |
|------|------|
| fstat02 | `st_nlink` 不正确 (1 vs 2) |
| link02 | link 后 stat 链接计数不匹配 |
| select01 | FIFO select 返回 1 而非 2 (2条) |
| readlinkat01 | readlinkat 失败 / 返回空 (2条) |
| realpath01 | realpath(".") 预期 ENOENT 但成功 |
| utime07 | 符号链接时间戳未更新 (4条) |
| times03 | tms_utime/tms_stime/tms_cutime/tms_cstime 值异常 (5条) |
| writev02 | writev 过量写入 |

---

### 2.10 网络/IPC 问题 (约 16 条)

| 用例 | 问题 |
|------|------|
| asapi_01 | IPv6 protocols 条目缺失 (9条) |
| in6_02 | if_nametoindex(lo) 返回 0、if_nameindex 失败 (3条) |
| socket01 | 协议错误码不对 (4条) |
| send01/send02/sendto01 | 发送语义不正确 |
| setgroups02 | setgroups 后读回不匹配 |
| setresgid02 | setresgid 权限检查过严 (3条) |

---

### 2.11 其他 TFAIL

| 用例 | 问题 |
|------|------|
| clone301 | clone3 返回 EINVAL (5条) |
| fcntl18 | 子进程异常退出 (2条) |
| pipe07 | 管道数量不匹配 (1024 vs 1020) |
| setrlimit04 | 子进程异常退出 |
| setxattr02 | setxattr 在 fifo/chr/blk/sock 上不应成功 (4条) |
| semctl06 | semget 失败导致测试失败 (7条) |

---

## 三、未实现系统调用汇总

从内核日志中提取的 `unimplemented syscall` 消息，按**命中频次**排序（去重后涉及 **108 个独立测试用例**）:

| 系统调用号 | 名称 | 命中次数 | 涉及用例数 |
|-----------|------|---------|-----------|
| 280 | `bpf` | 25 | ~9 (bpf_map01, bpf_prog01-07, accept03, splice07) |
| 434 | `pidfd_open` | 23 | ~8 (pidfd_open01-04, pidfd_getfd01/02, accept03, splice07) |
| 425 | `io_uring_setup` | 19 | ~4 (io_uring01/02, accept03, splice07) |
| 282 | `userfaultfd` | 18 | ~3 (userfaultfd01, accept03, splice07) |
| 433 | `fspick` | 17 | ~2 (accept03, splice07 — 探测型调用) |
| 430 | `fsopen` | 17 | ~2 |
| 428 | `open_tree` | 17 | ~2 |
| 262 | `fanotify_init` | 17 | ~2 |
| 241 | `perf_event_open` | 17 | ~2 |
| 181 | `mq_unlink` (POSIX MQ) | 17 | ~6 (mq_notify01, mq_open01, mq_timedreceive01, mq_timedsend01, pidns30/31) |
| 272 | `kcmp` | 10 | 3 (kcmp01/02/03) |
| 234 | `remap_file_pages` | 10 | 3 (remap_file_pages01/02, shmctl05) |
| 190 | `semget` | 9 | 7 (sem_nstest, semctl01/06, semop04/05, pipeio, sendmsg02) |
| 219 | `keyctl` | 8 | 7 (keyctl01/04/05/08, request_key03/04, wqueue09) |
| 118 | `sched_setparam` | 8 | 4 (sched_setparam01-04) |
| 269 | `sendmmsg` | 7 | 3 (sendmmsg01/02, recvmmsg01) |
| 217 | `add_key` | 7 | 6 (add_key01-04, keyctl03/06, request_key01/02) |
| 127 | `sched_rr_get_interval` | 7 | 3 (sched_rr_get_interval01-03) |
| 192 | `semtimedop` | 6 | 3 (semop01-03) |
| 439 | `faccessat2` | 5 | 2 (faccessat01/02) |
| 266 | `clockadjtime` | 5 | 5 (adjtimex01-03, clock_adjtime01/02) |
| 243 | `recvmmsg` (时间版) | 5 | 1 (recvmmsg01) |
| 180 | `mq_open` (POSIX MQ) | 4 | 3 (mq_notify01, mq_timedreceive01, mq_timedsend01) |
| 437 | `openat2` | 3 | 3 (openat201-203) |
| 424 | `pidfd_send_signal` | 3 | 3 (pidfd_send_signal01-03) |
| 142 | `reboot` | 3 | 2 (reboot01/02) |
| 184 | `mq_notify` (POSIX MQ) | 2 | 1 (mq_notify02) |
| 271 | `process_vm_writev` | 2 | 2 (process_vm01, process_vm_writev02) |
| 270 | `process_vm_readv` | 2 | 2 (process_vm_readv02/03) |
| 268 | `setns` | 2 | 2 (setns01/02) |
| 218 | `request_key` | 2 | 2 (keyctl07, request_key05) |
| 126 | `sched_get_priority_min` | 2 | 2 (sched_get_priority_min01/02) |
| 125 | `sched_get_priority_max` | 2 | 2 (sched_get_priority_max01/02) |
| 84 | `sync_file_range` | 2 | 1 (sync_file_range01) |
| 31 | `ioprio_get` | 2 | 2 (ioprio_get01, ioprio_set01) |
| 30 | `ioprio_set` | 2 | 2 (ioprio_set02/03) |
| 289 | `pkey_mprotect` | 1 | 1 (pkey01) |
| 259 | `riscv_flush_icache`? | 1 | 1 (mprotect04) |
| 213 | `readahead` | 1 | 1 (readahead01) |
| 191 | `semctl` | 1 | 1 (pipeio) |
| 170 | `stime` (set_time?) | 1 | 1 (stime01) |
| 168 | `getcpu` (旧版) | 1 | 1 (getcpu01) |
| 89 | `acct` | 1 | 1 (acct01) |
| 58 | `vhangup` | 1 | 1 (vhangup02) |
| 41 | `pivot_root` | 1 | 1 (pivot_root01) |
| 18446744073709551615 | unknown (-1) | 17 | ~2 (accept03, splice07 — 可能是探测性调用) |

---

## 四、修复优先级建议

按 **影响用例数** 和 **实现难度** 综合排序:

### P0 — 高收益低成本

| 改进项 | 影响用例数 | 说明 |
|--------|-----------|------|
| 修复 `getpwnam`/`getpwuid` (EFAULT→正确解析) | ~98 | 添加 `/etc/passwd` + `/etc/group` 解析支持；这是最大的单点阻塞 |
| 修复 `getcwd` ENOENT | ~25 | 检查 `sys_getcwd` 实现；解除 IPC 测试套件阻塞 |
| 提供 `/proc/config.gz` 或 `.config` | ~64 | 可以提供最简配置文件绕过 |

### P1 — 中等收益

| 改进项 | 影响用例数 | 说明 |
|--------|-----------|------|
| 实现 loop 设备 | ~72 | 挂载、设备、文件系统测试全依赖 |
| 补全 `/proc/self/status`, `/proc/meminfo`, `/proc/self/maps` | ~10 | 多个内存管理测试依赖 |
| 修复 `timer_settime` 参数校验 | 1 (24条) | 单用例产出大量 TFAIL |
| 修复 `open` 的 `O_NOFOLLOW` / 目录 WRONLY 语义 | ~3 | 文件语义正确性 |
| 实现 `prctl` 子命令 (PR_GET_NAME, PR_SET_TIMERSLACK, PR_GET_PDEATHSIG) | ~4 | |
| 修复 `clone3` namespace 支持 | ~11 | pidns 系列依赖 |

### P2 — 低优先级 (高级特性)

| 改进项 | 影响用例数 | 说明 |
|--------|-----------|------|
| 实现 System V 信号量 (semget/semctl/semop) | ~10 | IPC 子系统 |
| 实现 POSIX 消息队列 (mq_open/mq_unlink/mq_notify) | ~6 | IPC 子系统 |
| 实现 sendmmsg/recvmmsg | ~4 | 网络子系统 |
| 实现 bpf / pidfd_open / io_uring | ~20 | 高级 Linux 特性，对教学内核不必要 |
| 实现 /dev/ptmx (伪终端) | ~5 | 终端子系统 |
| 实现 renameat2 (RENAME_EXCHANGE/RENAME_NOREPLACE) | ~2 | 文件系统 |

---

## 五、总结

当前 703 条失败 (TBROK+TFAIL) 中:

- **~40%** (约 280 条) 由 **getpwnam/getpwuid EFAULT** 和 **kernel .config 缺失** 两个基础设施问题造成
- **~15%** (约 105 条) 由 **设备获取失败** 和 **getcwd ENOENT** 造成
- **~10%** (约 70 条) 由 **未实现系统调用直接返回 ENOSYS** 造成
- **~35%** (约 248 条) 由 **语义不匹配**、**错误码不对**、**信号处理问题** 等具体实现缺陷造成

修复 P0 类问题 (getpwnam + getcwd + .config) 可以一次性解除约 **187 个用例** 的阻塞，将有效测试覆盖率从 82.7% 提升至约 **87-90%**。
