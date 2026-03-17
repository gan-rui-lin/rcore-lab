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

## 4. 最新进展（2026-03-17）

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
- `waitpid01` 已确认通过

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
- `write05` 已确认通过
  - `EBADF`、`EFAULT`、`EPIPE` 均已对齐

#### 4.2.3 `dup2()` errno 兼容性

**问题**:
- `dup201` 期望 `dup2(0, -1)` 和 `dup2(0, maxfd)` 返回 `EBADF`
- 内核此前会把 `newfd >= RLIMIT_NOFILE` 误报为 `EMFILE`

**解决方案**:
- 调整 `sys_dup3()` 对非法 `newfd` 的返回值，改为 `EBADF`

**结果**:
- `dup201` 已确认通过

#### 4.2.4 `open01` mode / sticky bit 兼容

**问题**:
- `open01` 会检查新建文件的 mode，尤其是 sticky bit 是否被保留
- 之前 `stat/fstat/fstatat` 返回的 mode 偏向“合成默认值”，导致 `sticky bit is cleared unexpectedly`

**解决方案**:
- 在 `openat(O_CREAT)` / `mkdirat()` 创建对象时记录路径对应的 mode
- 在 `unlinkat()` 时同步清理该元数据
- `stat_from_fd()` / `stat_from_path()` / `sys_fstat()` / `sys_fstatat()` 统一读取保存下来的 mode bits

**结果**:
- `open01` 已确认通过
- `sticky bit` 和目录类型位都已对齐 LTP 预期

#### 4.2.5 `MAP_SHARED` 跨 `fork/clone(SIGCHLD)` 共享语义

**问题**:
- `getpid02` / `clone03` 依赖 `MAP_SHARED | MAP_ANONYMOUS` 在父子进程间共享同一页
- 之前 `MemorySet::from_existed_user()` 会给子进程重新分配页并拷贝内容，导致父进程看不到子进程写入的 pid

**解决方案**:
- 将 `MapArea` 的数据页持有方式改为 `Arc<FrameTracker>`，允许多个地址空间共享同一物理页
- 为共享映射增加 `shared_frames` 标记
- `sys_mmap()` 遇到 `MAP_SHARED` 时建立共享 framed area
- `MemorySet::from_existed_user()` 遇到共享映射时直接复用父进程已有物理页，而不是重新分配并复制

**结果**:
- `getpid02` 已确认通过
- `clone03` 已确认通过

#### 4.2.6 LTP 框架常见 syscall 补齐

**问题**:
- 在回归 `getpid02` / `clone03` 时，LTP 运行框架仍会触发 `sched_getaffinity`、`setitimer`、`msync`
- 这些 syscall 早先返回 `ENOSYS`，虽然未必直接让用例失败，但会污染日志并影响更多测例

**解决方案**:
- 增加最小兼容实现：
  - `sched_getaffinity()` 返回单核 CPU mask
  - `getitimer()` / `setitimer()` 提供最小成功路径与参数校验
  - `msync()` 对常规有效输入直接返回成功

**结果**:
- `getpid02` / `clone03` 回归时已不再出现对应 `ENOSYS`
- 为后续更多 LTP 用例继续扩展打掉了几处框架噪音

### 4.3 当前累计进度

以下统计基于“此前已逐个语义复测通过的 19 项”加上“本轮新修复后重新验证的失败项”得到。

#### 已累计确认通过：28 / 30

**进程管理类**:
- `getpid02`
- `fork01`, `fork03`, `wait01`, `wait02`, `wait401`
- `waitpid01`, `waitpid03`
- `clone01`, `clone02`, `clone03`

**基本 I/O 类**:
- `pipe01`
- `read01`, `read02`, `read04`
- `write01`, `write02`, `write03`, `write05`
- `close01`, `close02`
- `dup01`, `dup02`, `dup201`, `dup202`, `dup203`
- `open01`
- `lseek01`

说明:
- 相比上一版报告，新增确认通过的 5 个失败项为：`waitpid01`、`write05`、`open01`、`getpid02`、`clone03`
- 以上结论均按日志中的 `TPASS/TFAIL/TBROK` 复核

#### 仍未通过：2 / 30

- `exit01`
- `exit02`

说明:
- `exit01`、`exit02` 已重新回归，仍未通过
- 两者当前都没有跑到测试主体，而是在 `execve()` 阶段被识别成“非 ELF 文件”并回退给 `/bin/sh`

### 4.4 仍待解决的问题与根因

#### 4.4.1 `exit01` / `exit02`

**现象**:
- 两个用例都没有进入真正的 `exit()` 断言逻辑
- `execve("/musl/ltp/testcases/bin/exit01")` / `execve(.../exit02)` 时，内核把目标识别为“非 ELF 且无 shebang”，随后回退到 `/bin/sh`
- 最终表现分别为：
  - `exit01`: `syntax error: unexpected "("`
  - `exit02`: `root:x:0:0:root:/root:/bin/sh: not found`
- 进一步加临时探针后发现：
  - `exit01` 读到的是目录数据块，前几个目录项就是 `.` / `..`
  - `exit02` 读到的是 `/etc/passwd` 的首行文本

**根因判断**:
- 当前更像是这两个目标文件在镜像中的读取/识别异常，而不是 `sys_exit()` / `wait()` 的语义本身不对
- 问题点可能在：
  - ext4 路径解析命中了错误 inode
  - 文件内容读取错乱
  - 镜像中的这两个条目本身异常或与预期二进制不一致

**后续方案**:
- 在 `execve()` 失败路径打印这两个文件的前若干字节，确认到底读到了什么
- 必要时进一步排查 ext4 目录项解析和 inode 读取逻辑
- 在确认测试二进制本身能被正确读取后，再重新验证 `exit01` / `exit02` 是否还存在真实 `exit` 语义问题

### 4.5 建议的下一步优先级

1. 先定位 `exit01` / `exit02` 的文件读取异常
   - 这是当前目标集剩余的唯一 blocker

2. 顺手扩大一轮回归
   - 可以优先再试更多依赖 `MAP_SHARED`、`msync`、`setitimer`、`sched_getaffinity` 的 LTP 用例

3. 如有必要，再追 ext4 目录项 / inode 读取
   - 如果 `exit01` / `exit02` 读到的内容确实异常，这一层很可能是下一个核心战场

---

## 5. 总结

本轮工作的重点已经进一步从“补 stub 解锁 LTP 框架初始化”推进到“补关键内核语义并清理框架噪音”。

最新累计结果是：
- 已累计确认通过 28 / 30
- 这一阶段新增转绿的关键失败项为：`waitpid01`、`write05`、`open01`、`getpid02`、`clone03`
- 当前剩余重点问题集中在：`exit01` / `exit02` 的可执行文件读取或识别异常

当前最重要的经验是：**不能再看 LTP 自己打印的 `Summary` 或 `initcode` 外层 `[LTP] PASS`，必须按 `TPASS/TFAIL/TBROK` 判定真实结果。**
