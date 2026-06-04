# 待办事项索引

**日期**: 2026/06/04  
**分支**: feat/ltp-dev  
**基准**: glibc-rv TPASS 1045 → 3363 (+222%)

---

## 文档列表

| 编号 | 文档 | 说明 |
|------|------|------|
| 01 | [LTP glibc-rv 失败分类](01-LTP-glibc-rv失败分类.md) | TBROK/TFAIL 按根因聚类分析 |
| 02 | [新增系统调用代码审查](02-新增系统调用代码审查.md) | epoll/eventfd/signalfd/timer/mremap 代码质量 |
| 03 | [Skip List 分析与优化](03-Skip-List分析与优化.md) | 已跳过测试的分类与解除建议 |
| 04 | [内存管理待办](04-内存管理待办.md) | COW 安全性、mremap、mlock/mincore |
| 05 | [信号与定时器待办](05-信号与定时器待办.md) | POSIX timer REALTIME、sigaltstack、inotify |
| 06 | [缺失系统调用优先级](06-缺失系统调用优先级.md) | 按频率排序的未实现 syscall |
| 07 | [procfs 与虚拟文件系统](07-procfs与虚拟文件系统待办.md) | /proc 条目、/dev 设备、/etc 文件 |
| 08 | [性能与稳定性](08-性能与稳定性待办.md) | 内存泄漏、死锁风险、busy-poll |
| 09 | [glibc-rv 与 LA 对齐](09-glibc-rv与LA对齐剩余差距.md) | 剩余差距分析与提分路线图 |

---

## 快速提分路线（按 ROI 排序）

1. **SysV semaphore** (190-193) — 预计 +30 TPASS
2. **移除已实现 syscall 的 skip** (epoll/eventfd/mremap) — 预计 +20-50 TPASS
3. **/proc/config.gz** 虚拟文件 — 预计 +36 TPASS
4. **POSIX 消息队列** (mq_*) — 预计 +10 TPASS
5. **简单 stub syscall** (pidfd_open/faccessat2/openat2) — 预计 +15 TPASS
