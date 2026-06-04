# LTP Skip List 分析与优化

**日期**: 2026/06/04  
**位置**: `user/src/bin/initcode.rs` 中的 `is_skip_case()` 函数（musl ~L889, glibc ~L835）  
**说明**: 两份 skip list 内容完全一致，修改时需同步更新。

---

## 一、Skip List 完整模式提取

### 1.1 never_run 组（helper/非独立测试条目）

这些是 LTP 框架的辅助工具或库文件，不是独立测试用例，永远不应直接运行。

| 模式 | 说明 |
|------|------|
| `check_icmpv4_connectivity` | 网络连通性检查工具 |
| `check_icmpv6_connectivity` | IPv6 连通性检查工具 |
| `cpuacct_task` | cgroup 辅助程序 |
| `create_datafile` | 测试数据生成工具 |
| `create_file` | 文件创建辅助工具 |
| `data` | 测试数据目录/文件 |
| `dirty` | 脏页辅助程序 |
| `growfiles` | 文件增长测试辅助 |
| `kernbench` | 内核编译基准测试（需完整工具链） |
| `libcgroup_freezer` | cgroup freezer 库 |
| `locktests` | 锁测试辅助（NFS 相关） |
| `ltpServer` | LTP 网络测试服务端 |
| `mc_member_test` | 组播成员测试辅助 |
| `mc_recv` / `mc_send` | 组播收发辅助 |
| `mc_verify_opts` / `mc_verify_opts_error` | 组播选项验证辅助 |
| `mmap-corruption01` / `mmap2` | mmap 辅助/变体 |
| `mmstress_dummy` | 内存压力测试占位符 |
| `nfs01_open_files` / `nfs04_create_file` / `nfs_flock` / `nfs_flock_dgen` | NFS 辅助（无 NFS 支持） |
| `ns-echoclient` / `ns-tcpclient` | 网络命名空间客户端辅助 |
| `openfile` | 文件打开辅助工具 |
| `pm_*.py` | 电源管理 Python 脚本 |
| `print_caps` | capability 打印辅助 |
| `rwtest` | 读写测试辅助框架 |
| `sched_tc2` ~ `sched_tc5` | 调度测试辅助子程序 |
| `stress` | 通用压力测试辅助 |
| `test_ioctl` / `testsf_c` | ioctl/socket 辅助 |

**总计**: ~38 个固定模式

### 1.2 基础设施过滤模式（非测试条目）

| 模式 | 匹配范围 | 说明 |
|------|----------|------|
| `*.sh` | ~600+ 个 shell 脚本 | LTP shell wrapper，需 bash/ltp 运行框架支持 |
| `*_helper` / `*_helper.sh` | ~20+ | 辅助程序 |
| `*_child` | ~10+ | 子进程辅助程序 |
| `busy_poll_lib.sh` | 1 | 网络 busy poll 库 |
| `tst_*` / `tst_*.sh` | ~30+ | LTP 测试框架库函数 |
| `event_generator` | 1 | 事件生成器辅助 |
| `find_portbundle` | 1 | 端口查找辅助 |
| `fw_load` | 1 | 固件加载辅助 |

**总计**: ~660+ 个匹配（主要是 `*.sh`）

### 1.3 unimplemented_feature 组（依赖未实现的内核特性）

#### cgroup / cpuset / 资源控制器
| 模式 | 测试数量（源码估算） | 所需特性 |
|------|---------------------|----------|
| `cgroup_fj_proc` / `cgroup_fj_*` | ~10 | cgroup v1 freezer |
| `cgroup_regression_*` | ~5 | cgroup 回归测试 |
| `cpuctl_fj_*` / `cpuctl_*` | ~10 | CPU 控制器 |
| `cpuhotplug_do_*` / `cpuhotplug_report_*` | ~6 | CPU 热插拔 |
| `cpuset*` | ~20 | cpuset 控制器 |
| `cpu_controller*` / `memctl_*` / `memory_controller*` | ~15 | 资源控制器 |
| `memcg*` | ~10 | 内存 cgroup |
| `pids_task*` | ~3 | pids cgroup |

#### 内核模块
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `finit_module*` | 3 | 模块加载 |
| `delete_module*` | 5 | 模块卸载 |
| `crypto_user*` | ~2 | crypto API |

#### 文件系统高级 API
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `fsconfig*` | 3 | fsconfig(2) - 新 mount API |
| `fsmount*` | 2 | fsmount(2) - 新 mount API |
| `fsopen*` | 2 | fsopen(2) - 新 mount API |
| `fspick*` | 2 | fspick(2) - 新 mount API |
| `open_tree01` / `open_tree02` | 2 | open_tree(2) - 新 mount API |

#### 扩展属性（xattr）
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `fgetxattr*` | 3 | xattr 读取 |
| `flistxattr*` | 3 | xattr 列举 |
| `fsetxattr*` | 2 | xattr 设置 |
| `fremovexattr*` | 2 | xattr 删除 |
| `getxattr0[2-4]` | 3 | xattr 读取 |

#### fanotify / inotify（文件系统通知）
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `fanotify*` | 24 | fanotify(7) - 文件访问通知 |
| `inotify*` | 12 | inotify(7) - 已有桩实现，但只返回 fd |

#### 网络相关
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `fanout*` | ~3 | PACKET_FANOUT |
| `tcp4-*` / `tcp6-*` / `udp4-*` / `udp6-*` | ~20 | 完整网络栈测试 |
| `recvmsg01` | 1 | recvmsg 高级功能 |

#### 安全/CVE 相关
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `cve-*` | ~5 | 特定 CVE 复现 |
| `dirtyc0w*` / `dirtypipe` | ~3 | CVE 复现 |
| `capset04` | 1 | capability 高级操作 |
| `exec_with_inh` / `exec_without_inh` | 2 | capability 继承 |

#### ptrace / 调试
| 模式 | 测试数量 | 所需特性 |
|------|----------|----------|
| `ptrace02` / `ptrace06` | 2 | ptrace 高级功能 |

**本组总计**: ~180+ 个测试二进制

### 1.4 known_hang 组（会导致挂起/死锁）

| 模式 | 测试数量 | 原因 |
|------|----------|------|
| `nanosleep01` / `nanosleep04` | 2 | tst_timer_test 框架循环采样，耗时极长 |
| `clock_nanosleep*` | 4 | 同上 + TIMER_ABSTIME 未实现（flags!=0 返回 EINVAL） |
| `pause*` | ~3 | pause() 无信号唤醒则永久挂起 |
| `ppoll*` | 1 | ppoll 超时/信号交互可能挂起 |
| `pselect01` / `pselect01_64` / `pselect02*` | ~4 | pselect 超时/信号交互 |
| `select02` / `select04*` | ~3 | select 超时场景 |
| `poll02` | 1 | poll 超时场景 |
| `pipe13` | 1 | 管道阻塞死锁 |
| `fork_exec_loop` | 1 | 无限 fork/exec 循环 |
| `fsstress` | 1 | 文件系统压力测试无限循环 |
| `fsx-linux` | 1 | 文件系统随机操作压力测试 |
| `ebizzy` | 1 | 内存分配压力测试 |
| `hackbench` | 1 | 调度器基准测试 |
| `mallocstress` | 1 | malloc 压力测试 |
| `starvation*` | ~2 | 饥饿测试 |
| `timed_forkbomb*` | ~2 | fork 炸弹 |
| `nptl01` | 1 | POSIX 线程压力测试 |
| `lftest` | 1 | 长文件名/长路径测试（耗时） |
| `pthcli` / `pthserv` | 2 | 多线程网络客户端/服务端 |
| `leapsec01` | 1 | 闰秒测试（需等待较长时间） |
| `timerfd01` | 1 | timerfd 计时精度测试（循环采样，耗时） |
| `timer_settime03` | 1 | POSIX timer 超时精度测试 |

**本组总计**: ~35 个测试二进制

### 1.5 known_crash 组（会导致内核异常/panic）

| 模式 | 测试数量 | 原因 |
|------|----------|------|
| `crash*` | ~3 | 故意触发崩溃的测试 |
| `f00f` | 1 | 故意触发 CPU 异常（x86 F00F bug 复现） |
| `dio_read` / `dio_sparse` / `doio` | 3 | 直接 I/O 未支持 |
| `diotest2` / `dio_append` / `dio_truncate` | 3 | 直接 I/O 未支持 |
| `dma_thread_diotest` | 1 | DMA + DIO 线程测试 |
| `float_*` / `fptest*` | ~8 | 浮点异常测试，可能触发 IllegalInstruction |
| `mmap1` / `mmap3` / `mmapstress*` / `mmstress` | ~6 | 内存映射压力测试 |
| `mtest*` | ~3 | 内存测试（可能 OOM） |
| `ftest01` ~ `ftest08` | 8 | 文件测试（多进程竞争文件操作） |
| `thp01` | 1 | 透明大页（未实现） |
| `endian_switch01` | 1 | 字节序切换（RISC-V 不支持） |
| `eject_check_tray` | 1 | 光驱弹出（无设备） |
| `inode01` / `inode02` | 2 | inode 压力测试 |
| `shmat1` / `shm_test*` | ~4 | 共享内存高级测试 |

**本组总计**: ~45 个测试二进制

### 1.6 已知失败但不会 crash 的测试

| 模式 | 测试数量 | 原因 |
|------|----------|------|
| `fcntl01` ~ `fcntl39`（大量） | ~50（含 _64） | fcntl 高级功能：file lease / signal / advisory lock / F_SETOWN 等 |
| `fdatasync02` / `fdatasync03` | 2 | fdatasync 错误路径测试 |
| `fchmod02` / `fchmod05` | 2 | fchmod 权限检查 |
| `fchown*_16` / `fchown04` / `fchownat02` | ~8 | 16-bit UID 接口 + 权限检查 |
| `flock01` ~ `flock04` | 4 | flock 跨进程测试 |
| `fork05` / `fork07` / `fork09` / `fork13` / `fork14` | 5 | fork 边界条件测试 |
| `ftruncate01` / `ftruncate01_64` / `ftruncate04` / `ftruncate04_64` | 4 | ftruncate 错误路径 |
| `fstatfs01` / `fstatfs01_64` | 2 | fstatfs 返回值检查 |
| `futex_cmp_requeue*` | ~3 | futex 高级操作 |
| `futex_wait03` / `futex_wait05` / `futex_wait_bitset*` / `futex_waitv*` / `futex_wake02` / `futex_wake04` | ~8 | futex 高级操作 |
| `futimesat01` | 1 | futimesat 时间设置 |
| `fallocate02` / `fallocate04` ~ `fallocate06` | 4 | fallocate 高级模式 |
| `faccessat201` / `faccessat202` | 2 | faccessat2 测试 |
| `creat04` / `creat05` / `creat07` ~ `creat09` | 5 | creat 权限/边界测试 |
| `execve02` / `execve04` / `execve05` | 3 | execve 错误路径 |
| `execveat*` | 5 | execveat 系统调用 |
| `dup05` | 1 | dup 边界测试 |
| `kill08` / `kill10` / `kill11` | 3 | kill 信号边界测试 |
| `signal01*` | ~1 | signal 基本测试 |
| `tgkill*` | 3 | tgkill 线程信号 |
| `waitpid08*` | ~3 | waitpid 边界条件 |
| `readlink03` | 1 | readlink 错误路径 |
| `setpgid03` | 1 | setpgid 错误路径 |
| `setrlimit06` | 1 | setrlimit 错误路径 |
| `setfsgid03*` | ~2 | setfsgid 16-bit UID |
| `sendfile07*` | ~2 | sendfile 大文件偏移 |
| `chdir01` | 1 | chdir 基本测试 |
| `clock_settime03` | 1 | clock_settime 权限测试 |
| `gettimeofday02` | 1 | gettimeofday 错误路径 |
| `link05` / `link08` | 2 | link 权限/跨设备 |
| `lstat02` / `lstat02_64` | 2 | lstat 错误路径 |
| `madvise01` / `madvise02` | 2 | madvise 高级行为 |
| `mincore04` | 1 | mincore 准确性 |
| `mkdir03` / `mkdirat02` | 2 | mkdir 权限检查 |
| `msgrcv05` / `msgrcv06` / `msgsnd05` / `msgsnd06` | 4 | System V 消息队列 |
| `sem_comm` / `semtest_2ns` | 2 | System V 信号量 |
| `open04` / `openat04` | 2 | open 高级 flag |
| `stat03` / `stat03_64` | 2 | stat 错误路径 |
| `sched_datafile*` | ~2 | 调度数据文件辅助 |
| `pidns05` | 1 | PID namespace |
| `prot_hsymlinks` | 1 | 符号链接保护 |
| `io_control*` | ~3 | io_uring 控制 |
| `clone04` | 1 | clone 高级 flag |
| `timerfd_settime02` | 1 | timerfd 高级测试 |
| `frag` | 1 | 碎片化测试 |
| `fsync*` | 4 | fsync 错误路径 + 同步检查 |
| `nftw01` / `nftw6401` | 2 | 目录遍历测试 |

**本组总计**: ~160+ 个测试二进制

---

## 二、Potentially Fixable 分析：新实现的系统调用

以下系统调用近期已实现。分析其 skip 模式是否应该移除。

### 2.1 epoll (`epoll*` 模式) — 建议：部分移除

**已实现**: `sys_epoll_create1`, `sys_epoll_ctl`, `sys_epoll_pwait`  
**实现质量**: 较完整，支持 CLOEXEC、ADD/MOD/DEL、timeout、多事件返回  
**LTP 测试二进制** (源码估算):
- `epoll_create01`, `epoll_create02` (2)
- `epoll_create1_01`, `epoll_create1_02` (2)
- `epoll_ctl01` ~ `epoll_ctl05` (5)
- `epoll_wait01` ~ `epoll_wait07` (7)
- `epoll_pwait01` ~ `epoll_pwait05` (5)
- **总计约 21 个测试二进制**

**建议**: 将 `epoll*` 替换为精确列表，先尝试启用基础用例：
```
# 可尝试启用：
epoll_create01, epoll_create02, epoll_create1_01, epoll_create1_02
epoll_ctl01, epoll_ctl02
epoll_wait01, epoll_wait02, epoll_wait03
epoll_pwait01

# 保持 skip（可能涉及 EPOLLONESHOT/EPOLLET 等高级语义）：
epoll_ctl03~05, epoll_wait04~07, epoll_pwait02~05
```

**风险**: 中等。epoll_wait 的 edge-trigger / oneshot 语义如果未完整实现可能失败但不会 crash。

### 2.2 eventfd (`eventfd*` 模式) — 建议：部分移除

**已实现**: `sys_eventfd2`，支持 `EFD_CLOEXEC` / `EFD_NONBLOCK` / `EFD_SEMAPHORE`  
**LTP 测试二进制**:
- `eventfd01` ~ `eventfd06` (6)
- `eventfd2_01` ~ `eventfd2_03` (3)
- **总计约 9 个测试二进制**

**建议**: 将 `eventfd*` 替换为精确列表，尝试启用：
```
# 可尝试启用：
eventfd01, eventfd2_01, eventfd2_02

# 保持 skip（涉及 fork + eventfd 交互、semaphore 模式深度测试）：
eventfd02~06, eventfd2_03
```

**风险**: 低-中。eventfd read/write 语义需要完整实现（计数器语义、阻塞行为）。

### 2.3 mremap (`mremap*` 模式) — 建议：谨慎启用

**已实现**: `sys_mremap`，支持 `MREMAP_MAYMOVE`  
**实现细节**: 缩小时 no-op 返回原地址；扩大时分配新区域 + memcpy + 取消旧映射  
**LTP 测试二进制**: `mremap01` ~ `mremap06` (6)

**建议**: 尝试启用 `mremap01`，其余保持 skip：
```
# 可尝试启用（基本功能测试）：
mremap01

# 保持 skip（涉及 MREMAP_FIXED、MREMAP_DONTUNMAP 等高级 flag）：
mremap02~06
```

**风险**: 中-高。mremap 的内存语义复杂，错误实现可能导致 StorePageFault。

### 2.4 copy_file_range (`copy_file_range*` 模式) — 建议：暂不移除

**已实现**: `sys_copy_file_range`  
**实现缺陷**: **当前实现只读不写！** 读取源文件内容到 buf 后未写入目标文件，直接返回 bytes_read。这是一个 bug。  
**LTP 测试二进制**: `copy_file_range01` ~ `copy_file_range03` (3)

**建议**: **先修复 bug，再启用测试**。需要在读取后添加 `dst_file.write_at_kernel()` 调用并更新 `_off_out`。

### 2.5 signalfd4 (`signalfd*` 不在 skip list) — 无需操作

**已实现**: `sys_signalfd4`，支持创建和更新  
**状态**: `signalfd*` 模式 **不在** skip list 中，已经可以运行。

### 2.6 POSIX timers (timer_create/settime/gettime/getoverrun/delete) — 部分已启用

**已实现**: 完整的 POSIX timer 五件套  
**Skip list 中仅有**: `timer_settime03`（精度测试，可能因采样循环耗时过长）  
**其他测试**: `timer_create01`~`03`, `timer_delete01`~`02`, `timer_getoverrun01`, `timer_gettime01`, `timer_settime01`~`02` 均**未被 skip**，已经可以运行。

**建议**: 保持 `timer_settime03` skip（已知为耗时精度测试）。

### 2.7 timerfd — 部分已启用

**已实现**: `sys_timerfd_create`, `sys_timerfd_settime`, `sys_timerfd_gettime`  
**Skip list 中**: `timerfd01`, `timerfd_settime02`  
**未被 skip**: `timerfd02`, `timerfd04`, `timerfd_create01`, `timerfd_gettime01`, `timerfd_settime01`（5 个已可运行）

**建议**: 保持当前 skip。`timerfd01` 是精度采样循环测试，`timerfd_settime02` 涉及高级 timer 语义。

### 2.8 clock_nanosleep (`clock_nanosleep*` 模式) — 建议：暂不移除

**已实现**: `sys_clock_nanosleep`  
**实现限制**: `flags != 0` 返回 `EINVAL`，即 **不支持 TIMER_ABSTIME**  
**LTP 测试**: 4 个测试，其中 `clock_nanosleep01` 测试错误路径（含 TIMER_ABSTIME），`clock_nanosleep02` 是精度循环采样  

**建议**: 暂不移除。TIMER_ABSTIME 不支持会导致测试失败，精度测试会挂起。

### 2.9 inotify (`inotify*` 模式) — 建议：暂不移除

**已实现**: 仅桩实现 — `inotify_init1` 返回 devnull fd，`add_watch` 返回 1，`rm_watch` 返回 0  
**LTP 测试**: 12 个测试（inotify01 ~ inotify12）  

**建议**: 桩实现无法通过任何真正的 inotify 测试，保持全部 skip。

### 2.10 close_range (`close_range01` 仅) — 建议：移除

**已实现**: 完整实现，支持 `CLOSE_RANGE_CLOEXEC` 和普通关闭  
**Skip list 中仅有**: `close_range01`  
**LTP 测试**: `close_range01`, `close_range02`

**建议**: 移除 `close_range01`，两个测试都应该能运行。`close_range02` 已经不在 skip list 中。

---

## 三、优化建议汇总

### 3.1 立即可移除的 skip 模式

| 模式 | 预计可运行测试数 | 理由 |
|------|-----------------|------|
| `close_range01` | 1 | 完整实现，应能通过 |

### 3.2 替换通配符为精确列表（逐步启用）

| 原模式 | 建议启用 | 预计新增 | 保持 skip |
|--------|---------|---------|-----------|
| `epoll*` | `epoll_create01`, `epoll_create02`, `epoll_create1_01`, `epoll_create1_02`, `epoll_ctl01`, `epoll_ctl02`, `epoll_wait01`, `epoll_pwait01` | 8 | 其余 13 个 |
| `eventfd*` | `eventfd01`, `eventfd2_01`, `eventfd2_02` | 3 | 其余 6 个 |
| `mremap*` | `mremap01` | 1 | 其余 5 个 |

### 3.3 需先修 bug 再启用

| 模式 | Bug 描述 | 修复后可启用 |
|------|---------|-------------|
| `copy_file_range*` | `sys_copy_file_range` 只读不写，未将数据写入 dst_file | `copy_file_range01` (1个) |

### 3.4 需实现新特性后启用

| 模式 | 所需特性 | 测试数量 | 优先级 |
|------|---------|---------|--------|
| `clock_nanosleep*` | TIMER_ABSTIME 支持 | 2 (排除精度测试) | 中 |
| `inotify*` | 真正的 inotify 实现 | 12 | 低 |
| `fgetxattr*` / `fsetxattr*` 等 | xattr 完整实现 | 13 | 低 |
| `fanotify*` | fanotify 实现 | 24 | 低 |
| 所有 cgroup 相关 | cgroup 子系统 | ~60+ | 极低 |

### 3.5 永久保持 skip

| 类别 | 模式数量 | 理由 |
|------|---------|------|
| never_run (helper/辅助) | ~38 + 660(*.sh等) | 非独立测试用例 |
| crash/压力测试 | ~45 | 故意触发异常或无限循环 |
| 内核模块/设备相关 | ~10 | 不支持模块加载 |
| 网络深度测试 | ~25 | 网络栈不完整 |

---

## 四、推荐的修改方案

### 第一步：移除 `close_range01`（最安全）

从 skip list 中删除 `close_range01`，预计可直接通过。

### 第二步：将 `epoll*` 替换为精确 skip 列表

将 `epoll*` 替换为：
```
epoll_ctl03|epoll_ctl04|epoll_ctl05|epoll_wait04|epoll_wait05|epoll_wait06|epoll_wait07|epoll_pwait02|epoll_pwait03|epoll_pwait04|epoll_pwait05|epoll_wait02|epoll_wait03|epoll_create01|epoll_create02
```
然后逐步从 skip 列表中移除通过的测试。

> **注意**: 建议采用增量验证方式 — 每次只启用 2-3 个测试，运行确认无 hang/crash 后再启用更多。

### 第三步：将 `eventfd*` 替换为精确 skip 列表

类似思路，先启用 `eventfd01`、`eventfd2_01`、`eventfd2_02`。

### 第四步：修复 `copy_file_range` bug

在 `sys_copy_file_range` 中补充写入逻辑后启用 `copy_file_range01`。

### 第五步：尝试 `mremap01`

从 `mremap*` 中排除 `mremap01`，观察是否通过。

---

## 五、总量统计

| 分类 | 模式数量 | 匹配二进制估算 |
|------|---------|--------------|
| never_run | ~38 固定 + 通配符 | ~700+ |
| unimplemented_feature | ~50 模式 | ~180+ |
| known_hang | ~25 模式 | ~35 |
| known_crash | ~30 模式 | ~45 |
| known_fail (不 crash) | ~90 模式 | ~160+ |
| **potentially_fixable** | **6 通配符模式** | **~13 个可尝试启用** |

**当前总 skip**: 约 1120+ 个二进制被过滤  
**可尝试启用**: 约 13 个（close_range01 + epoll 基础 8 个 + eventfd 基础 3 个 + mremap01）  
**修 bug 后启用**: 约 1 个（copy_file_range01）
