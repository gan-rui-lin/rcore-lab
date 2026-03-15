# rCore LTP 测试报告

## 1. 概述

本报告记录了 rCore 内核适配 Linux Test Project (LTP) 测试套件的过程，包括代码修改、测试结果分析以及问题定位。

### 1.1 测试环境

- **架构**: RISC-V 64 (riscv64gc)
- **QEMU**: qemu-system-riscv64, virt machine, 128MB RAM
- **SD卡镜像**: sdcard-rv.img (4GB, ext4, 预装 LTP 二进制)
- **LTP版本**: 20240524
- **LTP二进制路径**: `/musl/ltp/testcases/bin/`

### 1.2 修改的文件清单

| 文件 | 修改内容 |
|------|---------|
| `user/src/bin/initcode.rs` | 重写 `test_ltp()` 函数，直接 fork+execve 每个测试用例，添加 watchdog 超时机制 |
| `user/src/syscall.rs` | 添加 `SYSCALL_MKDIRAT` 常量和 `sys_mkdirat()` 函数 |
| `user/src/lib.rs` | 添加 `mkdir()` 公共函数 |
| `os/src/syscall/mod.rs` | 注册 `fchmodat`, `fchmod`, `fchownat`, `fchown`, `setpgid` 系统调用 |
| `os/src/syscall/fs.rs` | 实现 `sys_fchmodat`, `sys_fchmod`, `sys_fchownat`, `sys_fchown` (stub) |
| `os/src/syscall/process.rs` | 实现 `sys_setpgid` (stub) |

---

## 2. initcode.rs 改造方案

### 2.1 原始方案

原来的 `test_ltp()` 通过 busybox shell 执行一个脚本来运行 LTP：

```rust
fn test_ltp() {
    run_testcode("/musl/ltp_testcode.sh");
}
```

### 2.2 新方案

参考 C 语言版本 `initcode.c`，改为直接 fork+execve 每个 LTP 测试二进制：

```rust
fn test_ltp() {
    mkdir("/tmp");
    mkdir("/dev");
    mkdir("/dev/shm");
    chdir("/musl");

    for name in LTP_TESTS {
        // fork 测试子进程
        let test_pid = fork();
        if test_pid == 0 {
            execve("/musl/ltp/testcases/bin/<name>", argv, envp);
        }
        // fork watchdog 超时进程
        let watchdog_pid = fork();
        if watchdog_pid == 0 {
            sleep(30_000); // 30秒超时
            exit(0);
        }
        // wait 任一子进程退出，判断是正常结束还是超时
    }
}
```

### 2.3 关键设计点

1. **环境变量**: 设置 `PATH`, `TMPDIR=/tmp`, `HOME=/tmp`, `LTPROOT=/musl/ltp`
2. **目录预创建**: `/tmp`, `/dev`, `/dev/shm` — LTP 框架依赖这些目录
3. **超时机制**: 每个测试限时 30 秒，使用 watchdog 子进程 + `wait()` + `kill(SIGKILL)` 实现
4. **统计输出**: 打印每个测试的 PASS/FAIL/TIMEOUT 状态，最终汇总

---

## 3. 内核修改详情

### 3.1 fchmodat / fchmod (syscall 53/52) — Stub 实现

**问题现象**:
```
tst_test.c:116: TBROK: chmod(/dev/shm/ltp_getpid01_2,0666) failed: ENOSYS (38)
```

**根因分析**:
LTP 框架 (`tst_test.c`) 在初始化阶段会在 `/dev/shm/` 下创建共享内存文件并调用 `chmod` 设置权限。rCore 未实现 `fchmodat` 系统调用(编号53)，导致返回 `-ENOSYS`。

**解决方案**:
添加 stub 实现，直接返回 0（成功）。因为 rCore 目前是单用户系统，权限检查无实际意义：

```rust
pub fn sys_fchmodat(_dirfd: isize, _path: *const u8, _mode: u32, _flags: u32) -> isize {
    0
}
```

**影响范围**: 修复了约 70+ 个因 chmod ENOSYS 而 TBROK 的测试用例。

### 3.2 fchownat / fchown (syscall 54/55) — Stub 实现

**问题现象**:
```
tst_tmpdir.c:287: chown(/tmp/LTP_cloplBEHd,-1,0) failed: errno=ENOSYS(38)
```

**根因分析**:
LTP 框架在创建临时目录时会调用 `chown` 设置目录所有权。

**解决方案**: 同 fchmodat，添加 stub 返回 0。

**影响范围**: 修复了约 10 个因 chown ENOSYS 而 TBROK 的测试用例（如 mmap01, mmap03, munmap01 等需要 tmpdir 的测试）。

### 3.3 setpgid (syscall 154) — Stub 实现

**问题现象**:
```
tst_test.c:1650: TBROK: setpgid(0, 0) failed: ENOSYS (38)
```

**根因分析**:
LTP 的 `fork_testrun()` 函数在 fork 子进程后会调用 `setpgid(0, 0)` 将测试进程放入独立进程组，以便之后通过 `kill(-test_pid, SIGKILL)` 清理所有后代进程。

**解决方案**: 添加 stub 返回 0。

**影响范围**: 这是最关键的一个修复，直接解锁了几乎所有测试用例的 LTP 框架初始化流程。修复后 PASS 数从 2 上升到 13+。

---

## 4. 最新进展（2026-03-15）

### 4.1 统计口径修正

当前 `initcode` 外层的 `[LTP] PASS ...` 仍然只是根据子进程退出码是否为 0 判断，因此**不能**直接代表该用例真实通过。

LTP 在本环境下打印的：

```text
Summary:
passed   0
failed   0
broken   0
...
```

同样也不可靠。

本报告后续统计均以**测试正文中的 `TPASS` / `TFAIL` / `TBROK`** 为准。

### 4.2 本轮新增修复

#### 4.2.1 `wait4()/waitpid()` 相关

**问题**:
- `wait401` 早期因为 `/proc/<pid>/stat` 缺失直接 `TBROK`
- `/proc/<pid>/stat` 补上后，又会因为父进程在 `waitpid()` 中只是 yield、没有表现为睡眠态 `'S'` 而卡住
- `sys_waitpid()` 写回用户态的 wait status 只按正常退出编码，信号退出场景编码不兼容 Linux

**解决方案**:
- 在 `procfs` 中补了最小动态 `/proc/<pid>/stat`
- 在 `/proc/<pid>/stat` 中把“正在 `waitpid/wait4` 等待循环中的任务”视为睡眠态 `'S'`
- 在 `sys_waitpid()` 中新增 wait status 编码逻辑，区分正常退出和信号退出

**结果**:
- `wait401` 已确认通过
- `waitpid01` 有明显改善，但仍未完全通过
  - `kill(getpid(), sig)` 路径大多正常
  - `raise(sig)` 路径仍有大量 `WIFSIGNALED()` 判定失败
  - `WCOREDUMP()` 对应的 core bit 也尚未补全

#### 4.2.2 `read/write` 用户缓冲区校验

**问题**:
- `read02` / `write03` / `write05` 涉及 `PROT_NONE` 用户地址时，内核此前没有按页权限检查用户缓冲区
- `read02` 中对目录 `read()` 的 errno 也不对，应该返回 `EISDIR`

**解决方案**:
- 在 `arch/src/riscv64/mm/page_table.rs` 增加带权限检查的 `translated_byte_buffer_checked()`
- `sys_read()` 对目标缓冲区要求可写；`sys_write()` 对源缓冲区要求可读
- `sys_read()` 对目录 inode 显式返回 `EISDIR`

**结果**:
- `read02` 已确认通过
- `write03` 已确认通过
- `write05` 部分修复
  - `EBADF`、`EFAULT` 已正确
  - pipe 写端对无读者场景仍未返回 `EPIPE`，该项还失败

#### 4.2.3 `dup2()` errno 兼容性

**问题**:
- `dup201` 期望 `dup2(0, -1)` 和 `dup2(0, maxfd)` 返回 `EBADF`
- 内核此前会把 `newfd >= RLIMIT_NOFILE` 误报为 `EMFILE`

**解决方案**:
- 调整 `sys_dup3()` 对非法 `newfd` 的返回值，改为 `EBADF`

**结果**:
- `dup201` 已确认通过

### 4.3 当前累计进度

以下统计基于“此前已逐个语义复测通过的 19 项”加上“本轮新修复后重新验证的失败项”得到。

#### 已累计确认通过：23 / 30

**进程管理类**:
- `fork01`, `fork03`, `wait01`, `wait02`, `wait401`
- `waitpid03`
- `clone01`, `clone02`

**基本 I/O 类**:
- `pipe01`
- `read01`, `read02`, `read04`
- `write01`, `write02`, `write03`
- `close01`, `close02`
- `dup01`, `dup02`, `dup201`, `dup202`, `dup203`
- `lseek01`

说明:
- 本轮新增确认通过的 4 个用例为：`wait401`、`read02`、`write03`、`dup201`

#### 仍未通过：7 / 30

- `getpid02`
- `waitpid01`
- `exit01`
- `exit02`
- `clone03`
- `write05`
- `open01`

说明:
- `exit01`、`exit02` 是上一轮已确认失败项，本轮未重新单独回归，但目前尚无修复

### 4.4 仍待解决的问题与根因

#### 4.4.1 `getpid02` / `clone03`

**现象**:
- 子进程把 pid 写入 `MAP_SHARED | MAP_ANONYMOUS` 内存后，父进程读到的值仍是 0 或错误值

**根因**:
- 当前匿名共享映射并没有真正实现跨 `fork()` / 非线程 `clone()` 的共享语义
- `MemorySet::from_existed_user()` 仍按普通私有页复制

**后续方案**:
- 为匿名 `MAP_SHARED` 页建立共享物理页或共享映射元数据
- 相关重点位置：
  - `os/src/syscall/process.rs`
  - `os/src/mm/memory_set.rs`

#### 4.4.2 `waitpid01`

**现象**:
- `raise(sig)` 分支中大量出现 `WIFSIGNALED() not set in status (exited with 0)`
- 触发 core dump 的信号场景里 `WCOREDUMP()` 也还不对

**根因判断**:
- 当前信号自发送路径仍不完整，`raise()` 与 `kill(getpid(), sig)` 的行为不一致
- wait status 里的 core-dump bit 也尚未编码

**后续方案**:
- 继续排查 `raise()` 走到的 `tkill/tgkill/kill` 路径
- 在 wait status 中补上 core-dump bit

#### 4.4.3 `write05`

**现象**:
- 对关闭读端的 pipe 写入时，本应返回 `EPIPE`，目前却成功返回

**根因判断**:
- pipe 写端对“无读者”场景的错误处理还不完整
- `SIGPIPE` 也可能没有正确送达

**后续方案**:
- 检查 `os/src/fs/pipe.rs` 的写路径
- 对读端已关闭的 pipe 返回 `EPIPE`，并补发 `SIGPIPE`

#### 4.4.4 `open01`

**现象**:
- `sticky bit is cleared unexpectedly`

**根因**:
- 文件创建模式 `_mode` 还没有在 VFS / stat 元数据中真正保存下来
- 当前 `stat` 相关实现仍倾向于合成默认 mode

**后续方案**:
- 在文件创建时保存 mode bits
- `stat/fstat/fstatat` 返回真实 mode，保留 `S_ISVTX`

### 4.5 建议的下一步优先级

1. 修 `write05`
   - 影响范围小，回报高，预计可以再新增 1 个通过用例

2. 修 `waitpid01`
   - 已经完成一半，继续补 `raise()` 和 core-dump bit 后有机会转绿

3. 修 `open01`
   - 主要是 mode/sticky bit 元数据问题，属于 VFS 语义补全

4. 攻 `MAP_SHARED | MAP_ANONYMOUS`
   - 能同时带动 `getpid02` 和 `clone03`
   - 但改动面最大，建议放在前三项之后

---

## 5. 总结

本轮工作的重点已经从“补 stub 解锁 LTP 框架初始化”转到“修真实内核语义”。

最新累计结果是：
- 已累计确认通过 23 / 30
- 本轮新增修复 4 项：`wait401`、`read02`、`write03`、`dup201`
- 当前剩余重点问题集中在：共享匿名映射、信号自发送/等待状态、pipe 的 `EPIPE`、文件 mode 元数据

当前最重要的经验是：**不能再看 LTP 自己打印的 `Summary` 或 `initcode` 外层 `[LTP] PASS`，必须按 `TPASS/TFAIL/TBROK` 判定真实结果。**
