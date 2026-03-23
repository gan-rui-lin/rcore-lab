# netperf 信号机制重构与调试记录

**日期**: 2026/3/23

---

## 1. 背景

netperf 是一个经典的网络性能测试工具，测试 5 种指标（UDP_STREAM、TCP_STREAM、UDP_RR、TCP_RR、TCP_CRR）。在 rCore-lab 中，iperf3 已全部 6/6 通过，但 netperf 在 UDP_STREAM 完成后 shell 卡死，后续测试无法执行。

netperf 的控制流依赖：
- `setitimer(ITIMER_REAL)` → SIGALRM 计时（1 秒后停止数据传输）
- TCP 控制通道进行 client/server 结果交换
- `shutdown_control()` 中 `select(60s)` 等待 server 关闭连接
- busybox shell 的 SIGCHLD handler 通过 `waitpid(-1, WNOHANG)` 收割子进程

## 2. 发现的三个核心 Bug

### 2.1 Bug #1: wait4 不支持 WNOHANG（罪魁祸首）

**现象**: UDP_STREAM 完成后，busybox shell 永远卡住，后续 4 个测试不执行。

**根因分析**:

syscall 260 (`wait4`) 的 Linux 签名是 `wait4(pid, wstatus, options, rusage)`，但内核 dispatch 时只传了前两个参数，**完全忽略了 `options`（args[2]）**：

```rust
// 旧代码
SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32),
```

busybox ash 注册了 SIGCHLD handler（sigaction signum=17）。当 netserver 子进程退出时：

1. SIGCHLD 发送到 shell（pid=2）
2. `sigprocmask(SIG_SETMASK)` 返回后，`handle_signals()` 投递 SIGCHLD
3. trap context 被修改，shell 跳转到 SIGCHLD handler
4. handler 调用 `waitpid(-1, &status, WNOHANG)` —— 即 `wait4(-1, &status, 1, NULL)`
5. 内核忽略 options=1，**按阻塞模式执行**
6. 此时可能无 zombie 子进程 → **waitpid 永远阻塞**
7. shell 卡在 SIGCHLD handler 内部，永远无法返回

**修复**: 传递 options 参数，支持 WNOHANG：

```rust
// 新代码
SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32, args[2] as i32),
```

同时在 waitpid 的 yield 循环中检测 pending signal 返回 EINTR，让信号能被及时投递。

### 2.2 Bug #2: TCP poll 不报告 EOF 可读（控制通道卡死）

**现象**: 4 个前置测试的 `shutdown_control` 60 秒超时，netperf 返回 exit(1)。

**根因分析**:

netperf 协议流程：
1. server `send_response()` → 结果通过 TCP 控制通道发送
2. client `recv()` 接收结果 → 打印
3. client `shutdown(ctrl_fd, SHUT_WR)` → 发送 FIN
4. server `recv_request()` 中 `select(ctrl_fd)` 等待可读 → `recv()` 返回 0 (EOF)
5. server `close(server_sock)` → 发送 FIN
6. client `shutdown_control()` 中 `select(ctrl_fd)` 等待可读 → 返回

问题出在步骤 4：server 的 `select()` 通过我们的 `sys_pselect6` → `SocketFile::poll()` 检查可读性。旧代码：

```rust
// 旧代码：只在有数据时报告 POLLIN
if events.contains(PollEvents::POLLIN) && socket.can_recv() {
    result |= PollEvents::POLLIN;
}
```

当 client 发送 FIN 后，server 的 socket 进入 CLOSE_WAIT 状态：`can_recv()` 为 false（无数据），`may_recv()` 为 false（收到 FIN）。但 **POLLIN 没有被设置**！Linux 语义下，当 `read()` 会立即返回（包括返回 0/EOF），fd 应该报告为可读。

**修复**: 增加 EOF 检测，但仅对已建立连接的 socket：

```rust
let was_connected = !matches!(state, State::Closed | State::Listen | State::SynSent | State::SynReceived);
if events.contains(PollEvents::POLLIN) && (socket.can_recv() || (was_connected && !socket.may_recv())) {
    result |= PollEvents::POLLIN;
}
```

`was_connected` guard 至关重要——未连接的 socket（如刚创建的）在 smoltcp 中 `may_recv() = false`，如果不加 guard 会导致所有未连接 socket 立即报告 POLLIN/POLLHUP，**这直接导致了 iperf3 回归**（iperf3 在连接前 poll socket，误判为连接断开）。

### 2.3 Bug #3: 阻塞网络 syscall 不响应信号（TCP_CRR 卡死）

**现象**: TCP_CRR 测试中，server 的 SIGALRM 永远不生效，控制通道无法完成。

**根因分析**:

TCP_CRR（Connect/Request/Response）每次迭代创建新连接。SIGALRM 通过 `check_timer()` 在定时器中断中设置 `signal_pending`，但 `handle_signals()` 只在 `user_trap_loop` 中调用——**内核态 busy-wait 循环中永远不会调用**。

具体流程：
1. client SIGALRM 在 1 秒后触发，client 停止创建新连接
2. server 在 `accept()` 的 `suspend_current_and_run_next()` 循环中
3. `check_timer()` 设置 server 的 `signal_pending |= SIGALRM`
4. 但 server 永远不回到 `user_trap_loop`，SIGALRM 永远不被投递
5. server 永远不发送结果 → client 永远等不到 → 死锁

**修复**: 在所有阻塞网络 syscall 的 yield 循环中检测 pending signal：

```rust
suspend_current_and_run_next();
if has_pending_signal() {
    return EINTR;
}
```

影响的 syscall：accept、connect、sendto(TCP)、recvfrom(TCP/UDP)。

## 3. 当前测试结果

### netperf (musl)

| 测试 | 状态 | 说明 |
|------|------|------|
| UDP_STREAM | PASS | ~226 Mbits/sec |
| TCP_STREAM | PASS | ~68 Mbits/sec |
| UDP_RR | PASS | request/response 延迟测试 |
| TCP_RR | PASS | request/response 延迟测试 |
| TCP_CRR | FAIL | 卡在 SIGALRM 后控制通道清理 |

### iperf3 (musl) — 无回归

| 测试 | 状态 |
|------|------|
| BASIC_UDP | PASS (41.9 Mbits/sec) |
| BASIC_TCP | PASS (27.2 Mbits/sec) |
| PARALLEL_UDP | PASS (209 Mbits/sec) |
| PARALLEL_TCP | PASS (72.8 Mbits/sec) |
| REVERSE_UDP | PASS (42.3 Mbits/sec) |
| REVERSE_TCP | PASS (29.1 Mbits/sec) |

### basic (musl) — 无回归

全部通过。

## 4. TCP_CRR 未解决问题分析

TCP_CRR 的 EINTR 修复让 server 的 SIGALRM 能中断 accept，但后续控制通道清理仍有问题。从日志看：

1. TCP_CRR 数据循环正常工作（大量 connect/accept/EOF 循环）
2. SIGALRM 正确触发
3. 但之后 server 或 client 在控制通道交互中卡死

可能原因：
- **smoltcp socket 资源耗尽**: TCP_CRR 快速创建/关闭大量 TCP 连接（~每秒数百个），smoltcp 的 TIME_WAIT socket 可能占满 socket pool（MAX_SOCKETS=64）
- **accept 返回 EINTR 后 netserver 的错误处理**: accept 返回 -4 (EINTR) 可能导致 netserver 误判为错误并关闭控制通道（errno 9 = EBADF），从而 client 的 `recv_response` 收到 EBADF
- **TCP 状态机清理不彻底**: 大量 CLOSE_WAIT/TIME_WAIT 状态的 socket 可能影响后续连接

### 下一步

1. 调查 EINTR 从 accept 返回后 netserver 的行为（是否正确重试 accept）
2. 检查 smoltcp socket pool 是否耗尽
3. 考虑在 accept EINTR 时更精细的处理（例如只在有对应信号 handler 时返回 EINTR）
4. TCP_CRR 可能需要 smoltcp 的 TIME_WAIT 快速回收机制

## 5. 调试方法与经验

### 5.1 测试启动命令

```bash
# 杀掉残留 QEMU，运行单个测试套件
pkill -9 -f qemu-system-riscv 2>/dev/null; sleep 1

# netperf 测试（SINGLE_TEST 选择套件，LOG 控制日志级别）
SINGLE_TEST=musl-netperf LOG=ERROR timeout 120 bash run.sh -f sdcard-rv.img -t all > netperf.log 2>&1
echo "exit=$?"

# iperf3 回归测试
SINGLE_TEST=musl-iperf LOG=ERROR timeout 180 bash run.sh -f sdcard-rv.img -t all > iperf.log 2>&1

# basic 回归测试
SINGLE_TEST=musl-basic LOG=ERROR timeout 120 bash run.sh -f sdcard-rv.img -t all > basic.log 2>&1

# 需要更详细的日志时用 INFO 或 SYSCALL
SINGLE_TEST=musl-netperf LOG=INFO timeout 180 bash run.sh -f sdcard-rv.img -t all > netperf-info.log 2>&1
```

**关键点**：
- `timeout` 必须加，否则卡死的测试会占住终端
- exit=0 正常结束，exit=124 超时（说明卡死了）
- `LOG=ERROR` 日志最少，适合快速验证；`LOG=INFO` 能看到网络/信号事件；`LOG=SYSCALL` 会输出每个 syscall（日志巨大，但能精确定位问题）

### 5.2 日志分析技巧

```bash
# 快速看测试结果
grep -a "begin\|end:\|success\|fail\|TEST GROUP\|completed\|All tests" test.log

# 看关键错误
grep -a "ERROR\|WARN\|IllegalInstruction\|PageFault\|SIGKILL\|Panicked" test.log | grep -v SYSCALL

# 跟踪特定进程（如 netserver child pid=9）
grep -a "pid=9" test.log | grep -v SYSCALL

# 看 SIGALRM/timer 事件
grep -a "itimer\|SIGALRM\|setitimer" test.log

# 看 TCP 控制通道关键事件
grep -a "shutdown.*TCP\|EOF\|EBADF\|errno\|shutdown_control\|no response" test.log | grep -v SYSCALL

# 看最后发生了什么（卡死时特别有用）
tail -40 test.log | grep -v SYSCALL
```

### 5.3 调试经验总结

**经验 1：从 netperf 协议理解问题，而非盲目加日志**

netperf 的 5 个测试共用一个架构：`setitimer` 计时 → SIGALRM 停止 → 控制通道交换结果 → `shutdown_control` 清理。理解这个流程后，就能快速判断卡在哪个阶段。比如看到 `shutdown_control: no response received errno 28` 就知道 server 没关闭控制通道，问题在 server 侧。

**经验 2：smoltcp 的 TCP 状态语义和 Linux 不完全一致**

smoltcp 中未连接 socket 的 `may_recv() = false`、`may_send() = false`、`is_open() = false`。如果用这些做 poll 判断而不考虑连接状态，会导致"未连接就报 EOF/POLLHUP"的诡异 bug。必须用 `socket.state()` 做状态过滤。

这个 bug 的表现极其隐蔽：netperf 工作正常（因为 netperf 的 socket 使用模式是 connect 后立刻 poll），但 iperf3 会卡死（因为 iperf3 在 connect 之前就 poll socket）。**改一个 syscall 语义时必须同时跑所有测试套件做回归验证。**

**经验 3：内核态 busy-wait 循环是信号盲区**

rCore 的 `handle_signals()` 只在 `user_trap_loop` 末尾调用。任何在内核态 `suspend_current_and_run_next()` 循环中的 syscall（waitpid、accept、recv、connect、poll 等）都不会投递信号。对于需要被信号中断的阻塞 syscall，必须在 yield 后手动检查 `pending signal` 并返回 EINTR。

这和 Linux 的行为不同——Linux 内核在信号到达时会直接中断阻塞 syscall。rCore 的协作式调度没有这个机制，所以需要每个阻塞点手动检查。

**经验 4：WNOHANG 缺失的影响范围远大于预期**

表面上 WNOHANG 只影响"非阻塞等待子进程"，但实际上 busybox 的整个 job control 和 SIGCHLD handler 都依赖它。缺少 WNOHANG 的症状是"shell 莫名卡死"，而且只在特定条件下触发（需要子进程退出 + SIGCHLD handler + handler 内 waitpid），非常难以复现和定位。

**经验 5：exit code 124 = timeout，看日志尾部**

当测试以 exit=124 结束时，说明是 `timeout` 命令杀掉了 QEMU。此时看 `tail -40 test.log` 能知道最后在做什么。如果最后一行是某个 syscall 的重复日志（如无限循环的 accept 或 recv），就是那个 syscall 卡住了。

## 6. 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `os/src/syscall/mod.rs` | wait4 dispatch 传递 options 参数 |
| `os/src/syscall/process.rs` | sys_waitpid 支持 WNOHANG + EINTR |
| `os/src/net/socket_file.rs` | TCP poll: POLLIN on EOF, POLLHUP with was_connected guard |
| `os/src/net/syscall.rs` | 网络 syscall EINTR、getsockopt TCP_MAXSEG、shutdown flush 增强 |
