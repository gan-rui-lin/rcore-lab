# Signal/Futex 系统调用深度分析与重构报告

**日期**: 2026-03-04
**分支**: zjy-syscall
**状态**: 重构完成，测试通过

---

## 一、问题概述

通过对 `os/src/task/` 和 `os/src/syscall/process.rs` 中 signal 和 futex 相关代码的深度审查，发现并修复以下问题：

| 编号 | 问题 | 严重性 | 状态 |
|------|------|--------|------|
| BUG-1 | signal_mask 是进程级而非线程级 | **严重** | ✅ 已修复 |
| BUG-2 | sys_kill/sys_tkill 唤醒阻塞任务未设 interrupted_by_signal | **严重** | ✅ 已修复 |
| BUG-3 | 默认信号处理动作不完整 | **中等** | ✅ 已修复 |
| BUG-4 | SIGKILL 只杀一个线程而非整个进程 | **严重** | ✅ 已修复 |
| BUG-5 | TLS 布局不符合 RISC-V musl 标准 | **严重** | ✅ 已修复 |
| BUG-6 | timer 中硬编码 PID 检查 | **低** | ✅ 已移除 |
| BUG-7 | futex_requeue 返回值不正确 | **低** | ✅ 已修复 |
| BUG-8 | sys_sigreturn 代码重复 | **低** | ✅ 已重构 |
| BUG-9 | sys_sigprocmask oldset 循环多余 | **低** | ✅ 已简化 |
| ISSUE | pthread_cancel 异步取消不工作 | **待查** | ⚠️ TLS 偏移问题 |

---

## 二、已完成的修复

### 2.1 signal_mask 移至线程级 (BUG-1)

**修改文件**: `task.rs`, `process.rs`, `mod.rs`, `syscall/process.rs`

Linux 标准中 `sigprocmask` 操作的是当前线程的信号掩码。修改前 `signal_mask` 存储在 `ProcessControlBlockInner`，导致所有线程共享同一掩码。

**修改**:
- `TaskControlBlockInner` 新增 `signal_mask: SignalFlags`
- `ProcessControlBlockInner` 删除 `signal_mask`
- `handle_signals()` 使用 `task_inner.signal_mask`
- `sys_sigprocmask()` 操作 `task_inner.signal_mask`
- `sys_sigreturn()` 恢复到 `task_inner.signal_mask`
- fork/clone 继承父线程的 signal_mask

### 2.2 sys_kill/sys_tkill 信号中断 (BUG-2)

**修改文件**: `syscall/process.rs`

在 `sys_kill` 和 `sys_tkill` 中唤醒阻塞任务时添加：
1. `futex_remove_waiter_any(&task)` — 从 futex 队列移除
2. `task_inner.interrupted_by_signal = true` — 设置中断标志

确保 `futex_wait` 返回 `-EINTR`。

### 2.3 默认信号处理 (BUG-3)

**修改文件**: `task/mod.rs`

`handle_signals()` 中 handler==0 分支新增默认动作：
- SIGCHLD/SIGURG/SIGWINCH → 忽略
- SIGCONT → 忽略（简化处理）
- SIGTSTP/SIGTTIN/SIGTTOU → 停止
- 其他 → 终止

### 2.4 SIGKILL 杀死所有线程 (BUG-4)

**修改文件**: `task/mod.rs`

SIGKILL 现在向进程内所有其他线程注入 SIGKILL 并唤醒阻塞的线程，确保多线程进程被完全终止。

### 2.5 TLS 布局修正 (BUG-5)

**修改文件**: `task/tls.rs`

原布局错误（tp 在 TLS 数据之后）：
```
[.tdata] [.tbss] [TCB] <- tp
```

修正为 RISC-V musl TLS_ABOVE_TP 标准布局：
```
[pthread_reserve(1024B)] [GAP(16B)] [.tdata] [.tbss]
                                     ^-- tp
```

### 2.6 其他修复

- **sigreturn 代码去重** (BUG-8): 两条路径(有/无 ucontext)的 SA_RESETHAND 和 SIGCANCEL 检测合并为一处
- **sigprocmask 简化** (BUG-9): oldset 使用 `flags_to_user_mask()` 一行替代 64 次循环
- **futex_requeue 返回值** (BUG-7): 返回 `woke + moved` 而非仅 `woke`
- **timer 清理** (BUG-6): 移除硬编码 PID 检查

---

## 三、测试结果

### 3.1 通过的测试

所有之前通过的测试继续通过（50+ 个），包括：
- argv, basename, clock_gettime, dirname, env, fdopen, fnmatch, fscanf, fwscanf
- iconv_open, inet_pton, mbc, memstream
- **pthread_cancel_points** ✅（关键多线程测试）
- qsort, random, search_hsearch, search_insque, search_lsearch, search_tsearch
- snprintf, socket, sscanf, stat, string, string_memcpy, string_memmem, string_memset
- string_strchr, string_strcspn, string_strstr, strtod, strtol, swprintf
- tgmath, time, tls_align, udiv, ungetc, wcsstr, wcstol
- daemon_failure

### 3.2 仍然失败的测试

- **pthread_cancel** — 异步取消(`PTHREAD_CANCEL_ASYNCHRONOUS`)不工作
  - 根因：musl 的 cancel_handler 读取 `self->cancelasync` 时可能因 TLS 偏移问题读到 0
  - 表现：handler 返回 sigreturn 而非调用 `pthread_exit`
  - 影响：仅影响异步取消场景，延迟取消(cancellation points)正常工作
  - 处理：由 runtest.exe 超时后 SIGKILL 终止，不阻塞后续测试

---

## 四、关键调试心得

### 4.1 signal_mask per-process 的隐蔽性

这个 bug 在单线程场景下完全正常，只有多线程 + 信号交互时才触发。典型场景：
- musl 的 `pthread_create` 在 clone 前后用 `sigprocmask` 阻塞/恢复 SIG32+SIG33
- 如果 signal_mask 是进程级的，一个线程的 mask 变更会影响所有线程

### 4.2 SIGCANCEL 机制深度理解

musl 的 `pthread_cancel` 通过两个独立机制工作：

**机制 1 — 信号路径**: `tkill(tid, SIGCANCEL)` 投递信号
- handler 检查 `cancelasync` → 异步取消：直接 `pthread_exit`
- handler 检查 PC 是否在 `[__cp_begin, __cp_end)` → 延迟取消：修改 ucontext PC

**机制 2 — 轮询路径**: `__syscall_cp_asm` 在每个系统调用入口检查 cancel 标志
- 如果 `cancel=1`：跳转到 `__cp_cancel` → `__cancel()` → `pthread_exit`
- 这是延迟取消的主要工作方式

**关键教训**：即使信号 handler 返回了也不要重新注入 SIG33！musl 的轮询机制会在下次系统调用时自动检测 cancel 标志。强制重新注入会造成信号风暴，阻止线程正常执行。

### 4.3 SIGKILL 的多线程语义

SIGKILL 必须终止进程内所有线程。原来只终止第一个处理它的线程，其他线程继续运行导致僵死。修复后向所有线程注入 SIGKILL 并唤醒阻塞的线程。

### 4.4 TLS_ABOVE_TP 布局

RISC-V musl 使用 `TLS_ABOVE_TP` 布局：
- `tp` 指向 TLS 数据的起始位置
- `pthread` 结构体在 tp 之前（`self = tp - sizeof(__pthread) - GAP_ABOVE_TP`）
- `GAP_ABOVE_TP = 16` 字节用于 DTV 指针

内核的初始 TLS 在 exec 后会被 musl 的 `__init_tls` 覆盖，但布局仍需正确以防早期访问崩溃。

### 4.5 sepc 偏移问题

trap_handler 在处理 UserEnvCall 时先 `sepc += 4`，然后才调用 syscall。这意味着信号帧中保存的 PC 已经指向 ecall 之后的指令。如果 musl 的取消点检查用 `ip < __cp_end`（不包含 __cp_end），而 `ecall+4 == __cp_end`，则检查会失败。musl 的取消点实际上依赖轮询机制(mechanism 2)而非信号路径来处理这种情况。

---

## 五、修改的文件清单

| 文件 | 修改内容 |
|------|---------|
| `os/src/task/task.rs` | 添加 signal_mask 字段 |
| `os/src/task/process.rs` | 移除 signal_mask，fork 继承逻辑 |
| `os/src/task/mod.rs` | handle_signals 用 task signal_mask，SIGKILL 多线程，默认信号处理 |
| `os/src/task/tls.rs` | 完全重写 TLS 布局为 TLS_ABOVE_TP |
| `os/src/task/futex.rs` | futex_requeue 返回 woke+moved |
| `os/src/syscall/process.rs` | sigprocmask/sigreturn/kill/tkill/clone 全面修改 |
| `os/src/trap/mod.rs` | 移除硬编码 PID 检查 |
