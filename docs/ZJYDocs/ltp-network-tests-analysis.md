# LTP 网络测试源码分析与适配计划

**日期**: 2026/3/23

---

## 1. LTP 测试架构概述

### 1.1 什么是 LTP

LTP（Linux Test Project）是 Linux 内核最权威的系统调用合规性测试套件，由 IBM/SGI/OSDL 维护。其 `testcases/kernel/syscalls/` 目录包含约 2700+ 个测试二进制，覆盖几乎所有 POSIX 和 Linux 特有系统调用。

### 1.2 rCore-lab 中的 LTP

- **源码位置**: `/Users/mac/Desktop/project/syscall/testsuits-for-oskernel/ltp-full-20240524/`
- **sdcard 上位置**: `/musl/ltp/testcases/bin/`（约 2708 个二进制）
- **测试脚本**: `/musl/ltp_testcode.sh` — 遍历 `ltp/testcases/bin/` 下所有文件并执行

```bash
# ltp_testcode.sh 核心逻辑
for file in "$target_dir"/*; do
  echo "RUN LTP CASE $(basename "$file")"
  "$file"
  ret=$?
  echo "FAIL LTP CASE $(basename "$file") : $ret"
done
```

### 1.3 评分机制

judge 脚本解析每个测试的 `Summary:` 输出段：
```
Summary:
passed   6
failed   0
broken   0
skipped  0
warnings 0
```
最终分数 = 所有测试的 `passed` 之和。

---

## 2. 当前阻塞问题：fchmodat ENOSYS

### 2.1 现象

几乎所有 LTP 测试启动时立即失败：
```
chmod(/dev/shm/ltp_accept01_3,0666) failed: ENOSYS (38)
```

### 2.2 根因

LTP 测试框架（`tst_tmpdir.c`）在每个测试开始时创建临时目录并调用 `chmod()` 设置权限。musl libc 将 `chmod()` 转换为 `fchmodat(AT_FDCWD, path, mode, 0)` 系统调用（syscall 53）。rCore-lab 当前未实现 `fchmodat`，返回 ENOSYS。

### 2.3 修复方案

实现 `fchmodat` 桩函数——由于 rCore-lab 的文件系统不跟踪文件权限位，可以简单返回 0（成功）而不实际修改权限。这足以让 LTP 框架初始化通过。

```rust
// os/src/syscall/fs.rs
pub fn sys_fchmodat(_dirfd: i32, _path: *const u8, _mode: u32, _flags: i32) -> isize {
    0 // stub: rCore-lab FS doesn't track permission bits
}
```

---

## 3. LTP 网络 syscall 测试详细分析

### 3.1 测试清单

sdcard 上存在的网络相关 LTP 测试（按字母序）：

| 测试二进制 | 测试的 syscall | 核心测试内容 |
|-----------|---------------|-------------|
| accept01 | accept() | 错误处理: EBADF, EINVAL, EOPNOTSUPP |
| accept02 | accept() | CVE-2017-8890 multicast 复制问题 |
| accept03 | accept() | 已连接/未连接状态 |
| accept4_01 | accept4() | SOCK_CLOEXEC, SOCK_NONBLOCK 标志 |
| bind01 | bind() | 错误处理: EINVAL, ENOTSOCK, EAFNOSUPPORT, EBADF |
| bind02 | bind() | 特权端口绑定 EACCES |
| bind03 | bind() | AF_UNIX 重复绑定 EADDRINUSE |
| connect01 | connect() | 错误处理: EBADF, EFAULT, EISCONN, ECONNREFUSED |
| connect02 | connect() | IPv6 ADDRFORM 转换 (CVE-2018-9568) |
| listen01 | listen() | 错误处理: EBADF, ENOTSOCK, EOPNOTSUPP |
| recv01 | recv() | 错误处理: EBADF, ENOTSOCK, EFAULT, MSG_OOB |
| recvfrom01 | recvfrom() | 错误处理: EBADF, ENOTSOCK, EINVAL, MSG_OOB |
| recvmsg01 | recvmsg() | 错误处理 + SCM_RIGHTS 文件描述符传递 |
| recvmsg02/03 | recvmsg() | 更多 recvmsg 场景 |
| send01 | send() | 错误处理: EBADF, ENOTSOCK, EFAULT, EPIPE, MSG_OOB |
| send02 | send() | MSG_MORE 标志 (TCP/UDP 批量发送) |
| sendto01 | sendto() | 错误处理: EBADF, ENOTSOCK, EFAULT, EPIPE, EMSGSIZE |
| sendto02 | sendto() | SCTP 协议测试 |
| sendmsg01 | sendmsg() | 错误处理 + SCM_RIGHTS + MSG_OOB |
| sendmsg02 | sendmsg() | AF_UNIX DGRAM 竞态条件 |
| socket01 | socket() | 域/类型/协议组合: AF_UNIX, AF_INET, SOCK_RAW |
| socket02 | socket() | SOCK_CLOEXEC, SOCK_NONBLOCK 标志验证 |
| socketpair01 | socketpair() | 域/类型组合 + 错误处理 |
| getsockname01 | getsockname() | 错误处理: EBADF, ENOTSOCK, EFAULT |
| getpeername01 | getpeername() | 错误处理: EBADF, ENOTSOCK, ENOTCONN, EFAULT |
| getsockopt01 | getsockopt() | 错误处理: EBADF, ENOTSOCK, EFAULT, ENOPROTOOPT |
| getsockopt02 | getsockopt() | SO_PEERCRED (AF_UNIX 对端凭证) |
| setsockopt01 | setsockopt() | 错误处理: EBADF, ENOTSOCK, EFAULT, ENOPROTOOPT |
| setsockopt02-10 | setsockopt() | 各种 CVE 修复验证 |
| shutdown01/02 | shutdown() | 正常关闭 + 错误处理 |

### 3.2 各测试详细分析

#### socket01 — socket() 创建测试

```
测试 1: socket(0, SOCK_STREAM, 0)         → EAFNOSUPPORT  (无效域)
测试 2: socket(AF_INET, 75, 0)            → EINVAL        (无效类型)
测试 3: socket(PF_UNIX, SOCK_DGRAM, 0)    → 成功          (Unix域套接字)
测试 4: socket(PF_INET, SOCK_RAW, 0)      → EPROTONOSUPPORT (非root)
测试 5: socket(PF_INET, SOCK_DGRAM, 17)   → 成功          (UDP)
测试 6: socket(PF_INET, SOCK_STREAM, 17)  → EPROTONOSUPPORT (类型/协议不匹配)
测试 7: socket(PF_INET, SOCK_DGRAM, 6)    → EPROTONOSUPPORT (类型/协议不匹配)
测试 8: socket(PF_INET, SOCK_STREAM, 6)   → 成功          (TCP)
测试 9: socket(PF_INET, SOCK_STREAM, 1)   → EPROTONOSUPPORT (ICMP+STREAM不匹配)
```

**当前支持情况**:
- 测试 1,2: 需要返回正确 errno（当前: EAFNOSUPPORT 支持, EINVAL 支持）
- 测试 3: **需要 AF_UNIX 支持**（当前不支持）
- 测试 4: 需要返回 EPROTONOSUPPORT（当前可能返回 EINVAL）
- 测试 5,8: 已支持
- 测试 6,7,9: 需要检查协议/类型匹配

#### socket02 — socket() 标志测试

```
测试 1: socket(AF_INET, SOCK_STREAM, 0)                  → 无 FD_CLOEXEC
测试 2: socket(AF_INET, SOCK_STREAM|SOCK_CLOEXEC, 0)     → 有 FD_CLOEXEC
测试 3: socket(AF_INET, SOCK_STREAM, 0)                  → 无 O_NONBLOCK
测试 4: socket(AF_INET, SOCK_STREAM|SOCK_NONBLOCK, 0)    → 有 O_NONBLOCK
```

**当前支持情况**: 已支持 SOCK_CLOEXEC 和 SOCK_NONBLOCK，但需要 `fcntl(F_GETFD)` 和 `fcntl(F_GETFL)` 返回正确值。

#### accept01 — accept() 错误处理

```
测试 1: accept(400, ...)                    → EBADF
测试 2: accept(fd, (sockaddr*)3, ...)       → EINVAL
测试 3: accept(fd, ..., (socklen_t*)1)      → EINVAL
测试 4: accept(listen_fd_no_conn, ...)      → EINVAL (无排队连接)
测试 5: accept(udp_fd, ...)                 → EOPNOTSUPP
```

**当前支持情况**: 部分支持。需要补充更严格的参数校验。

#### connect01 — connect() 错误处理

```
测试 1: connect(400, ...)                   → EBADF
测试 2: connect(fd, (sockaddr*)-1, 28)      → EFAULT
测试 3: connect(fd, addr, 3)                → EINVAL
测试 4: connect(devnull_fd, ...)            → ENOTSOCK
测试 5: connect(already_connected, ...)     → EISCONN
测试 6: connect(fd, refused_addr, ...)      → ECONNREFUSED
测试 7: connect(fd, {family=47}, ...)       → EAFNOSUPPORT
```

**当前支持情况**: EBADF/EINVAL/ECONNREFUSED 已支持，需要补充 EFAULT/ENOTSOCK/EISCONN/EAFNOSUPPORT。

#### send01 — send() 错误处理

```
测试 1: send(400, ...)                      → EBADF
测试 2: send(devnull_fd, ...)               → ENOTSOCK
测试 3: send(fd, (void*)-1, ...)            → EFAULT
测试 4: send(udp_fd, 128KB_buf, ...)        → EMSGSIZE
测试 5: send(shutdown_fd, ...)              → EPIPE  (+ SIGPIPE)
测试 6: send(udp_fd, ..., MSG_OOB)          → EOPNOTSUPP
```

**关键**: `send()` 在 Linux 上是 `sendto(fd, buf, len, flags, NULL, 0)` 的包装。我们需要确保 `sendto` 在 `dest_addr=NULL` 时对已连接 socket 正常工作。

---

## 4. 需要实现/修复的 syscall 清单

### 4.1 前置依赖（阻塞所有 LTP 测试）

| syscall | 编号 | 优先级 | 实现方案 |
|---------|------|--------|---------|
| fchmodat | 53 | **P0** | 桩函数返回 0 |
| fchmod | 52 | P0 | 桩函数返回 0 |

### 4.2 网络 syscall 错误处理增强

| 功能 | 优先级 | 说明 |
|------|--------|------|
| socket() 协议/类型匹配检查 | P1 | protocol!=0 时检查与 type 的一致性 |
| socket() EPROTONOSUPPORT | P1 | SOCK_RAW 返回 EPROTONOSUPPORT |
| connect() EISCONN | P1 | 已连接 socket 再次 connect 返回 EISCONN |
| connect() ENOTSOCK | P1 | 非 socket fd 调用 connect |
| connect() EFAULT | P2 | 无效用户指针检测 |
| send()/sendto() ENOTSOCK | P1 | 非 socket fd |
| send()/sendto() EPIPE + SIGPIPE | P1 | 向已关闭的连接发送 |
| send()/sendto() EMSGSIZE | P2 | UDP 数据报过大 |
| recv()/recvfrom() ENOTSOCK | P1 | 非 socket fd |
| accept() EOPNOTSUPP | P1 | UDP socket 调 accept |
| listen() EOPNOTSUPP | P1 | UDP socket 调 listen（已实现） |
| getsockopt/setsockopt 错误路径 | P2 | ENOPROTOOPT 等 |

### 4.3 新 syscall 实现

| syscall | 编号 | 优先级 | 说明 |
|---------|------|--------|------|
| sendmsg | 211 | P2 | 需要 struct msghdr 解析 |
| recvmsg | 212 | P2 | 需要 struct msghdr 解析 |
| socketpair | 199 | P3 | 需要 AF_UNIX 支持 |
| accept4 | 242 | P1 | accept + SOCK_CLOEXEC/SOCK_NONBLOCK |

### 4.4 AF_UNIX 域套接字（长期目标）

多个 LTP 测试依赖 AF_UNIX（Unix 域套接字）：
- socket01 测试 3
- bind03（AF_UNIX 绑定）
- socketpair01
- sendmsg01/recvmsg01 的 SCM_RIGHTS 测试
- getsockopt02 的 SO_PEERCRED

AF_UNIX 是一个独立的大功能模块，建议作为后续独立任务。

---

## 5. 实现计划

### 阶段 1: 解除 LTP 框架阻塞

1. **实现 fchmodat/fchmod 桩函数** — 返回 0，让 LTP 框架能初始化
2. **验证**: 运行 `accept01` 等网络测试，确认进入实际测试逻辑

### 阶段 2: 网络 syscall 错误处理

1. **socket() 协议匹配**: 检查 protocol 与 type 的一致性
2. **connect() 错误路径**: EISCONN, ENOTSOCK
3. **send/recv 错误路径**: ENOTSOCK, EPIPE+SIGPIPE, EMSGSIZE
4. **accept4() 实现**: 在 accept 基础上传递 flags
5. **验证**: 逐个运行 socket01, accept01, connect01, send01 等

### 阶段 3: 扩展功能

1. sendmsg/recvmsg 基础实现
2. MSG_MORE, MSG_DONTWAIT 等标志
3. 更完整的 setsockopt/getsockopt

---

## 6. 预期可通过的测试

基于当前内核能力 + 阶段 1-2 修复后，预期可通过的网络测试子项（每个二进制含多个子测试）：

| 测试 | 总子测试 | 预期通过 | 主要障碍 |
|------|---------|---------|---------|
| socket01 | 9 | 6-7 | AF_UNIX (1项) |
| socket02 | 4 | 4 | 已支持 |
| bind01 | 7 | 5-6 | AF_UNIX (1项) |
| accept01 | 5 | 4-5 | |
| connect01 | 7 | 5-6 | EFAULT 校验 |
| listen01 | 3 | 3 | 已支持 |
| send01 | 6 | 4-5 | MSG_OOB |
| sendto01 | 10 | 7-8 | EFAULT 校验 |
| recv01 | 5 | 3-4 | MSG_OOB/ERRQUEUE |
| recvfrom01 | 7 | 4-5 | MSG_OOB/ERRQUEUE |
| getsockname01 | 6 | 4-5 | EFAULT 校验 |
| getpeername01 | 7 | 5-6 | EFAULT 校验 |
| shutdown01/02 | ~4 | 3-4 | |

**保守估计**: 网络相关 LTP 测试可通过 60-70% 的子测试项。主要缺口是 AF_UNIX 和 EFAULT（用户指针校验）。

---

## 7. 测试命令

```bash
# 运行全部 LTP（约 2700 个测试，耗时很长）
SINGLE_TEST=musl-ltp LOG=ERROR timeout 600 bash run.sh -f sdcard-rv.img -t all > ltp.log 2>&1

# 只看网络相关结果
strings ltp.log | grep -aE "RUN LTP CASE (socket|bind|listen|accept|connect|send|recv|shutdown|getsock|setsock|getpeer)"

# 快速验证单个测试（需要修改 ltp_testcode.sh 或在内核中选择性运行）
```
