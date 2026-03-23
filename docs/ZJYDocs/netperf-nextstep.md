# netperf 调试进展与下一步计划

**日期**: 2026/3/23

---

## 1. 当前成果

### iperf3: 全部通过 (musl + glibc, 6/6)

| 测试 | musl | glibc |
|------|------|-------|
| BASIC_UDP | 47.6 Mbits/sec | 39.8 Mbits/sec |
| BASIC_TCP | 27.2 Mbits/sec | 27.1 Mbits/sec |
| PARALLEL_UDP (-P 5) | 76.9 Mbits/sec | 66.7 Mbits/sec |
| PARALLEL_TCP (-P 5) | 71.5 Mbits/sec | 68.7 Mbits/sec |
| REVERSE_UDP (-R) | 41.6 Mbits/sec | 34.7 Mbits/sec |
| REVERSE_TCP (-R) | 26.9 Mbits/sec | 28.7 Mbits/sec |

### netperf: UDP_STREAM 通过, 后续卡死

- UDP_STREAM: ~231 Mbits/sec sender, ~20.6 Mbits/sec receiver
- TCP_STREAM 及后续 4 个测试: 未到达

---

## 2. netperf 测试架构

netperf 测 5 种网络性能指标, 全部在 127.0.0.1 loopback 上:

| 测试 | 含义 |
|------|------|
| UDP_STREAM | 单向 UDP 吞吐量 (持续 1 秒灌包) |
| TCP_STREAM | 单向 TCP 吞吐量 |
| UDP_RR | UDP 请求-响应延迟 (ping-pong) |
| TCP_RR | TCP 请求-响应延迟 |
| TCP_CRR | TCP 连接-请求-响应-关闭 (每次新建连接) |

### 2.1 架构

```
testcode.sh (busybox shell, pid=2)
  |
  +-- ./netserver -D -L 127.0.0.1 -p 12865 &    (pid=3, 后台)
  |     +-- fork child (pid=5, 处理连接)
  |
  +-- run_netperf UDP_STREAM ...                  (前台)
  |     +-- ./netperf -H 127.0.0.1 -t UDP_STREAM -l 1  (pid=4)
  |
  +-- run_netperf TCP_STREAM ...
  ...
```

每个测试靠 `setitimer(ITIMER_REAL, 1s)` -> SIGALRM 来在 1 秒后结束数据传输。

### 2.2 控制通道协议

netperf client 和 netserver 之间有一个 TCP 控制通道:
1. client connect -> server accept
2. client 发送测试参数
3. server 创建数据通道 (TCP/UDP)
4. 数据传输 (1 秒)
5. SIGALRM 停止传输
6. server 通过控制通道发送结果 (`sendto(ctrl_fd, results)`)
7. client 通过控制通道接收结果 (`recv(ctrl_fd)`)
8. client `shutdown(ctrl_fd, SHUT_WR)` + `recv()` 等 server 关闭
9. client 的 `shutdown_control()` 有 60 秒超时

---

## 3. 已修复的问题

### 3.1 setitimer/SIGALRM (核心)

netperf 依赖 `setitimer(ITIMER_REAL)` 来计时. 之前 syscall 103 未实现返回 ENOSYS, 导致 netperf 无限发包不会停止.

**修复**:
- PCB 添加 `itimer_real_expire_ms` / `itimer_real_interval_ms`
- `sys_setitimer` / `sys_getitimer` 实现
- `check_timer()` 中遍历所有进程, 到期时设 `signal_pending |= SIGALRM`

### 3.2 loopback UDP demux (iperf parallel)

同端口多 UDP socket 的数据分发. 详见 `iperf3-网络测试适配.md` 3.12 节.

### 3.3 TCP write/shutdown loopback flush

TCP 发送数据后立即 poll loopback, 确保对端能及时收到.

---

## 4. 当前卡死的精确分析

### 4.1 时间线 (QEMU monitor + SYSCALL log 确认)

```
t=0s    shell (pid=2) fork -> pid=3 (netserver), fork -> pid=4 (netperf)
t=1s    SIGALRM 触发, netperf + netserver 停止发数据
t=2s    netserver sendto(ctrl_fd=6, results=656 bytes)
        netperf  recvfrom(ctrl_fd=5, 656 bytes) -> 打印结果
        netperf  shutdown(ctrl_fd=5, SHUT_WR)
        TCP: client FIN-WAIT-1 -> FIN-WAIT-2, server ESTABLISHED -> CLOSE-WAIT
t=62s   netperf shutdown_control select(60s timeout) 超时
        打印 "shutdown_control: no response received  errno 28"
        调用 exit(1) -> exit_group(1)
t=62s+  [WARN] [exit_group] pid=4 name=netperf code=1   <-- 确认调了
t=???   shell 应该 waitpid 收割 pid=4, 但永远没发生
```

### 4.2 根因: busybox shell 卡在用户态

**关键发现**: busybox shell (pid=2) 在 fork netperf (pid=4) 后:
1. 最后一个 syscall: `sigprocmask(SIG_SETMASK, old_mask)` -- 恢复信号 mask
2. **之后再也没有任何 syscall** -- 永远不调 `wait4` / `waitpid`
3. QEMU monitor 确认 CPU 在 `sys_waitpid` 里 -- 但那是 **initproc** (pid=1) 的 waitpid, 不是 shell 的

Shell 卡在 **用户态代码** -- `sigprocmask` 返回后, 还没进入 `waitpid` syscall 就永远停了.

### 4.3 可能原因

1. **信号处理死循环**: `sigprocmask` 解除信号阻塞后, pending 的 SIGCHLD (from pid=3 netserver 的子进程退出) 被立即投递. signal trampoline 跳到 handler, handler 返回后 `rt_sigreturn`, 但如果 ucontext 恢复有 bug, 可能导致 PC 回到 `sigprocmask` 而非原来的下一条指令, 形成无限循环.

2. **SIGCHLD handler 问题**: busybox ash 注册了 SIGCHLD handler (sigaction signum=17). 如果 handler 内部调 `waitpid(-1, WNOHANG)` 但我们的 WNOHANG 行为不正确 (比如不返回 0 而是挂住), shell 就卡在 handler 里.

3. **信号 mask 不正确**: `sigprocmask` 设置了错误的 mask, 导致后续所有信号都被阻塞, shell 永远收不到 SIGCHLD 来 wake 自己.

4. **内核态 bug**: `rt_sigreturn` 恢复 context 时 sepc 被错误修改.

---

## 5. 下一步计划

### 5.1 调试 shell 卡死 (高优先级)

这是 netperf 所有后续测试被阻塞的根因. 需要:

1. **GDB 精确定位**: 安装 `riscv64-unknown-elf-gdb`, 在 shell 的 `sigprocmask` 返回后设断点, 单步跟踪用户态执行流. 关注 sepc 是否被正确恢复.

```bash
# 安装 GDB (如果还没有)
brew install riscv-gnu-toolchain  # 确保包含 gdb

# 或者通过 QEMU monitor 持续采样 sepc
while true; do echo "info registers" | nc -U /tmp/qemu-monitor.sock | grep sepc; sleep 0.1; done
```

2. **SIGCHLD 投递追踪**: 在 `handle_signals` 中对 SIGCHLD 添加 warn 日志, 确认是否有信号被投递, handler 地址是否正确.

3. **rt_sigreturn 验证**: 在 `sys_rt_sigreturn` 中 log 恢复的 sepc, 对比 sigprocmask 返回时的原始 sepc, 确认 context 恢复正确.

4. **WNOHANG 验证**: 检查 busybox SIGCHLD handler 是否调 `waitpid(-1, WNOHANG)`, 以及我们的 WNOHANG 实现是否正确返回 0 (无 zombie) 而非阻塞.

### 5.2 netperf TCP 控制通道 (中优先级)

即使 shell 卡死问题修复, `shutdown_control` 的 60 秒超时说明 **server 从未关闭控制 TCP 连接**. 需要排查:

1. server (pid=5) 在 `sendto(ctrl_fd, results)` 后做了什么
2. server 是否正确处理了 client 的 FIN (CLOSE-WAIT -> 应该 close)
3. smoltcp loopback TCP 数据投递是否有延迟

### 5.3 judge 评分验证 (低优先级)

iperf judge 正则 `^\[\s*[56SUM]*]` 可能不匹配我们的 stream ID `[  7]`. 需要在 Docker 评测环境中验证实际得分.

### 5.4 其他测试套件

- glibc-netperf: 待 musl 通过后测试
- lmbench: 依赖类似的网络/timer 基础设施, 可复用
- cyclictest: 实时性测试, 依赖 clock_nanosleep

---

## 6. 技术债务

| 项目 | 状态 | 影响 |
|------|------|------|
| TCP MSS = 0 | getsockopt(TCP_MAXSEG) 返回 0 | "nonsense TCP MSS 0" 警告 |
| UDP 丢包 ~35% | loopback 上不应丢包 | 吞吐量评分偏低 |
| suspend poll_net 性能 | 已回退 | 需要更轻量的方案 |
| setitimer debug log | 已加入 | 提交前需移除 |
| exit_group debug log | 已加入 | 提交前需移除 |
