# 文件描述符上限(RLIMIT_NOFILE)修复与 pthread_cancel 异步取消问题分析

**日期**: 2026/3/4
**分支**: busybox-test
**状态**: RLIMIT_NOFILE 已修复；pthread_cancel 异步取消待解决

---

## 一、问题概述

通过 `LOG=DEBUG timeout 120 bash run.sh -f sdcard-rv.img -t all` 运行完整测试集，发现两个关键问题：

| 编号 | 问题 | 严重性 | 状态 |
|------|------|--------|------|
| BUG-1 | `alloc_fd()` 不检查 RLIMIT_NOFILE，daemon_failure 测试无限循环导致超时 | **严重** | ✅ 已修复 |
| BUG-2 | pthread_cancel 异步取消不工作（cancel_handler 返回 sigreturn 而非 pthread_exit） | **中等** | ⚠️ 待解决 |

---

## 二、BUG-1：RLIMIT_NOFILE 未执行导致 daemon_failure 卡死

### 2.1 罪魁祸首

**内核的 `alloc_fd()` 函数（`os/src/task/process.rs:116`）永远不会失败**。它只会无限扩展 `fd_table` 向量，从不检查 `RLIMIT_NOFILE` 限制。

### 2.2 问题发现过程

运行测试 `LOG=DEBUG timeout 120 bash run.sh -f sdcard-rv.img -t all > all1.log` 后，2 分钟超时退出（exit code 124）。搜索日志发现：

```
# pid=113 从 fd=8 一直 dup 到 fd=49531+，永不停止
[SYSCALL] pid=113 name=entry-static.exe num=23(dup) args=[0x1,...] ret=8
[SYSCALL] pid=113 name=entry-static.exe num=23(dup) args=[0x1,...] ret=9
...
[SYSCALL] pid=113 name=entry-static.exe num=23(dup) args=[0x1,...] ret=49531
```

最后一个启动但未结束的测试是 `daemon_failure`：
```
========== START entry-static.exe daemon_failure ==========
（无 END 标记 — 卡死）
```

### 2.3 根因分析

`daemon-failure.c`（libc-test 回归测试）的核心逻辑：

```c
void t_fdfill(void) {
    int fd = 1;
    if (dup(fd) == -1) { ... }
    while(dup(fd) != -1);  // 循环直到 dup 返回 EMFILE
}

int main(void) {
    ...
    t_fdfill();      // 耗尽所有 fd
    daemon(0, 0);    // 预期因 open("/dev/null") 失败返回 -1
    // 验证 errno == EMFILE
}
```

测试设计意图：调用 `t_fdfill()` 将 fd 耗尽到 RLIMIT_NOFILE 上限，之后 `daemon()` 内部的 `open("/dev/null")` 应返回 `EMFILE`，从而验证 daemon 在失败时不 fork 的行为。

但内核的 `alloc_fd()` 实现：

```rust
// 修复前：永远成功，无限扩展
pub fn alloc_fd(&mut self) -> usize {
    if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
        fd
    } else {
        self.fd_table.push(None);
        self.fd_table.len() - 1  // 永远返回新 fd，从不失败
    }
}
```

尽管 `default_rlimits()` 正确设置了 `RLIMIT_NOFILE = 1024`，但没有任何代码检查这个限制。

### 2.4 修复方案

修改 `alloc_fd()` 返回 `Option<usize>`，在分配前检查 RLIMIT_NOFILE：

```rust
pub fn alloc_fd(&mut self) -> Option<usize> {
    let limit = self.rlimits[RLIMIT_NOFILE].rlim_cur as usize;
    if let Some(fd) = (0..self.fd_table.len()).find(|fd| self.fd_table[*fd].is_none()) {
        if fd < limit { Some(fd) } else { None }
    } else {
        let new_fd = self.fd_table.len();
        if new_fd >= limit { return None; }
        self.fd_table.push(None);
        Some(new_fd)
    }
}
```

同时更新所有 4 个调用点，在 `None` 时返回 `errno(EMFILE)`：

| 文件 | 函数 | 修改 |
|------|------|------|
| `syscall/fs.rs:251` | `sys_open` | `alloc_fd()` → match Some/None |
| `syscall/fs.rs:508` | `sys_dup` | 同上 |
| `syscall/fs.rs:887-889` | `sys_pipe2` | 两次 alloc_fd，第二次失败需回滚第一次 |
| `syscall/fs.rs:526-528` | `sys_dup3` | 检查 newfd 是否超过 RLIMIT_NOFILE |

### 2.5 修复效果

修复后测试从超时(2分钟)变为正常完成(exit code 0)：

- **修复前**: 只能跑到 daemon_failure 就卡死，entry-static 运行到约 60%
- **修复后**: 全部 107 个 entry-static 测试完成，99 通过，8 失败

entry-static.exe 失败的 8 个测试：

| 测试 | 退出状态 | 原因 |
|------|---------|------|
| pthread_cancel | 247 (SIGKILL) | TLS 偏移问题，异步取消不工作 |
| socket | 1 | socket 系统调用未实现 |
| stat | 1 | S_ISCHR 检查失败 |
| utime | 1 | futimens 未完整实现 |
| fflush_exit | 1 | 首次运行到，待分析 |
| pthread_robust_detach | 1 | 首次运行到，待分析 |
| statvfs | 1 | statvfs 未实现 |
| syscall_sign_extend | 1 | 符号扩展问题，待分析 |

entry-dynamic.exe 全部失败（status 1），因为不支持动态链接，属预期行为。

---

## 三、BUG-2：pthread_cancel 异步取消问题

### 3.1 问题表现

`pthread_cancel` 测试以 status 247 失败。247 = 被 SIGKILL 终止（runtest.exe 超时 10 秒后杀掉）。

### 3.2 详细日志分析

测试流程（从 all2.log 第 2657-2739 行）：

```
1. pid=36(entry-static.exe) 执行 pthread_cancel 测试
2. clone(flags=0x7d0f00, stack=0x40022ac0, tls=0x40022bd0) → 创建子线程 tid=1
3. 子线程设置 cancelasync=1（pthread_setcanceltype）
4. 子线程 sem_post 通知主线程
5. 子线程进入 for(;;) 死循环（sepc=0x27428）
6. 主线程收到信号量，调用 pthread_cancel → tkill(tid=1, sig=33)

7. 子线程收到 SIG33(SIGCANCEL)：
   - 进入 cancel_handler(0x3e134)
   - 【关键】cancel_handler 读 self->cancelasync == 0（应为 1）
   - handler 不调用 pthread_exit，而是 sigreturn 返回

8. sigreturn 恢复 PC=0x27428（回到 for(;;) 循环）
9. 子线程继续死循环
10. 10秒后 runtest.exe 超时 → kill(pid=36, SIGKILL) → FAIL [status 247]
```

关键日志：
```
[sigreturn] pid=36 ucontext_ptr=0x40022750 saved_pc=0x27428 ucontext_pc=0x27428 sigmask=0x100010000
[sample] pid=36 name=entry-static.exe sepc=0x27428 sp=0x40022a90 ra=0x27428
# ↑ sigreturn 后子线程回到 0x27428，进入死循环
```

### 3.3 原因分析

musl 的 cancel_handler 通过 TLS 获取 `self`（pthread 结构体）指针：

```c
// RISC-V musl TLS_ABOVE_TP 布局
static inline struct __pthread *__pthread_self() {
    char *self;
    __asm__("mv %0, tp" : "=r"(self));
    return (void*)(self - sizeof(struct __pthread) - GAP_ABOVE_TP);
}
```

然后读取 `self->cancelasync` 字段。如果 `cancelasync != 0` 且异步取消已启用，则直接调用 `__cancel()` → `pthread_exit(PTHREAD_CANCELED)`。

**但 cancel_handler 读到的 `self->cancelasync` 是 0**，说明 `self` 指针计算错误或 TLS 偏移有问题。

可能原因：
1. **`sizeof(struct __pthread)` 不匹配**：如果内核假设的 pthread 结构体大小与实际 musl 编译出的大小不同，`self` 指针就会偏移
2. **内核 TLS 初始化覆盖了 musl 的 TLS**：在 exec 时内核写入的 TLS 区域可能影响了 musl 后续的初始化
3. **信号帧中 tp 寄存器未正确保存/恢复**：如果 sigreturn 恢复了错误的 tp 值

### 3.4 为什么 pthread_cancel_points 能通过

`pthread_cancel_points` 测试使用**延迟取消**（默认模式），不依赖 `cancelasync` 字段：
- musl 的 `__syscall_cp_asm` 在每个取消点（系统调用）入口检查 `cancel` 标志
- 如果 `cancel=1`，跳转到 `__cp_cancel` → `__cancel()` → `pthread_exit`
- 这个机制通过内存中的 cancel 标志而非 TLS 中的 cancelasync 字段工作

---

## 四、下一步计划

### 4.1 高优先级：调试 pthread_cancel 异步取消

1. **反汇编 cancel_handler**：挂载 sdcard-rv.img，提取 entry-static.exe，反汇编 0x3e134 处的 cancel_handler 代码，确认其如何读取 `cancelasync`

2. **GDB 断点调试**：
   ```bash
   LOG=INFO bash run.sh -f sdcard-rv.img -t debug -d > debug.log 2>&1 &
   riscv64-unknown-elf-gdb os/target/.../release/os
   # 在 cancel_handler 入口设断点
   (gdb) b *0x3e134
   # 检查 tp 值和 self 指针
   (gdb) p/x $tp
   (gdb) x/xg ($tp - sizeof_pthread - 16)
   ```

3. **添加内核调试日志**：在 `handle_signals` 的信号投递路径中打印 tp (x[4]) 的值，在 sigreturn 路径中打印恢复的 tp 值，验证 tp 是否在信号处理前后一致

4. **检查 musl 的 `sizeof(struct __pthread)`**：从 musl 编译产物中确认实际大小，与 TLS 布局对比

### 4.2 中优先级：其他失败测试

- **fflush_exit**: 分析退出时 fflush 行为
- **syscall_sign_extend**: 检查 64 位系统调用参数的符号扩展
- **pthread_robust_detach**: 检查 robust mutex 支持
- **statvfs**: 实现 statvfs 系统调用

### 4.3 低优先级

- entry-dynamic.exe 动态链接支持（当前全部失败）
- stat 测试中 S_ISCHR 的字符设备支持

---

## 五、修改的文件清单

| 文件 | 修改内容 |
|------|---------|
| `os/src/task/process.rs` | `alloc_fd()` 改为返回 `Option<usize>`，检查 RLIMIT_NOFILE |
| `os/src/syscall/fs.rs` | `sys_open`/`sys_dup`/`sys_dup3`/`sys_pipe2` 处理 EMFILE |
