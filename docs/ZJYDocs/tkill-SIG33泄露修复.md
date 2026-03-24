# tkill SIG33 泄露修复——pthread_cancel 杀死父进程链

**日期**: 2026/3/25

## 罪魁祸首

`sys_tkill(tid, signum)` 实现中，先用 `pid2process(tid)` 做全局 PID 查找，再在匹配进程的 tasks 中搜索线程。当 musl 的 `pthread_cancel` 调用 `tkill(tid=2, SIGCANCEL=33)` 时，`pid2process(2)` 找到了 **busybox（PID=2）** 而非当前进程的 tid=2 线程，导致 SIG33 被误投到父进程，整条进程链被杀死。

## 背景知识

### musl 的 pthread_cancel 机制

POSIX 线程取消（`pthread_cancel`）在 musl 中通过实时信号实现：

- **SIGCANCEL = SIG33**（`__SIGCANCEL`）：musl 内部专用信号，不对外暴露
- 调用 `pthread_cancel(thread)` 时，musl 向目标线程发送 `tkill(tid, 33)`
- 目标线程在 cancellation point 检查到取消请求后执行清理并退出
- `entry-static.exe` 的 `pthread_cancel_points` 测试会注册 SIG33 handler，然后多次 `tkill(tid, 33)` 给工作线程

### rCore-Lab 的 TID 模型

rCore-Lab 使用**进程内 TID**（0, 1, 2...）而非 Linux 的全局唯一 TID。这意味着不同进程可能有相同数值的 tid：

```
pid=1 (initproc):   tid=0
pid=2 (busybox sh): tid=0
pid=4 (test binary): tid=0 (主线程), tid=1 (工作线程A), tid=2 (工作线程B)
```

当 pid=4 调用 `tkill(tid=2, 33)` 时，期望发给自己进程的 tid=2 线程。

## 如何发现问题

### 第一步：跑全量测试，观察崩溃模式

```bash
SINGLE_TEST=tmp-libctest LOG=SYSCALL timeout 120 bash run.sh -f sdcard-rv.img -t rv > pthread1.log 2>&1
```

日志末尾：
```
Pass!
========== END entry-static.exe pthread_cancel_points ==========
[WARN] [exit_group] pid=3 name=runtest.exe code=1
[INFO] [exit] pid=3 tid=0 name=runtest.exe code=1
...
[INFO] [handle_signals] pid=2 tid=0 sig33 ... task_pending=SIG33
[WARN] [signal] pid=2 name=busybox default handler for signal 33 -> terminate
...
[WARN] [signal] pid=1 name=initproc default handler for signal 33 -> terminate
[kernel] Panicked
```

**关键观察**：pid=2（busybox sh）收到了 SIG33，但 SIG33 是 musl 内部信号，busybox 没有注册 handler，被默认 terminate 处理杀死。这不可能是正常行为——SIG33 只应该在 pid=4 的线程之间传递。

### 第二步：追踪 SIG33 的来源

搜索日志中所有 SIG33 相关事件：

```bash
strings pthread1.log | grep -E "sig33|SIG33|signum=33|tkill.*33"
```

输出：
```
kernel:pid[4] sys_sigaction signum=33        ← pid=4 注册 handler ✓
kernel:pid[4] sys_tkill tid=1 signum=33      ← 发给 tid=1 ✓
kernel:pid[4] sys_tkill tid=1 signum=33
kernel:pid[4] sys_tkill tid=1 signum=33
kernel:pid[4] sys_tkill tid=1 signum=33
kernel:pid[4] sys_tkill tid=2 signum=33      ← 发给 tid=2 ← 关键！
kernel:pid[4] sys_tkill tid=2 signum=33
kernel:pid[4] sys_tkill tid=2 signum=33
[handle_signals] pid=2 tid=0 sig33 ... task_pending=SIG33  ← pid=2 收到了！
```

**关键线索**：pid=4 调用 `tkill(tid=2, 33)` 后，**pid=2**（busybox，完全不同的进程）收到了 SIG33。tid=2 和 PID=2 发生了混淆。

### 第三步：审查 sys_tkill 实现

查看 `os/src/syscall/process.rs` 中的 `sys_tkill`：

```rust
pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    let tid = tid as usize;
    // BUG: 先用 tid 当作 PID 全局查找！
    if let Some(process) = pid2process(tid) {
        let process_pid = process.getpid();
        let inner = process.inner_exclusive_access();
        let ret = send_signal_to_task_from_list(tid, process_pid, &inner.tasks, flag);
        if ret == 0 { return 0; }  // 找到就返回！
    }
    // 然后才搜索当前进程
    let process = current_process();
    ...
}
```

执行路径分析（pid=4 调用 `tkill(tid=2, 33)`）：

1. `pid2process(2)` → 找到 **PID=2 的 busybox 进程**
2. `send_signal_to_task_from_list(2, 2, busybox.tasks, SIG33)`
3. `task_matches_linux_tid(process_pid=2, task, target_tid=2)`
   - busybox 的 tid=0 线程：`internal_tid == 0 && process_pid == target_tid` → `0 == 0` 不成立... 等等
   - 实际是 `internal_tid == 0 && process_pid == 2 == target_tid` → **true**！
   - 因为 `task_matches_linux_tid` 的逻辑是：如果是主线程（tid=0），则 PID 也是它的有效 tid
4. 于是 SIG33 被投递到 busybox 的 tid=0 主线程
5. `ret == 0`，直接返回，**不再搜索当前进程**

### 第四步：理解 task_matches_linux_tid 的映射逻辑

```rust
fn task_matches_linux_tid(process_pid, task, target_tid) -> bool {
    internal_tid == target_tid ||
    (internal_tid == 0 && process_pid == target_tid)
}
```

这个函数的本意是：主线程（tid=0）既匹配 tid=0 也匹配 PID。这在进程内是正确的（Linux 中线程组 leader 的 tid 等于 PID），但与 `pid2process` 全局查找结合后，就产生了跨进程误匹配。

### 第五步：确认修复方向

Linux 的 `tkill` 系统调用只能向**同一进程**的线程发送信号。即使传入的 tid 碰巧等于另一个进程的 PID，也不应该跨进程投递。

修复方案：**调换搜索顺序**——先在当前进程搜索，找到就返回；找不到再 fallback 到全局 pid2process（用于跨进程场景，虽然实际上很少用到）。

```rust
// 修复后
pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    // 1. 先搜索当前进程（最常见场景：pthread_cancel）
    let process = current_process();
    let ret = send_signal_to_task_from_list(tid, process_pid, &inner.tasks, flag);
    if ret == 0 { return 0; }

    // 2. Fallback：全局 PID 查找（跨进程 tkill）
    if let Some(process) = pid2process(tid) {
        ...
    }
}
```

### 第六步：验证修复

修复后重新运行 pthread 测试：

```bash
SINGLE_TEST=tmp-libctest LOG=SYSCALL timeout 120 bash run.sh -f sdcard-rv.img -t rv > pthread2.log 2>&1
```

结果：**11/11 全部 Pass**，无 SIG33 泄露，正常到达 `=== All tests completed ===`。

## 根因总结

| 组件 | 问题 |
|------|------|
| `sys_tkill` 搜索顺序 | 先 `pid2process(tid)` 全局查找，后搜当前进程 |
| `task_matches_linux_tid` | 主线程同时匹配 tid=0 和 PID，导致跨进程误匹配 |
| 进程内 TID 模型 | tid 值可与其他进程的 PID 冲突（如 tid=2 vs PID=2） |

三个因素叠加：`tkill(2, 33)` → `pid2process(2)` 找到 busybox → busybox 主线程的 PID=2 匹配 target=2 → SIG33 投给 busybox → 整条进程链被杀。

## 影响范围

- 所有使用 `pthread_cancel` 的测试（pthread_cancel, pthread_cancel_points, pthread_cancel_sem_wait, pthread_exit_cancel 等）
- 任何创建 2+ 线程并使用 `tkill` 的多线程程序，当线程 tid 恰好等于某个活跃进程的 PID 时会触发
- 此 bug 只存在于 hwt-ltp1 分支的旧 `sys_tkill` 实现中，主分支（loongarch-net）的实现已经直接按索引查找当前进程的 tasks，不受此影响

## 附：调试技巧

1. **SIG32/SIG33 是 musl 内部信号**：如果在非 musl 进程（busybox, initproc 等）的日志中看到它们，几乎肯定是信号投递出了问题
2. **用 `strings log | grep sig33` 追踪信号流向**：对于二进制日志，`strings` + `grep` 是快速定位的好方法
3. **关注 `task_pending` vs `proc_pending`**：`task_pending=SIG33` 说明是通过 `tkill`（线程级）投递的，而非 `kill`（进程级）
4. **通过 `initcode.rs` 写临时脚本到 `/tmp/`**：无需修改 sdcard 镜像即可自定义测试用例，利用 `write_embedded_elf` 在运行时写入任意文件
