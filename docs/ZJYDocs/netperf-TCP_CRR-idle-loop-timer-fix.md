# netperf TCP_CRR 修复——空闲循环定时器中断盲区

**日期**: 2026/3/23

---

## 1. 罪魁祸首

**TCP_CRR 测试永远卡死的根因是：当所有进程都阻塞在内核态系统调用中时，RISC-V 的 `sstatus.SIE`（Supervisor Interrupt Enable）始终为 0，导致定时器中断永远无法触发，`check_timer()` 永远不被调用，服务端的 SIGALRM 永远无法投递。**

这是一个 rCore 调度器层面的架构缺陷，不是网络模块本身的 bug。

---

## 2. 问题发现过程

### 2.1 初始现象

netperf 5 个测试中前 4 个（UDP_STREAM、TCP_STREAM、UDP_RR、TCP_RR）全部 PASS，唯独 TCP_CRR 卡死（exit=124，被 `timeout` 杀掉）。

### 2.2 日志分析：定位到 SIGALRM

通过 `LOG=INFO` 运行，搜索关键日志：

```bash
strings /tmp/netperf1.log | grep -aE "SIGALRM|itimer|setitimer"
```

发现：
- **客户端 (pid=4)** 的 SIGALRM 正常触发（5 次，每个测试各 1 次）
- **服务端 (pid=9)** 设置了 `setitimer(val=5000ms, expire_at=13202ms)`，但 **SIGALRM 从未触发**

```
[setitimer] pid=9 val=5000ms int=0ms expire_at=13202ms now=8202ms  ← 设置了
# ... 无 pid=9 SIGALRM fired 日志 ...
```

测试运行了 120 秒才被 timeout 杀掉，服务端的 5 秒定时器应该早就到期了，但日志中没有任何 `pid=9 SIGALRM fired` 记录。

### 2.3 逐步排查：check_timer() 根本没被调用

在 `check_timer()` 函数顶部和 `check_itimers()` 入口分别添加 `warn!` 日志，重新运行：

```rust
// timer.rs check_timer() 顶部
log::warn!("[timer] check_timer count={} time_ms={}", count, current_ms);

// check_itimers() 入口
log::warn!("[itimer-enter] t={} dbg_count={}", current_ms, dbg_count);
```

**结果：在 t=9233ms（客户端 SIGALRM 触发并完成后续操作）之后，这两条日志都不再出现。** 这意味着 `check_timer()` 函数本身在客户端操作完成后就再也没被调用过。

### 2.4 根因分析：RISC-V 中断使能机制

`check_timer()` 只在定时器中断处理函数中被调用（`handle_user_time_interrupt` 和 `kernel_interrupt_dispatch`）。但定时器中断能否触发取决于 `sstatus.SIE`：

**RISC-V S 模式中断规则**：
- 当 trap（系统调用、中断）发生时，硬件自动将 `SIE` 保存到 `SPIE`，并清除 `SIE = 0`
- 内核代码运行在 `SIE = 0` 状态下，定时器中断不会被响应
- 当通过 `sret` 返回用户态时，`SPIE` 恢复到 `SIE`，中断重新使能

**正常流程**（前 4 个测试为什么正常）：

```
用户态执行 → SIE=1 → 定时器中断触发 → trap → SIE=0 → 处理中断 →
check_timer() → sret → SIE=1 → ... 循环
```

前 4 个测试中，客户端在数据发送循环中频繁地在 "用户态 → 系统调用 → 返回用户态" 之间切换，每次回到用户态时 SIE=1，定时器中断有机会触发。

**TCP_CRR 卡死时的流程**：

```
t=9233ms: 客户端完成最后操作，阻塞在 recvfrom (控制通道)
          服务端一直阻塞在 accept (数据端口)
          shell 阻塞在 waitpid
          initproc 阻塞在 waitpid

所有进程 → 都在内核态系统调用循环中 → SIE=0
         → suspend_current_and_run_next() 上下文切换
         → 空闲循环 run_tasks() 继续运行
         → 但 SIE 从内核态继承为 0
         → 定时器中断永远不触发
         → check_timer() 永远不调用
         → check_itimers() 永远不检查
         → 服务端 SIGALRM 永远不投递
```

关键在于 `run_tasks()` 空闲循环中 SIE 的传播：

```
1. 任务在内核态（SIE=0）调用 suspend_current_and_run_next()
2. 上下文切换保存/恢复 callee-saved 寄存器，但不保存 sstatus
3. 切换到 idle 循环时，SIE 仍为 0（继承自被切出的内核代码）
4. idle 循环的 PROCESSOR.exclusive_access() 保存 SIE=0，禁用中断
5. drop(processor) 恢复 SIE=0
6. 整个 idle 循环始终 SIE=0
```

---

## 3. 修复方案

### 3.1 核心修复：idle 循环中开启中断窗口

**文件**: `os/src/task/processor.rs`

在 `run_tasks()` 循环顶部，先 enable 再 disable 中断，创造一个极短的中断响应窗口：

```rust
pub fn run_tasks() {
    loop {
        // Enable interrupts briefly so pending timer interrupts can fire.
        // This is critical: when all tasks are blocked in kernel-mode
        // syscalls (e.g., accept, recv, waitpid), sstatus.SIE remains 0
        // and timer interrupts are never taken. Without this, SIGALRM
        // from setitimer can never be delivered.
        arch::enable_interrupts();
        arch::disable_interrupts();

        let mut processor = PROCESSOR.exclusive_access();
        // ... 原有调度逻辑不变 ...
    }
}
```

**原理**：RISC-V 的定时器中断是电平触发的。一旦硬件计时器到期，中断请求持续挂起直到被确认。因此，即使 `enable_interrupts()` 和 `disable_interrupts()` 之间只有一条指令的时间窗口，CPU 也会立即响应挂起的中断，跳转到 `kernel_interrupt_dispatch` → `check_timer()` → `check_itimers()` → 投递 SIGALRM。

### 3.2 防御性修复：check_itimers 使用 try_lock

**文件**: `os/src/timer.rs`

将 `check_itimers` 中的 `process.inner_exclusive_access()` 改为 `process.try_inner_exclusive_access()`：

```rust
fn check_itimers(current_ms: usize) {
    let procs = pid2process_snapshot();
    for (_pid, process) in procs {
        // Use try_inner_exclusive_access to avoid deadlocking if the timer
        // interrupt fires while someone holds this process's inner lock.
        let mut inner = match process.try_inner_exclusive_access() {
            Some(inner) => inner,
            None => continue,  // 下次 tick 再检查
        };
        // ... 检查定时器并投递 SIGALRM ...
    }
}
```

这是防御性措施：如果定时器中断恰好在某段代码持有进程内部锁时触发，`try_lock` 不会死锁，而是跳过这个进程，在下一个 tick 重试。

---

## 4. 验证结果

### 4.1 netperf 5/5 全部通过

| 测试 | 状态 | 说明 |
|------|------|------|
| UDP_STREAM | PASS | 单向 UDP 吞吐量 |
| TCP_STREAM | PASS | 单向 TCP 吞吐量 |
| UDP_RR | PASS | UDP 请求-响应延迟 |
| TCP_RR | PASS | TCP 请求-响应延迟 |
| **TCP_CRR** | **PASS** | TCP 连接-请求-响应 (新修复) |

修复后日志中可以看到服务端 SIGALRM 正确触发：

```
[itimer] pid=4 SIGALRM fired, expire=9412 now=9422    ← 客户端
[itimer] pid=9 SIGALRM fired, expire=13418 now=13422   ← 服务端 (新!)
====== netperf TCP_CRR end: success ======
```

### 4.2 回归测试——无回归

**iperf3 (musl) 6/6 PASS**:

| 测试 | 状态 |
|------|------|
| BASIC_UDP | PASS |
| BASIC_TCP | PASS |
| PARALLEL_UDP | PASS |
| PARALLEL_TCP | PASS |
| REVERSE_UDP | PASS |
| REVERSE_TCP | PASS |

**basic (musl) 全部 PASS**，无 ERROR/WARN/PageFault。

---

## 5. 当前网络模块架构总览

### 5.1 整体架构

```
os/src/net/
├── mod.rs           # 全局网络栈 (NetStack)，smoltcp 初始化
├── socket_file.rs   # SocketFile: File trait 实现，poll/read/write
└── syscall.rs       # 网络系统调用: socket/bind/listen/accept/connect/...
```

底层依赖：
- **smoltcp**: 用户态 TCP/IP 协议栈（内核中以库的形式使用）
- **VirtIO-Net**: QEMU 虚拟网卡驱动（外部网络）
- **Loopback**: smoltcp 内置的 loopback 设备（127.0.0.1 流量）

### 5.2 已实现的网络系统调用

| syscall | 函数 | 状态 | 说明 |
|---------|------|------|------|
| socket | `sys_socket` | 完整 | AF_INET, SOCK_STREAM/DGRAM, NONBLOCK/CLOEXEC |
| bind | `sys_bind` | 完整 | TCP 端口记录 + UDP 实际绑定 |
| listen | `sys_listen` | 完整 | TCP LISTEN 状态，backlog 忽略 |
| accept | `sys_accept` | 完整 | socket 交换模型 + EINTR 支持 |
| connect | `sys_connect` | 完整 | TCP 阻塞连接 + UDP 记录目标 + EINTR |
| sendto | `sys_sendto` | 完整 | TCP 阻塞发送 + UDP loopback inject + EINTR |
| recvfrom | `sys_recvfrom` | 完整 | TCP 阻塞接收 + EOF 检测 + UDP + EINTR |
| shutdown | `sys_shutdown_socket` | 完整 | TCP close + loopback FIN flush |
| getsockname | `sys_getsockname` | 完整 | |
| getpeername | `sys_getpeername` | 完整 | |
| setsockopt | `sys_setsockopt` | 桩 | 接受常见选项但不真正生效 |
| getsockopt | `sys_getsockopt` | 部分 | SO_ERROR/SNDBUF/RCVBUF/TCP_MAXSEG/TCP_INFO |
| socketpair | - | 未实现 | 返回 EOPNOTSUPP |
| sendmsg | - | 未实现 | 返回 EOPNOTSUPP |
| recvmsg | - | 未实现 | 返回 EOPNOTSUPP |

### 5.3 已实现的网络特性

1. **双网络接口**：VirtIO-Net（10.0.2.15/24）+ Loopback（127.0.0.1/8）
2. **Loopback TCP 即时投递**：write/sendto 后立即 poll loopback 4 轮，确保对端能及时收到
3. **Loopback UDP demux**：`loopback_udp_inject` 按 connected/wildcard 优先级分发，支持 iperf3 parallel UDP
4. **TCP poll EOF 检测**：`was_connected` guard 区分"未连接"和"已断开"，避免 iperf3 回归
5. **阻塞 syscall EINTR**：accept/connect/sendto/recvfrom 的 yield 循环中检查 pending signal
6. **SOCK_NONBLOCK**：read/write 中支持非阻塞返回
7. **Socket Drop 优雅关闭**：`close()` 发送 FIN + loopback flush，而非 `abort()`

### 5.4 待实现 / 已知不足

| 项目 | 优先级 | 说明 |
|------|--------|------|
| SO_REUSEADDR 实际实现 | 低 | 当前只是桩，不检查端口冲突 |
| TCP TIME_WAIT 管理 | 低 | Drop 立即移除 socket，无 TIME_WAIT 等待 |
| sendmsg/recvmsg | 中 | 一些应用依赖 scatter/gather I/O |
| socketpair | 低 | Unix domain socket 语义 |
| UDP 丢包率 | 中 | loopback 上约 35% 丢包（buffer 配置问题） |
| IPv6 | 低 | 目前只支持 AF_INET (IPv4) |
| 外部网络测试 | 中 | 目前测试全在 loopback，VirtIO-Net 路径未充分验证 |

---

## 6. 非网络模块发现的 Bug 及修复

### 6.1 Bug #1: wait4 不支持 WNOHANG（已在前序 commit 修复）

**现象**: netperf 第一个测试完成后 busybox shell 永远卡死。

**根因**: `syscall/mod.rs` 中 `SYSCALL_WAITPID` dispatch 忽略了 `options` 参数（`args[2]`），导致 WNOHANG 标志丢失。busybox 的 SIGCHLD handler 内部调用 `waitpid(-1, WNOHANG)` 时，内核按阻塞模式执行，如果没有 zombie 子进程就永远阻塞。

**修复**: 传递 options 参数：
```rust
SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32, args[2] as i32),
```

**教训**: WNOHANG 看似只影响"非阻塞等待"，但实际上 busybox 整个 job control 和 SIGCHLD handler 都依赖它。

### 6.2 Bug #2: waitpid 阻塞循环不响应信号（已在前序 commit 修复）

**现象**: waitpid 阻塞时收到信号不返回 EINTR。

**修复**: 在 `sys_waitpid` 的 yield 循环中添加 `has_pending_signal()` 检查，返回 EINTR。

### 6.3 Bug #3: 内核态定时器中断盲区（本次修复）

**现象**: TCP_CRR 的服务端 SIGALRM 永远不触发。

**根因**: 空闲循环 `run_tasks()` 中 `sstatus.SIE` 始终为 0，定时器中断永远不被响应。

**修复**: 在 idle 循环顶部 `enable_interrupts(); disable_interrupts();` 开窗口。详见本文第 3 节。

**教训**: rCore 的协作式调度与 Linux 的抢占式调度在信号投递时机上有本质差异。rCore 中，当所有任务都在内核态 busy-wait 时，不仅 `handle_signals()` 不会被调用（已知），连 `check_timer()` 也不会被调用（新发现）。修复 EINTR 让信号能从 syscall 循环中传播出去还不够，还必须确保定时器中断本身能够触发。

---

## 7. 调试经验总结

### 经验 6：定时器中断不触发 ≠ 定时器不工作

当 `check_timer()` 的日志完全消失时，问题不在定时器逻辑本身，而在于 **定时器中断本身没有被 CPU 响应**。这是一个硬件/中断控制层面的问题，需要从 `sstatus.SIE` 的传播链条入手分析。

排查方法：在 `check_timer()` 入口添加 `warn!` 日志，如果日志消失，说明函数根本没被调用，问题在中断层而非 timer 层。

### 经验 7：用减法定位——逐层加日志确认"执行到了哪里"

本次调试的关键手法是逐步缩小范围：
1. 先确认 SIGALRM 没触发（搜 `SIGALRM fired`）
2. 再确认 `check_itimers()` 没执行（入口加日志）
3. 再确认 `check_timer()` 没执行（入口加日志）
4. 最终定位到定时器中断本身没触发

每一步只需添加一行 `warn!` 并重新运行 ~30 秒即可确认，整个排查过程不超过 10 分钟。

### 经验 8：exit code 0 vs 124 的快速判别

```bash
SINGLE_TEST=musl-netperf LOG=ERROR timeout 60 bash run.sh -f sdcard-rv.img -t all > test.log 2>&1
echo "exit=$?"
```

- `exit=0`: 正常完成
- `exit=124`: 超时（卡死了）

---

## 8. 修改文件清单

| 文件 | 修改内容 | 类型 |
|------|----------|------|
| `os/src/task/processor.rs` | `run_tasks()` 循环顶部添加 `enable_interrupts(); disable_interrupts();` | 核心修复 |
| `os/src/timer.rs` | `check_itimers` 使用 `try_inner_exclusive_access` 替代 `inner_exclusive_access` | 防御性修复 |
