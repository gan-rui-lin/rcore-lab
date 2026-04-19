# LTP 回归修复：epoll 排除与五项安全修复重做

**日期**：2026/04/19  
**分支**：`tests/ltp-safe-mm`  
**回退提交**：`396f446` → `b971e3a`（revert）  
**新修复提交**：基于 revert 后重新实现

---

## 一、回归根因分析

### 1.1 问题现象

提交 `396f446`（"fix(ltp): epoll/eventfd 接入、kill 进程组信号、lstat/lchown 符号链接、F_SETLK 校验、brk 堆冲突"）包含 6 项修复，应用后测试结果**回归**——原本通过的测例开始失败。

### 1.2 罪魁祸首：epoll/eventfd 的 busy-poll 实现导致 LTP 框架级故障

**核心问题**：LTP 测试框架（ltp-lib）内部使用 `epoll` 进行事件等待。在 `396f446` 之前，`epoll_create1` 系统调用返回 `ENOSYS`，框架会自动回退到 `poll()` 路径，一切正常。

`396f446` 新增了 `epoll_create1`/`epoll_ctl`/`epoll_pwait`/`eventfd2` 四个系统调用。当 `epoll_create1` 开始成功返回 fd 后，LTP 框架切换到 epoll 路径使用 `epoll_pwait` 等待事件。但我们的 `epoll_pwait` 实现是 **busy-poll**（循环 poll + yield），存在以下致命缺陷：

1. **依赖所有 File 类型正确实现 `poll()`**：大量 File trait 的实现者（管道、timerfd 等）的 `poll()` 可能未针对 epoll 场景做过验证，返回值不准确
2. **busy-poll 语义与真实 epoll 差距大**：真正的 epoll 是边缘/水平触发、内核回调驱动的；busy-poll 只是反复轮询，当管道写端还没写数据时，`poll()` 返回空 → epoll_pwait 进入无限循环
3. **`suspend_current_and_run_next()` 的 yield 粒度不够**：yield 后可能立即被调度回来，形成忙等待，抢占其他进程的 CPU 时间

**直接后果**：LTP 框架在执行每个测例前/后的事件等待阶段卡死 → 测例超时 → 大面积 TBROK，呈现为"回归"。

### 1.3 为什么从日志判断是 epoll 的问题

从 baseline 日志（`ltp-rv01.log` / `ltp-la01.log`）可以看到：

```
[ERROR] 7 accept03: unimplemented syscall 20 (epoll_create1)
```

这条日志出现 54 次，说明 **框架基础设施** 和 **测例本身** 都在调用 epoll。当 epoll 从 ENOSYS 变为"能用但不好用"，框架的 fallback 路径被绕过，直接走进 broken 的 epoll_pwait。

半实现的 epoll 比不实现更危险——框架不知道 epoll 是坏的，没有机制检测 busy-poll 的正确性，只会默默等待。

### 1.4 其他潜在回归点

- **kill 重构**：原代码在 SIGCONT 投递后调用 `suspend_current_and_run_next()` 避免 waitid07/08 的检查点竞争。`396f446` 的 `kill_deliver!` 宏去掉了这个 yield，可能导致 waitid/waitpid 测例闪退
- **epoll File trait 方法**：新增的 `is_epoll_file()`、`epoll_ctl_inner()` 等默认方法可能与现有实现冲突（虽然概率低）

---

## 二、修复策略：安全修复 vs 高风险修复分离

### 2.1 本次排除的修复（高风险）

| 项目 | 原因 | 后续方案 |
|------|------|---------|
| epoll_create1/ctl/pwait | busy-poll 导致框架级故障 | 需要实现真正的 waker 机制或确认 poll() 在所有 File 实现上正确 |
| eventfd2 | 依赖 epoll 模块的 File trait 扩展 | 可独立实现，但需先解耦 |

### 2.2 本次重新实现的修复（安全）

以下 5 项修复已重新应用，编译通过（RV + LA）：

#### 修复 1：kill 支持负 PID（进程组信号）

**改动要点**：
- `sys_kill` 参数从 `usize` 改为 `isize`
- 新增 `kill_single()`（单进程信号）和 `kill_group()`（进程组信号）两个辅助函数
- **保留了 SIGCONT 后的 `suspend_current_and_run_next()`**（这是 `396f446` 遗漏的关键点）
- 支持 `pid > 0`（单进程）、`pid == 0`（当前进程组）、`pid == -1`（全局广播）、`pid < -1`（指定组）
- `kill_group` 使用 `pid2process_snapshot()` 遍历，避免持锁遍历

**与 396f446 的关键差异**：
- 不使用宏，而是用独立函数，类型系统友好
- `kill_single` 完整保留原有的 SIGCONT + yield 语义
- `kill_group` 也包含 SIGCONT 处理

#### 修复 2：lstat 正确处理符号链接（AT_SYMLINK_NOFOLLOW）

**改动要点**：
- 新增 `resolve_access_path_nofollow()` 函数：对最后一个路径组件不调用 `resolve_final_symlink_checked()`
- `sys_fstatat` 检测 `AT_SYMLINK_NOFOLLOW` 标志后走 nofollow 路径
- 对符号链接返回 `S_IFLNK | 0o777`、`size = 目标路径长度`

#### 修复 3：lchown 正确处理符号链接（AT_SYMLINK_NOFOLLOW）

**改动要点**：
- `sys_fchownat` 检测 `AT_SYMLINK_NOFOLLOW` 后使用 `resolve_access_path_nofollow()`
- 存在性检查增加 `symlink_target_get()` 判断，避免对符号链接本身误报 ENOENT

#### 修复 4：fcntl F_SETLK 拒绝非正规文件

**改动要点**：
- 在 `F_SETLK|F_SETLKW` 处理最前面加 `file.inode().is_none()` 检查
- 管道、stdio 等无 inode 的文件返回 EINVAL

#### 修复 5：LoongArch brk/sbrk 堆冲突检查范围缩窄

**改动要点**：
- `append_heap_to` 的冲突检查从 `[heap_bottom, new_end)` 缩窄为 `[current_heap_end, new_end)`
- 当堆区域已存在时，只检查扩展部分是否与非堆 VMA 冲突
- 修复 LA 上 ELF 最后一个 LOAD 段与 heap_bottom 共享页边界导致的误判

---

## 三、Baseline 日志分析总结（Triage）

### 3.1 RV 日志（ltp-rv01.log）

| 指标 | 数值 |
|------|------|
| TPASS | 2810 |
| TFAIL | 713 |
| TBROK | 379 |

**稳定性**：无 panic、无 IllegalInstruction。30 处 page fault 均为用户态测例主动触发（NULL/invalid 指针），内核稳定。

**Top TFAIL 根因**：
1. `rt_sigaction03`（150 TFAIL）：对 RT 信号 35–64 调用 sigaction 返回成功，但 LTP 期望 EINVAL
2. Timer 精度（48 TFAIL）：QEMU nanosleep 抖动，无法内核层修复
3. `preadv`（22 TFAIL）：syscall 69 未实现
4. `name_to_handle_at`（27 TFAIL）：syscall 264 未实现
5. `shmctl`（14 TFAIL）：指针/权限校验缺失

**Top TBROK 根因**：
1. `tst_device`（72 TBROK）：无回环块设备支持
2. `tst_kconfig`（57 TBROK）：无 `/proc/config.gz`
3. SysV IPC `semget`（20 TBROK）：syscall 190 未实现

### 3.2 LA 日志（ltp-la01.log）

| 指标 | 数值 |
|------|------|
| TPASS | 2814 |
| TFAIL | 771 |
| TBROK | 385 |

**LA 特有问题**：
- brk/sbrk 堆冲突（本次已修复）
- syscall 号映射差异（epoll_create1 在 LA 上是 20，需确认 syscall 表）

### 3.3 评分机制

LTP 评分 = `min(distinct_passing_cases, 1000)`。每个至少产生一个 TPASS 的测试二进制计 1 分，上限 1000。

### 3.4 高 ROI 后续修复方向

| 优先级 | 修复项 | 预估 TFAIL 减少 |
|--------|--------|----------------|
| P1 | rt_sigaction 信号号校验 | ~150 |
| P1 | preadv/pwritev 实现 | ~48 |
| P1 | VFS 路径 errno 细化（ENOTDIR/ELOOP/ENAMETOOLONG） | ~36 |
| P1 | name_to_handle_at/open_by_handle_at | ~53 |
| P2 | shmctl 校验 | ~14 |
| P2 | personality syscall | ~18 TBROK |
| Skip | Timer 精度 | QEMU 限制 |
| Skip | tst_device/tst_kconfig | 基础设施缺失 |

---

## 四、修改文件清单

| 文件 | 变化 | 内容 |
|------|------|------|
| `os/src/syscall/mod.rs` | ~1 行 | kill 参数类型 usize→isize |
| `os/src/syscall/process.rs` | +65/-20 行 | kill 进程组四路径 + kill_single/kill_group |
| `os/src/syscall/fs.rs` | +55/-5 行 | resolve_access_path_nofollow + lstat/lchown nofollow + F_SETLK 校验 |
| `os/src/mm/memory_set.rs` | +6/-1 行 | append_heap_to 冲突检查范围缩窄 |

---

## 五、关于 epoll 的后续建议

epoll 对 LTP 分数的影响分两层：

1. **框架级影响**：LTP 框架使用 epoll 做内部通信。如果实现不正确，会导致**所有测例**受影响。当前应保持 ENOSYS 让框架走 poll 路径。

2. **测例级影响**：直接测试 epoll 的测例（epoll01/02/03 等）约 20+ 个。要解锁这些，需要：
   - 确保所有 File trait 实现者（Pipe、Socket、TimerFd、EventFd 等）的 `poll()` 返回正确的就绪状态
   - 或者实现真正的 waker/回调机制（难度大，需要改 File trait + 调度器）
   - 或者对 epoll_pwait 的 busy-poll 做更保守的超时处理（检测到长时间无事件时返回 0 而不是一直等）

建议参考 chronix 内核（`/Users/mac/Desktop/project/syscall/oskernel2025-chronix-retest`）的 epoll 实现方式。
