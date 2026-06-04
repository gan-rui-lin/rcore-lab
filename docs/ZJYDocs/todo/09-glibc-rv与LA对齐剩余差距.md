# glibc-rv 与 LA 对齐剩余差距分析

**日期**: 2026/06/04

---

## 当前分数对比

| 测试集 | TPASS | TFAIL | TBROK | 通过率 |
|--------|-------|-------|-------|--------|
| LA musl (baseline) | 2814 | 771 | 385 | 70.9% |
| **新 RV glibc** | **3363** | 232 | 471 | 82.7% |
| 新 RV musl (部分) | 3564+ | 269+ | 260+ | ~82% |

**结论**: RV glibc TPASS (3363) 已超过 LA musl (2814)，但 TBROK 较高（471 vs 385）。

---

## RV glibc TBROK=471 的主要来源

### 1. getpwnam/getpwuid 失败 — 约 30-40 个
- COW 修复后大部分解决，但部分场景仍有 EFAULT
- 可能是 glibc 2.35 的 NSS 在特定路径下的行为

### 2. "Cannot parse kernel .config" — 36 个
- 需要 `/proc/config.gz`，LTP 框架用来检测内核功能
- 实现方案：创建虚拟的 `/proc/config.gz`，返回最小有效内容

### 3. "Failed to acquire device" — 22 个
- 需要 loop 设备，文件系统测试依赖
- 实现复杂度高，低优先级

### 4. mq_open/mq_notify ENOSYS — 约 10 个
- POSIX 消息队列未实现（mq_open, mq_close, mq_send, mq_receive, mq_notify, mq_unlink）
- 实现难度中等

### 5. SysV semaphore ENOSYS — 约 30 个
- semget/semctl/semop/semtimedop 未实现
- 参考已有 msgget 实现，工作量适中

---

## TFAIL=232 的主要来源

### 1. errno 不匹配（最大类别）
- 内核返回的 errno 与 Linux 标准不一致
- 例：`fcntl(F_SETLK)` 期望 EFAULT 返回了 EINVAL
- 需要逐个对齐

### 2. 信号行为差异
- `rt_sigaction` 某些边界条件
- `siginfo_t` 字段不完整

### 3. 文件系统语义差异
- `msync` 成功但不应该成功
- `mmap` 权限检查不够严格

---

## 提分优先级路线图

### 第一梯队（预计 +100-200 TPASS）
1. SysV semaphore (semget/semctl/semop/semtimedop) — 解锁 ~30 测试
2. 移除 skip list 中已实现的 syscall 对应测试（epoll, eventfd, mremap 等）
3. /proc/config.gz 虚拟文件 — 解锁 ~36 测试

### 第二梯队（预计 +50-100 TPASS）
4. POSIX 消息队列 (mq_*) — 解锁 ~10 测试
5. pidfd_open, faccessat2, openat2 — 简单实现
6. errno 对齐修复（逐个 case）

### 第三梯队（长期）
7. /proc/self/ns/* 虚拟文件
8. loop 设备模拟
9. inotify 完整实现
