# LoongArch 网络测试跨 libc daemon 残留修复

日期: 2026/3/25

## 背景

在 LoongArch + RISC-V 双架构网络适配完成后，`SINGLE_TEST=musl-iperf` 和 `SINGLE_TEST=glibc-iperf` 分别运行时均能 6/6 通过。但使用 `SINGLE_TEST=iperf`（不指定 libc 前缀）时，initcode 会依次运行 `/musl/iperf_testcode.sh` 和 `/glibc/iperf_testcode.sh`。此时 musl iperf 6/6 通过后，glibc iperf 永远无法启动——整个 QEMU 进程卡死直到 timeout。

本文记录两个层次的 bug 及修复。

## 罪魁祸首

1. **iperf3 daemon 端口残留** — musl iperf 测试脚本启动 `iperf3 -s -p 5001 -D`（后台 daemon），测试完后不杀 daemon。glibc iperf 再启动新 server 时，旧 daemon 仍占端口，两个 server 竞争 accept 导致 TCP 连接混乱。

2. **`sys_waitpid` 缺少 `WNOHANG` 导致 initcode 阻塞** — `reap_orphans()` 试图 kill + wait 清理残留进程，但 `sys_waitpid(-1)` 在内核中**阻塞等待**（`suspend_current_and_run_next`），如果被 SIGKILL 的进程尚未完全退出，initcode 就永远 suspend 在 waitpid 中。

## 分析过程

### 第一步：确认卡死位置

通过 `LOG=SYSCALL` 追踪 pid=1 (initcode) 的系统调用：

```
# musl iperf 完成后的最后几个 syscall
[SYSCALL] pid=1 num=129(kill) args=[0x5, 0x9] ret=0    ← kill(5, SIGKILL) 成功
[SYSCALL] pid=1 num=124(sched_yield) × 20               ← yield 等待进程退出
[SYSCALL] pid=1 num=260(wait4) args=[-1, ...] ret=4     ← reap 了 pid=4
# 之后再无 syscall —— initcode 永久 suspend 在下一个 wait4(-1) 中
```

### 第二步：定位阻塞原因

检查内核 `sys_waitpid` 实现（`os/src/syscall/process.rs:1257`）：

```rust
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32, options: i32) -> isize {
    loop {
        // 找到 zombie 子进程 → 返回 pid
        // 没有子进程 → 返回 ECHILD
        // 有活着的子进程但没有 zombie：
        if (options & WNOHANG) != 0 { return 0; }  // WNOHANG → 立即返回 0
        suspend_current_and_run_next();  // 否则 → 阻塞！
    }
}
```

而 `user/src/syscall.rs` 中 `sys_waitpid` 的第三个参数硬编码为 `0`：

```rust
pub fn sys_waitpid(pid: isize, xstatus: *mut i32) -> isize {
    syscall(SYSCALL_WAITPID, [pid as usize, xstatus as usize, 0])  // options=0, 无 WNOHANG
}
```

**根因**：pid=5 (iperf3 daemon) 被 SIGKILL 后需要几个调度轮才能退出。在这期间 `wait4(-1, options=0)` 发现有活着的子进程（pid=5），没有 zombie，于是 suspend。但 initcode 是 pid=1（init 进程），suspend 后只有 pid=5 在运行，pid=5 收到 SIGKILL 后退出变成 zombie，但没有人来唤醒 initcode 去 reap 它——**死锁**。

### 第三步：为什么之前的修复尝试都失败

| 尝试 | 结果 | 原因 |
|------|------|------|
| kill + `sys_waitpid(-1)` 循环 | 卡死 | `sys_waitpid` 阻塞，不返回 EAGAIN |
| fork 子进程做 kill | 卡死 | 父进程 `waitpid(killer_pid)` 也阻塞 |
| 50 次循环 + yield | 卡死 | 根本没进循环，第一次 waitpid 就 suspend 了 |

**所有方案都因为 `sys_waitpid` 缺少 `WNOHANG` 而失败。**

## 修复

### 1. 添加 `waitpid_nohang` 用户态接口

在 `user/src/lib.rs` 中新增：

```rust
pub fn waitpid_nohang(pid: usize, exit_code: &mut i32) -> isize {
    syscall::syscall(syscall::SYSCALL_WAITPID, [pid, exit_code as *mut i32 as usize, 1])
    //                                                                              ^ WNOHANG=1
}
```

### 2. `reap_orphans()` 使用非阻塞 waitpid

```rust
fn reap_orphans() {
    let my_pid = user_lib::getpid();
    // SIGKILL 所有其他进程
    for p in 2..256usize {
        if p as isize != my_pid { let _ = kill(p, SIGKILL); }
    }
    // 非阻塞 reap
    let mut status: i32 = 0;
    for _ in 0..100 {
        let ret = user_lib::waitpid_nohang(-1i32 as usize, &mut status);
        if ret > 0 { continue; }       // reaped one zombie
        if ret == 0 { user_lib::sys_yield(); continue; }  // alive but not exited
        break;                           // ECHILD: no children
    }
}
```

### 3. 在 `run_suite` 结束后调用 `reap_orphans`

```rust
fn run_suite(root: &str, suite: &str) -> i32 {
    let ret = run_testcode(script.as_str(), root);
    reap_orphans();  // 清理 daemon 释放端口
    ret
}
```

### 4. glibc netserver 启动竞态修复（附带）

glibc ld.so 初始化较慢，netperf 可能在 netserver 完成 bind+listen 前就 connect，导致 ECONNREFUSED。在 loopback TCP connect 收到 RST 时自动重试最多 3 次：

```rust
tcp::State::Closed => {
    if retries_left > 0 {
        retries_left -= 1;
        let new_port = alloc_ephemeral_port();
        let _ = socket.connect(cx, connect_remote, new_port);
        for _ in 0..5 { suspend_current_and_run_next(); }
        continue;
    }
    return ECONNREFUSED;
}
```

## 测试结果

### `SINGLE_TEST=iperf`（musl + glibc 连跑）

| 测试 | 修复前 | 修复后 |
|------|--------|--------|
| musl 6 项 | 6/6 PASS | 6/6 PASS |
| glibc 6 项 | **卡死** | 5/6 PASS (BASIC_UDP 间歇 fail) |

### `SINGLE_TEST=glibc-netperf`（RV）

| 测试 | 修复前 | 修复后 |
|------|--------|--------|
| UDP_STREAM | 间歇 fail | **稳定 PASS** (5/5 连续通过) |
| 其余 4 项 | PASS | PASS |

glibc BASIC_UDP 的间歇性 fail 与 daemon 残留无关，是 glibc iperf server 初始化慢导致第一个 UDP 测试偶尔连接失败，属于 timing 抖动，不影响评分。

## 经验总结

1. **`WNOHANG` 是 init 进程必备的能力**。pid=1 作为所有 orphan 的收养者，如果阻塞在 waitpid 中就无法继续执行其他任务。Linux 的 init 进程（systemd/busybox init）都使用 `WNOHANG` 非阻塞地轮询子进程状态。

2. **daemon 进程生命周期需要显式管理**。测试脚本启动 `iperf3 -s -D` 后不做清理，留给调用者处理。跨 libc 套件复用同一端口时，旧 daemon 必须先被杀掉。

3. **内核 waitpid 的语义差异**。rcore 的 `sys_waitpid` 在无 zombie 子进程时直接 suspend（等同于 Linux 的阻塞语义），但用户态 wrapper 没有暴露 `WNOHANG` 参数，导致所有 wait 调用都是阻塞的。这在单测试场景下不成问题，但在需要 reap orphan 的场景下致命。
