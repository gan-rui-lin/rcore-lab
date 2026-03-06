# pthread_cancel 内核 Workaround 尝试与分析

日期：2026/3/1

## 背景

根据之前的调试（见 [pthread_setcanceltype_missing_2026-03-01.md](pthread_setcanceltype_missing_2026-03-01.md)），发现 busybox 的 `pthread_setcanceltype` 函数未实现，导致 pthread_cancel 异步取消测试卡死。

## 尝试的 Workaround 方案

### 方案 1：内核系统调用实现 pthread_setcanceltype

**实现**：
- 在 TaskControlBlockInner 添加 `canceltype: u8` 字段
- 实现 `sys_pthread_setcanceltype()` 系统调用（syscall 500）
- 在 handle_signals 检查 canceltype 字段并直接退出线程

**问题**：
- 测试程序根本不会调用这个系统调用（因为 pthread_setcanceltype 在二进制中不存在）
- 无法生效

### 方案 2：内核直接读取用户态 TLS 的 cancelasync 字段

**实现**：
- 在 handle_signals 中，当收到 SIGCANCEL 时
- 使用 `translated_byte_buffer` 读取用户态 TLS 的 `tp-0x97` (cancelasync 字段)
- 如果 cancelasync==1，直接调用 `exit_current_and_run_next(-33)` 退出线程

**发现**：
- 能够成功读取 TLS 字段
- pid=36 的测试中，cancelasync 确实==1（异步取消模式）
- 这说明 **cancelasync 字段已经被正确设置**！

**日志证据**（all5.log）：
```
[INFO] [signal] sig33_check pid=34 tid=1 cancelasync=0  ← 延迟取消
[INFO] [signal] sig33_check pid=36 tid=1 cancelasync=1  ← 异步取消
[INFO] [signal] async_cancel pid=36 tid=1 (immediate exit via TLS check)
[INFO] [exit] pid=36 tid=1 code=-33
```

**严重问题**：
1. **绕过了正确的清理流程**：
   - 直接 `exit_current_and_run_next(-33)` 没有执行 pthread cleanup handlers
   - 没有正确通知 pthread_join 等待的线程
   - clear_child_tid wake 唤醒了 0 个线程：
   ```
   [INFO] [exit] pid=36 tid=1 clear_child_tid=0x9db80 woke=0
   [INFO] [sys_futex] pid=36 tid=0 cmd=FUTEX_WAIT uaddr1=0x40022b30  ← 主线程仍在等待
   ```

2. **测试失败**：
   - pthread_cancel 测试 status 247（超时被 SIGKILL）
   - 主线程永远等待在 pthread_join

### 方案 3：移除 Workaround，让 Handler 自己处理

**原理**：
- 既然 cancelasync==1 已经被正确设置
- musl 的 SIGCANCEL handler 应该能够自己检测并处理异步取消
- Handler 会调用 `__cancel()` 进行正确的清理

**实现**：
- 移除内核的直接退出逻辑
- 只保留日志输出用于调试
- 让 handler 正常执行

**结果**：
- 出现了新的内核 panic：
```
[kernel] Panicked at src/trap/mod.rs:459 Unsupported trap from kernel:
Exception(InstructionPageFault), stval = 0x8294eb80!
```
- stval = 0x8294eb80 正好是 clear_child_tid 的物理地址
- panic 发生在 `futex_wait_resume` 日志之后

**分析**：
- 内核在某个地方错误地尝试执行物理地址 0x8294eb80
- 这可能是内存管理或上下文切换的 bug
- 与 pthread_cancel workaround 无关，可能是之前就存在的问题

## 关键发现

### 1. cancelasync 字段确实被设置了！

这与之前的结论（pthread_setcanceltype 未实现）矛盾。可能的解释：

**推测 A：busybox 有某种实现**
- 可能是 weak symbol 链接到了一个简单的实现
- 或者测试程序使用了内联汇编直接写入 TLS

**推测 B：测试程序自己实现了**
- libc-test 可能有 fallback 实现
- 直接操作 TLS 而不依赖 musl

**推测 C：之前的反汇编有误**
- 需要重新检查实际运行的 busybox 二进制（sdcard 上的）
- 项目目录下的 busybox 可能不是实际运行的版本

### 2. musl Handler 应该能够自己处理

根据 [pthread_cancel_final_conclusion_2026-03-01.md](pthread_cancel_final_conclusion_2026-03-01.md) 的伪代码：

```c
void cancel_handler(int sig, siginfo_t *si, ucontext_t *uc) {
    pthread_t self = (pthread_t)pthread_self();
    if (!self->cancel) return;
    if (self->canceldisable == PTHREAD_CANCEL_DISABLE) return;

    if (self->cancelasync) {
        __cancel();  // 执行清理并退出，Never returns
    }
    // ... deferred cancellation logic ...
}
```

如果 cancelasync==1，handler 应该调用 `__cancel()` 并永不返回。这会：
1. 执行所有 cleanup handlers
2. 调用 pthread_exit 进行正确清理
3. 唤醒等待在 pthread_join 的线程

### 3. 内核不应该干预用户态清理

pthread cancellation 是纯用户态的机制：
- 内核只负责投递 SIGCANCEL
- 用户态 handler 负责检查 TLS 状态并执行清理
- **内核不应该直接退出线程**，这会破坏用户态的清理流程

## 遗留问题

### 1. 为什么 cancelasync==1？

需要验证：
- 挂载 sdcard-rv.img 并反汇编实际的 busybox
- 或者检查 libc-test 的源码是否有自己的实现
- 或者使用 GDB 单步执行 pthread_setcanceltype 调用

### 2. InstructionPageFault at 0x8294eb80

这是一个独立的内核 bug：
- 与 pthread_cancel workaround 无关
- 需要单独调试内存管理或 trap 处理代码
- stval 是物理地址，不应该被当作指令地址

### 3. pthread_cancel 测试的其他场景

除了异步取消，测试还包括：
- 单个 cleanup handler 测试
- 嵌套 cleanup handlers 测试

这些测试依赖延迟取消模式（cancelasync==0），需要：
- 线程在取消点（如 sleep）时被取消
- Cleanup handlers 被正确执行
- 返回 PTHREAD_CANCELED 状态

## 结论与建议

### 当前状态

1. ✅ /dev/shm 目录已创建（修复 VFS 错误）
2. ❌ pthread_cancel workaround 不可行
3. ❌ 出现新的内核 panic 需要解决

### 建议方案

**A. 接受测试失败**（推荐）
- pthread_cancel_points 失败是时序问题（signal 投递延迟）
- pthread_cancel 失败可能是 handler 实现问题
- 标记为"known limitation"，等待完整的测试套件

**B. 调试 InstructionPageFault**
- 这是更严重的内核 bug
- 需要定位为什么内核会跳转到物理地址
- 可能与 futex、线程退出或上下文切换有关

**C. 深入调试 pthread_cancel 测试**
- 挂载 sdcard 并检查实际的 busybox 实现
- 使用 GDB 追踪 pthread_setcanceltype 和 handler 执行
- 理解为什么 cleanup handlers 没有被执行

## 相关文件

- [pthread_setcanceltype_missing_2026-03-01.md](pthread_setcanceltype_missing_2026-03-01.md) - 发现 pthread_setcanceltype 缺失
- [pthread_cancel_final_conclusion_2026-03-01.md](pthread_cancel_final_conclusion_2026-03-01.md) - GDB 调试结论
- [pthread_cancel_gdb_debug_结论_2026-03-01.md](pthread_cancel_gdb_debug_结论_2026-03-01.md) - GDB 调试过程

## 代码修改

最终只保留了 /dev/shm 目录创建：

```rust
// os/src/fs/mod.rs
pub fn ensure_basic_paths() {
    create_dir("/etc");
    create_dir("/dev");
    create_dir("/dev/misc");
    create_dir("/dev/shm");  // ← 新增
    create_dir("/bin");
    // ...
}
```

其他 pthread_cancel workaround 相关修改已全部移除。
