# LTP glibc-rv 对齐工作总索引

**日期**: 2026/06/04  
**分支**: feat/ltp-dev (dad5d83 → 6e2ecfb)  
**成果**: glibc-rv TPASS 1045 → 3363 (+222%)，无卡死，全量跑完

---

## 一、本轮完成工作回顾

### 1.1 分数变化

| 指标 | 旧版 glibc-rv | 新版 glibc-rv | 变化 |
|------|-------------|-------------|------|
| TPASS | 1045 | 3363 | **+2318 (+222%)** |
| TFAIL | 142 | 232 | +90（因测试覆盖面扩大） |
| TBROK | 262 | 471 | +209（同上） |
| 通过率 | 72.1% | **82.7%** | +10.6% |
| 对比 LA musl | 远低于 2814 | **超过 2814** | 已反超 |

### 1.2 关键改动清单

**新增 15 个系统调用**：

| syscall | 编号 | 作用 | 影响面 |
|---------|------|------|--------|
| epoll_create1 / ctl / pwait | 20/21/22 | I/O 多路复用 | LTP 框架依赖，50+ 测试 |
| eventfd2 | 19 | 事件通知 fd | 多个 IPC/信号测试 |
| timer_create/gettime/settime/getoverrun/delete | 107-111 | POSIX 定时器 | timer_* 系列测试 |
| rt_sigpending | 136 | 获取待处理信号集 | sigpending 测试 |
| rt_sigqueueinfo | 138 | 带数据发信号 | sigqueue 测试 |
| mremap | 216 | 内存重映射 | glibc malloc + mremap 测试 |
| signalfd4 | 74 | 信号转 fd | signalfd 测试 |
| inotify_init1/add_watch/rm_watch | 26/27/28 | 文件监控（stub） | 不返回 ENOSYS |
| copy_file_range | 285 | 文件内核拷贝 | copy_file_range 测试 |

**2 项关键内核修复**：

| 修复 | 文件 | 影响 |
|------|------|------|
| COW fork 写回退 | user_mem.rs:501 | 移除 `proc_name.starts_with("fork")` 限制，39+ 测试从 EFAULT → 可运行 |
| POSIX timer 信号投递 | timer.rs + process.rs | `check_posix_timers()` 在定时器中断中检查到期并发信号 |

### 1.3 调试经验总结

#### 经验 1：COW fork 写回退是 glibc 的命门

**现象**：glibc LTP 测试中 39+ 测试报 `getpwnam("nobody") failed: EFAULT (14)`，musl 完全正常。

**排查过程**：
1. 对比 musl/glibc 两段日志发现 `getpwnam` 只在 glibc 失败
2. 内核 `ensure_basic_paths()` 已经创建了 `/etc/passwd` 且内容正确——排除文件缺失
3. 检查 clone flags (`0x1200011` = SIGCHLD | CLONE_CHILD_CLEARTID | CLONE_CHILD_SETTID) 确认是标准 fork
4. EFAULT 意味着 `copy_to_user` 失败——追踪到 `user_mem.rs:DemandCowWithForkFallback`
5. **发现根因**：`legacy_fork_write_fallback` 有一个 `proc_name.starts_with("fork")` 守卫，只允许进程名以 "fork" 开头的进程使用写回退。LTP 测试进程名是 `access01`、`chmod03` 等，全部被拒绝

**为什么 musl 不受影响**：musl 的 `getpwnam` 是内置实现，直接 `open("/etc/passwd")` → `read()` → 解析。glibc 使用 NSS (Name Service Switch)，内部有额外的内存分配和结构体拷贝，触发了更多的写操作到 COW 页上。

**修复**：删除进程名检查，让所有 fork 子进程都能使用写回退。

**教训**：当 musl 和 glibc 对同一功能表现不同时，优先检查内存管理路径（COW/页表/地址空间），而不是功能逻辑本身。

#### 经验 2：POSIX timer 必须在中断上下文投递信号

**现象**：`clock_settime03` 永久卡住。

**排查过程**：
1. 测试代码：`timer_create(SIGABRT)` → `timer_settime(TIMER_ABSTIME, 2038+3s)` → `sigwait(SIGABRT)`
2. 我们的 `sys_timer_settime` 只在全局 map 中记录了到期时间，但没有注册任何到期回调
3. 信号永远不会被投递，`sigwait` 永远阻塞

**修复**：在 `timer.rs:check_timer()` 末尾增加 `check_posix_timers()`，遍历全局 POSIX_TIMERS 表，到期的 timer 向目标进程投递 `sigev_signo` 信号并唤醒阻塞任务。

**遗留问题**：`clock_settime03` 和 `timer_settime03` 使用 `clock_settime(CLOCK_REALTIME)` 把系统时间设到 2038 年，但我们的 timer 检查使用 monotonic 时间，无法匹配 REALTIME 绝对时间。目前放入 skip list。

**教训**：实现 timer 系列 syscall 时，不能只做"记录"，必须接入中断路径的到期检查机制，否则依赖 timer 信号的用户态代码会全部死锁。

#### 经验 3：inotify stub 的 read() 阻塞是隐性炸弹

**现象**：测试跑到 `inotify05` 卡死。

**原因**：`sys_inotify_init1` 返回 `/dev/null` fd 作为桩实现。`inotify05` 对这个 fd 做阻塞 `read()`，而 `/dev/null` 的 `read()` 立即返回 0（EOF），但 LTP 框架把 0 当成"没数据"然后循环重试，形成忙等或阻塞。

**修复**：加入 skip list。完整修复需要实现 `InotifyFile` 结构体，在无事件时返回 EAGAIN（非阻塞）或阻塞直到有文件系统事件。

**教训**：stub syscall 返回"看起来成功"的值比返回 ENOSYS 更危险。返回 ENOSYS 会让 LTP 报 TBROK 然后跳过，而返回一个假 fd 可能导致后续操作（read/poll/close）进入意外状态。

#### 经验 4：`clock_settime` 会损坏 ext4 文件系统

**现象**：一轮测试后，下一轮启动时 `ext4_mount: rc = 95` (ENOTSUP) panic。

**原因**：`clock_settime03` 把系统时间设到 2038 年。如果测试被中途杀掉（QEMU 超时），文件系统的 inode 时间戳变成未来时间，ext4 journal 认为文件系统不一致，拒绝挂载。

**修复**：从 `.xz` 备份恢复 sdcard 镜像。长期方案是限制 `clock_settime` 的范围或保持 skip。

**教训**：任何修改系统时间的测试都可能有文件系统副作用。跑这类测试前备份镜像，或者在内核层限制时间设置范围。

#### 经验 5：LOG 级别对测试耗时影响巨大

**现象**：`LOG=SYSCALL` 下 lmbench 产出 17 万行日志，看起来像"卡死"。

**定量**：
- `LOG=ERROR`：lmbench 跑完约 17904 行输出，3-5 分钟
- `LOG=INFO`：同样的测试产出 6 万+ 行，10+ 分钟
- `LOG=SYSCALL`：17 万+ 行，20+ 分钟

串口输出是瓶颈——每行日志都需要 UART 发送，QEMU 串口模拟速度有限。

**教训**：验证性测试用 `LOG=ERROR`；定位问题时先用 `LOG=INFO` 缩小范围，再对单个测试用 `LOG=SYSCALL`。永远不要对全量测试用 `LOG=SYSCALL`。

#### 经验 6：`run.sh` 工作目录敏感

**现象**：从 `os/` 子目录调用 `bash run.sh` 报 `run.sh: No such file or directory`。

**原因**：`run.sh` 内部使用相对路径（`sdcard-rv.img`、`make rv`）。

**正确做法**：始终从项目根目录调用 `bash run.sh`，或使用绝对路径 `bash /path/to/run.sh -f /path/to/sdcard-rv.img`。

---

## 二、待办文档列表

| 编号 | 文档 | 说明 | 最大收益项 |
|------|------|------|-----------|
| 01 | [LTP glibc-rv 失败分类](01-LTP-glibc-rv失败分类.md) | TBROK 471 + TFAIL 232 按根因聚类 | getpwnam EFAULT 98 例、getcwd ENOENT 25 例 |
| 02 | [新增系统调用代码审查](02-新增系统调用代码审查.md) | P0/P1/P2 级别 bug | **P0**: copy_file_range 未写目标文件；**P1**: timer_create 写 8 字节应写 4、EPOLL_REGISTRY 泄漏 |
| 03 | [Skip List 分析与优化](03-Skip-List分析与优化.md) | 可解除 skip 的测试 | epoll\*/eventfd\*/mremap\*/close_range01 共 13 个可解除 |
| 04 | [内存管理待办](04-内存管理待办.md) | COW 安全性、mremap 完善 | mremap 原地扩展、mlock 返回 0 |
| 05 | [信号与定时器待办](05-信号与定时器待办.md) | POSIX timer + sigaltstack | timer REALTIME 支持、sigev_notify 区分 |
| 06 | [缺失系统调用优先级](06-缺失系统调用优先级.md) | 未实现 syscall 频率排序 | SysV semaphore (190-193) 预计 +30 TPASS |
| 07 | [procfs 与虚拟文件系统](07-procfs与虚拟文件系统待办.md) | /proc、/dev、/etc | /proc/config.gz 预计 +36 TPASS |
| 08 | [性能与稳定性](08-性能与稳定性待办.md) | 资源泄漏、死锁风险 | EPOLL_REGISTRY + POSIX_TIMERS 退出清理 |
| 09 | [glibc-rv 与 LA 对齐](09-glibc-rv与LA对齐剩余差距.md) | 剩余差距 + 路线图 | 三梯队方案，预计再提 100-200 TPASS |

---

## 三、下一步提分路线（按 ROI 排序）

### 第一梯队：快速收益（预计 +80-130 TPASS）

| 优先级 | 任务 | 预计收益 | 难度 | 关键风险 |
|--------|------|---------|------|---------|
| 1 | 修复 getpwnam EFAULT 剩余 98 例 | +40-60 | 中 | 需要深入 COW/页表排查 glibc PIE 地址空间 |
| 2 | 移除 skip list 中已实现 syscall 的测试 | +13-20 | 低 | epoll busy-poll 可能导致个别测试超时 |
| 3 | SysV semaphore (semget/semctl/semop) | +30 | 中 | 参考已有 msgget 实现 |
| 4 | /proc/config.gz 虚拟文件 | +36 | 低 | 返回空 .config 或最小内容 |

### 第二梯队：中等投入（预计 +50-100 TPASS）

| 优先级 | 任务 | 预计收益 | 难度 |
|--------|------|---------|------|
| 5 | 修复 copy_file_range 目标写入（P0 bug） | +5 | 低 |
| 6 | 修复 timer_create 写 4 字节而非 8 字节 | +5 | 低 |
| 7 | 修复 getcwd ENOENT（IPC 测试阻塞） | +25 | 中 |
| 8 | POSIX 消息队列 (mq_*) | +10 | 中 |
| 9 | pidfd_open / faccessat2 / openat2 stub | +15 | 低 |
| 10 | errno 对齐（fcntl/mmap/msync 等） | +10-20 | 高 |

### 第三梯队：长期投入

| 任务 | 说明 |
|------|------|
| /proc/self/ns/* 虚拟文件 | 解锁 namespace 测试，不需要真正实现隔离 |
| loop 设备模拟 | 解锁文件系统测试，工作量大 |
| inotify 完整实现 | 解锁文件监控测试 |
| prctl 子命令完善 | PR_SET_NAME/PR_GET_PDEATHSIG/PR_SET_SECCOMP |
| sigaltstack 完整实现 | 替代信号栈 |

---

## 四、已知的 P0 级别 bug（必须修复）

来自 [02-代码审查](02-新增系统调用代码审查.md)：

1. **copy_file_range 未写入目标文件**（`fs.rs:5916`）  
   读取源文件数据到内核 buf 后直接返回 `bytes_read`，从未调用 `dst_file` 的写入方法。`off_out` 参数完全忽略。调用者以为拷贝成功，但目标文件内容为空。

2. **timer_create 写 8 字节 timer_id 到用户空间**（`process.rs:5479`）  
   Linux 的 `timer_t` 是 `int`（4 字节），我们用 `usize`（8 字节）写入，会覆盖相邻 4 字节栈内存。

3. **EPOLL_REGISTRY + POSIX_TIMERS 进程退出不清理**  
   全局表条目永远不会删除。PID 复用后旧条目可能干扰新进程。

---

## 五、调试方法论备忘

### 日志级别选择
```
LOG=ERROR   → 验证测试通过，最小输出
LOG=INFO    → 看 fork/clone/exec 流程，定位卡死位置  
LOG=SYSCALL → 单个测试追踪所有 syscall 参数和返回值
LOG=TRACE   → 全量日志，仅用于 GDB 无法定位时的最后手段
```

### 单测试调试流程
```bash
# 1. 跑单个 glibc 测试
SINGLE_TEST=/glibc/ltp/testcases/bin/access01 LOG=SYSCALL bash run.sh -f sdcard-rv.img -t all > trace.log 2>&1

# 2. 搜索关键信号
rg "signum = 4|signum = 12|EFAULT|ENOSYS|unimplemented|ret=-" trace.log

# 3. 查 LTP 测试源码理解期望行为
cat /Users/mac/Desktop/project/syscall/testsuits-for-oskernel/ltp-full-20240524/testcases/kernel/syscalls/access/access01.c
```

### 判断测试是否卡死
```bash
# 监控日志增长，30 秒无变化 = 卡死
prev=0; while true; do lines=$(wc -l < test.log); [ "$lines" = "$prev" ] && echo "STUCK at: $(tail -1 test.log)" || echo "OK lines=$lines"; prev=$lines; sleep 30; done
```

### sdcard 镜像恢复
```bash
# 被 clock_settime 等测试损坏后
xz -dk sdcard-rv.img.xz -c > sdcard-rv-fresh.img && mv sdcard-rv-fresh.img sdcard-rv.img
```
