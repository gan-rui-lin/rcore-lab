# netperf 全通 + LTP 网络测试适配记录

**日期**: 2026/3/23

---

## 1. 本次成果总览

| 测试套件 | 结果 | 变化 |
|---------|------|------|
| **netperf (musl)** | **5/5 PASS** | TCP_CRR 从 FAIL→PASS |
| **iperf3 (musl)** | 6/6 PASS | 无回归 |
| **basic (musl)** | 全部 PASS | 无回归 |
| **LTP 网络 (musl)** | **14/25 PASS** | 从 0→14 |

---

## 2. netperf TCP_CRR 修复

### 2.1 罪魁祸首：idle loop 定时器中断盲区

**现象**: netperf 前 4 个测试全部通过，TCP_CRR（Connect/Request/Response）永远卡死。

**调试过程**:

1. 通过 `LOG=INFO` 搜索 SIGALRM 日志，发现客户端 (pid=4) 的 SIGALRM 正常触发（5 次），但服务端 (pid=9) 的 SIGALRM **从未触发**，尽管 `setitimer(val=5000ms)` 已正确调用。

2. 在 `check_timer()` 和 `check_itimers()` 入口添加 `warn!` 日志，发现**两者在 t≈9233ms 之后再也没有被调用**。这说明定时器中断本身没有触发，而不是定时器逻辑有问题。

3. 分析 RISC-V 中断机制：当 trap（系统调用）发生时，硬件自动 `SIE=0`（禁用中断）。内核代码在 `SIE=0` 下运行。当所有进程都阻塞在内核态系统调用中（accept、recv、waitpid 等），`SIE` 始终为 0，定时器中断永远不被 CPU 响应。

**根因**: `run_tasks()` 空闲循环中，`sstatus.SIE` 从内核态继承为 0。上下文切换保存/恢复 callee-saved 寄存器但不保存 `sstatus`，所以 SIE 永远是 0。idle loop 虽然在迭代间有极短的中断使能窗口（`UPIntrFreeCell` guard drop 时恢复），但恢复的也是 SIE=0。

```
所有进程阻塞在内核 syscall → SIE=0
→ yield → idle loop → SIE 仍然=0（继承自内核代码）
→ 定时器中断永远不触发
→ check_timer() 永远不调用
→ SIGALRM 永远不投递
→ 服务端永远不退出 accept 循环
→ 死锁
```

**修复** (`os/src/task/processor.rs`):

```rust
pub fn run_tasks() {
    loop {
        // 在每次循环顶部短暂开启中断，让挂起的定时器中断能被响应
        arch::enable_interrupts();
        arch::disable_interrupts();
        // ... 原有调度逻辑 ...
    }
}
```

RISC-V 定时器中断是电平触发的——一旦硬件计时器到期，中断请求持续挂起。`enable_interrupts()` 和 `disable_interrupts()` 之间只需一个指令周期，CPU 就会立即响应挂起的中断，跳转到 `kernel_interrupt_dispatch` → `check_timer()` → `check_itimers()` → 投递 SIGALRM。

**防御性修复** (`os/src/timer.rs`): `check_itimers` 改用 `try_inner_exclusive_access()`，避免定时器中断在某段代码持有进程锁时触发导致死锁。

### 2.2 验证

```
[itimer] pid=4 SIGALRM fired, expire=9412 now=9422   ← 客户端（一直正常）
[itimer] pid=9 SIGALRM fired, expire=13418 now=13422  ← 服务端（修复后新增！）
====== netperf TCP_CRR end: success ======
```

---

## 3. LTP 网络测试适配

### 3.1 LTP 测试架构

LTP（Linux Test Project）是 Linux 内核最权威的系统调用合规性测试套件。sdcard 上有约 2708 个测试二进制，其中网络相关约 25 个核心测试。

测试脚本（`ltp_testcode.sh`）遍历 `ltp/testcases/bin/` 下所有文件并执行。每个测试输出 `TPASS`/`TFAIL`/`TCONF` 和 `Summary:` 段落。judge 脚本解析 Summary 统计通过/失败数。

### 3.2 发现的问题与修复

#### 问题 1: fchmodat/fchmod ENOSYS → 所有 LTP 测试阻塞

LTP 框架在每个测试开始时创建临时目录并调用 `chmod()` 设置权限。musl 将 `chmod()` 转换为 `fchmodat` (syscall 53)，内核返回 ENOSYS，所有 2708 个测试全部在初始化阶段失败。

**修复**: 添加 fchmodat(53)/fchmod(52) 桩函数，返回 0。rCore-lab 的文件系统不跟踪权限位，桩函数足够。同理添加 fchownat(54)/fchown(55) 和 msync(227) 桩函数。

```rust
SYSCALL_FCHMOD | SYSCALL_FCHMODAT => 0,
54 | 55 => 0, // fchownat/fchown
227 => 0,     // msync
```

#### 问题 2: waitpid 对 SIGCHLD/SIGUSR1 返回虚假 EINTR → LTP 框架崩溃

LTP 框架 fork 子进程执行实际测试，父进程 `waitpid(child)` 等待。子进程通过 `kill(parent, SIGUSR1)` 通知"初始化完成"。父进程的 SIGUSR1 handler 使用了 **SA_RESTART** 标志。

在 Linux 上，SA_RESTART 让 waitpid 在 handler 执行后自动重启，不返回 EINTR。但 rCore-lab 之前**总是返回 EINTR**，不区分信号类型和标志位：

```
旧行为: 任何 unmasked pending signal → 返回 EINTR
```

这导致两类问题：
- SIGCHLD（SIG_DFL=忽略）也触发 EINTR——Linux 上不会
- SIGUSR1 + SA_RESTART 也触发 EINTR——Linux 上 waitpid 会自动重启

**修复** (`os/src/syscall/process.rs`): waitpid 的 EINTR 判断改为三层过滤：

```rust
for each pending unmasked signal:
    1. handler == SIG_DFL(0) || SIG_IGN(1) → 跳过（不触发 EINTR）
    2. handler 有用户函数 && SA_RESTART 设置 → 跳过（可重启 syscall）
    3. handler 有用户函数 && 无 SA_RESTART → 返回 EINTR
```

这需要新增 `SA_RESTART` 常量（`0x10000000`）并从 `action.rs` 导出。

#### 问题 3: accept() 对非 LISTEN socket 卡死 → accept01 永远不返回

LTP `accept01` 测试第 2 个子项：对一个已 bind 但**未 listen** 的 TCP socket 调用 `accept()`，期望返回 EINVAL。

旧代码只检查 `bound_port == 0`，已 bind 的 socket 通过检查，进入 accept 循环等待连接——永远不会有连接来，卡死。

**修复** (`os/src/net/syscall.rs`):

```rust
// 旧: if sock_type != SocketType::Tcp || bound_port == 0 { return EINVAL; }
// 新:
if sock_type != SocketType::Tcp { return EOPNOTSUPP; }  // UDP → EOPNOTSUPP
if !listening || bound_port == 0 { return EINVAL; }      // 非 LISTEN → EINVAL
```

### 3.3 LTP 网络测试逐项结果

| 测试 | 结果 | 子测试 | 说明 |
|------|------|--------|------|
| **socket01** | PASS | 9 项 | socket() 域/类型/协议组合 + 错误处理 |
| **socket02** | PASS | 4 项 | SOCK_CLOEXEC, SOCK_NONBLOCK 标志 |
| bind01 | FAIL(32) | 7 项 | 需要 AF_UNIX + /dev/null ENOTSOCK |
| bind02 | FAIL(2) | 1 项 | 需要 getpwnam("nobody") → 用户数据库 |
| bind03 | FAIL(32) | | 需要 AF_UNIX domain socket |
| **listen01** | PASS | 3 项 | EBADF, ENOTSOCK, EOPNOTSUPP |
| **accept01** | PASS | 5 项 | EBADF, EINVAL(×2), EOPNOTSUPP |
| **accept02** | PASS | | multicast CVE 测试 |
| accept03 | FAIL(2) | | 需要更多 accept 场景 |
| accept4_01 | FAIL(32) | | 需要 /proc/self/maps |
| **connect01** | PASS | 7 项 | EBADF, EFAULT, EISCONN, ECONNREFUSED 等 |
| **connect02** | PASS | | IPv6 ADDRFORM CVE 测试（跳过） |
| send01 | FAIL(127) | | 二进制执行失败 |
| send02 | FAIL(127) | | 二进制执行失败 |
| sendto01 | FAIL(3) | 10 项 | 部分子测试失败 |
| **sendto02** | PASS | | SCTP 测试（跳过=通过） |
| sendmsg01 | FAIL(2) | | 需要 loopback 接口配置 |
| **recv01** | PASS | 5 项 | EBADF, ENOTSOCK, EFAULT + MSG_OOB/ERRQUEUE |
| **recvfrom01** | PASS | 7 项 | 同 recv01 + from 地址参数 |
| **getsockname01** | PASS | 6 项 | EBADF, ENOTSOCK, EFAULT 等 |
| getpeername01 | FAIL(32) | | 需要 AF_UNIX socketpair |
| **getsockopt01** | PASS | 9 项 | EBADF, ENOTSOCK, EFAULT, ENOPROTOOPT 等 |
| getsockopt02 | FAIL(32) | | 需要 AF_UNIX SO_PEERCRED |
| **setsockopt01** | PASS | 8 项 | 错误处理 |
| **socketpair01** | PASS | 10 项 | 域/类型组合 + 错误处理 |

**通过率: 14/25 (56%)**

### 3.4 未通过测试的分类

| 失败原因 | 测试 | 数量 |
|---------|------|------|
| **需要 AF_UNIX** | bind01, bind03, getpeername01, getsockopt02 | 4 |
| **需要 /proc** | accept4_01 (需要 /proc/self/maps) | 1 |
| **二进制执行失败** | send01, send02 (ret=127) | 2 |
| **需要用户数据库** | bind02 (getpwnam) | 1 |
| **需要 loopback 接口** | sendmsg01 (ifconfig/ip 配置) | 1 |
| **部分子测试失败** | accept03, sendto01 | 2 |

---

## 4. glibc 动态链接问题（未解决）

glibc 版 netperf/netserver 是动态链接的（而 iperf3 是静态链接的），运行时报：

```
symbol lookup error: ./netserver: undefined symbol: stdin, version GLIBC_2.27
```

`libm.so.6` 和 `libc.so.6` 均存在于 `/glibc/lib/`，版本也兼容（GLIBC_2.27~2.35）。问题是 glibc 动态链接器解析 COPY relocation 时失败——`stdin` 和 `optind` 是数据对象，需要 `R_RISCV_COPY` 重定位把它们从 libc.so.6 复制到可执行文件的 BSS 段。rCore-lab 的 ELF loader 可能不支持这种重定位类型。

**已做的尝试**:
- initcode 添加 `LD_LIBRARY_PATH=/glibc/lib` 环境变量 → 解决了 `libm.so.6: cannot open` 问题
- initcode 添加 `/lib/libc.so.6` → `/glibc/lib/libc.so.6` 硬链接 → 解决了 `no version information` 问题
- 但 `undefined symbol: stdin` 仍然存在 → 这是 ELF loader 层面的问题

**结论**: glibc 动态链接二进制需要内核 ELF loader 支持 COPY relocation，这是一个独立的大功能，不在本次 netperf 修复范围内。

---

## 5. 修改文件清单

| 文件 | 修改内容 | 类型 |
|------|----------|------|
| `os/src/task/processor.rs` | idle loop 开中断窗口 | netperf TCP_CRR 核心修复 |
| `os/src/timer.rs` | check_itimers 使用 try_lock | 防御性修复 |
| `os/src/syscall/mod.rs` | fchmod/fchmodat/fchownat/msync 桩函数 | LTP 框架依赖 |
| `os/src/syscall/process.rs` | waitpid EINTR: SIG_DFL 跳过 + SA_RESTART 跳过 | LTP 框架依赖 |
| `os/src/task/action.rs` | 添加 SA_RESTART 常量 | SA_RESTART 支持 |
| `os/src/task/mod.rs` | 导出 SA_RESTART | SA_RESTART 支持 |
| `os/src/net/syscall.rs` | accept() 检查 LISTEN 状态 | accept01 卡死修复 |
| `user/src/bin/initcode.rs` | LD_LIBRARY_PATH + /lib/ 硬链接 | glibc 动态链接支持 |

---

## 6. 下一步计划

### P0: 提分项（收益大、改动小）

1. **send01/send02 执行失败 (ret=127)**: 排查为什么二进制无法执行，可能是动态链接或路径问题
2. **sendto01 部分子测试失败**: 对照源码补充 EPIPE+SIGPIPE、EMSGSIZE 等错误路径
3. **accept03**: 查看源码确认失败原因

### P1: AF_UNIX 域套接字（4 个测试依赖）

AF_UNIX 是一个独立的大功能模块（内核内 socketpair、bind 到文件系统路径、SCM_RIGHTS 等），但 bind01/bind03/getpeername01/getsockopt02 都依赖它。可以先实现最小 AF_UNIX（socketpair + 基础 read/write），不支持文件系统绑定。

### P2: 扩展功能

- sendmsg/recvmsg 基础实现（当前返回 EOPNOTSUPP）
- accept4() 传递 SOCK_CLOEXEC/SOCK_NONBLOCK 标志
- /proc/self/maps 支持（accept4_01 依赖）

### P3: glibc 动态链接

- 排查 ELF loader 对 R_RISCV_COPY relocation 的支持
- 这将解锁所有 glibc 动态链接测试

---

## 7. 调试经验总结

### 经验 1: 定时器中断不触发 ≠ 定时器逻辑有问题

当 `check_timer()` 的日志完全消失时，问题不在定时器代码本身，而在中断使能层面。排查方法：在 `check_timer()` 入口加 `warn!`，如果日志消失说明函数根本没被调用，问题在 `sstatus.SIE`。

### 经验 2: 逐层加日志——用减法定位

```
SIGALRM 没触发 ← check_itimers 没执行 ← check_timer 没被调用 ← 定时器中断没触发 ← SIE=0
```

每一层只需一行 `warn!` + 重跑 30 秒即可确认，整个排查不超过 10 分钟。

### 经验 3: SA_RESTART 是 LTP 的隐含依赖

LTP 框架大量使用 SIGUSR1 + SA_RESTART 进行父子进程同步。不实现 SA_RESTART，几乎所有需要 fork 的 LTP 测试都会因 waitpid EINTR 而 TBROK。这不是某个测试的 bug，而是整个 LTP 框架的基础设施依赖。

### 经验 4: EINTR 的三层语义

Linux 的 EINTR 语义比"有 pending signal 就返回"精细得多：

| 信号类型 | waitpid 行为 |
|---------|-------------|
| SIG_DFL（默认=忽略，如 SIGCHLD） | 不返回 EINTR |
| SIG_IGN | 不返回 EINTR |
| 用户 handler + SA_RESTART | 自动重启 syscall，不返回 EINTR |
| 用户 handler + 无 SA_RESTART | 返回 EINTR |

之前我们对所有 unmasked pending signal 都返回 EINTR，这在 netperf 场景下恰好工作（因为 SIGALRM 的 handler 没有 SA_RESTART），但在 LTP 框架下立即暴露。

### 经验 5: accept() 必须检查 LISTEN 状态

对非 LISTEN 的 socket 调 accept 应该立即返回错误，而不是进入等待循环。这种"应该快速失败却卡死"的 bug 在日志中表现为"某个测试之后再也没有输出"，需要用 `tail -20` 看最后执行到哪个测试来定位。

### 经验 6: 桩函数是解锁大量测试的捷径

`fchmodat`、`fchownat`、`msync` 这些 syscall 对 rCore-lab 的功能没有实质影响（不跟踪权限、单核无需 sync），但它们是 LTP 框架的硬依赖。一行 `=> 0` 的桩函数就能解锁几乎所有 2708 个 LTP 测试的初始化阶段。在适配测试套件时，优先找这种"花 1 分钟改、解锁 1000 个测试"的低垂果实。

---

## 8. 测试命令参考

```bash
# netperf (5 个测试)
SINGLE_TEST=musl-netperf LOG=ERROR timeout 60 bash run.sh -f sdcard-rv.img -t all

# iperf3 (6 个测试)
SINGLE_TEST=musl-iperf LOG=ERROR timeout 180 bash run.sh -f sdcard-rv.img -t all

# LTP 网络测试 (需要先替换 sdcard 上的 ltp_testcode.sh 为网络专用版本)
SINGLE_TEST=musl-ltp LOG=ERROR timeout 180 bash run.sh -f sdcard-rv.img -t all

# 替换 sdcard 上的 ltp_testcode.sh (通过 docker)
docker run --rm --privileged -v sdcard-rv.img:/sdcard.img ubuntu:22.04 bash -c '
  e2fsck -y /sdcard.img > /dev/null 2>&1
  mkdir -p /mnt/sd && mount -o loop /sdcard.img /mnt/sd
  # 写入只含网络测试的脚本 (参见 ltp-network-tests-analysis.md)
  cp /mnt/sd/musl/ltp_testcode.sh /mnt/sd/musl/ltp_testcode_full.sh
  printf "#!/bin/bash\n..." > /mnt/sd/musl/ltp_testcode.sh
  sync && umount /mnt/sd'
```
