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

## 4. 测试结果

### 4.1 三轮测试对比

| 轮次 | 修复内容 | PASS | 主要失败原因 |
|------|---------|------|-------------|
| Round 1 | 无修改，原始代码 | 2/92 | chmod ENOSYS (70+), chown ENOSYS (10+) |
| Round 2 | +fchmodat, +fchownat, +/dev/shm | 4/60* | setpgid ENOSYS (50+) |
| Round 3 | +setpgid | 13/25* | 各种具体测试问题 |

*注: Round 2/3 因 chdir01 卡住导致未跑完全部用例

### 4.2 已验证 PASS 的测试用例（共 28 个）

**进程管理类** (Round 3):
- `getpid02`, `fork01`, `fork03`, `wait01`, `wait02`, `wait401`
- `waitpid01`, `waitpid03`, `exit01`, `exit02`
- `clone01`, `clone02`, `clone03`

**基本 I/O 类** (单独验证):
- `pipe01` — 管道读写
- `read01` — 基本文件读取
- `read02` — 读取错误处理 (EBADF)
- `read04` — 数据正确性验证
- `write01` — 基本文件写入
- `write02` — 空写入 (NULL, 0)
- `write03` — 写入失败检测
- `write05` — 写入错误处理
- `close01` — 关闭文件/管道/socket
- `close02` — 关闭无效 fd (EBADF)
- `dup01` — dup + fstat inode 验证
- `dup02` — dup 无效 fd (EBADF)
- `dup201`, `dup202`, `dup203` — dup2 各种场景
- `open01` — 创建文件 + 打开目录
- `lseek01` — lseek SEEK_SET/CUR/END

### 4.3 已知失败的测试用例及原因分类

#### 4.3.1 需要 /proc 文件系统（ENOENT）

| 测试 | 缺失的文件 |
|------|-----------|
| `getpid01`, `getppid01`, `getppid02` | `/proc/sys/kernel/pid_max` |

**修复建议**: 实现基本的 procfs，或在 initcode 中创建静态文件。

#### 4.3.2 二进制文件缺失（EXEC_FAIL）

| 测试 | 说明 |
|------|------|
| `fork02`, `waitpid02`, `mkdir01`, `chdir02`, `chdir03` | SD卡镜像中缺少对应二进制 |
| `fstat01`, `unlink01`, `unlink02`, `kill01` | 同上 |

**修复建议**: 重新编译 LTP 并将缺失的二进制文件打入 SD 卡镜像。

#### 4.3.3 需要 mknod/mkfifo 系统调用（ENOSYS）

| 测试 | 需要的系统调用 |
|------|--------------|
| `read03` | `mknod()` — 创建设备文件用于测试 |
| `lseek02` | `mkfifo()` — 创建 FIFO 管道 |

**修复建议**: 实现 `mknodat` 系统调用(编号33)。

#### 4.3.4 需要 checkpoint 机制（futex + shared mmap）

| 测试 | 说明 |
|------|------|
| `pipe02` | `tst_checkpoint_wait/wake` 超时 |
| `fork04`, `execve05` | 需要 checkpoint 做进程间同步 |

**根因**: LTP checkpoint 使用 `/dev/shm` 下的共享内存文件 + `futex` 系统调用做进程间同步。虽然 `futex` 已实现，但需要 `mmap(MAP_SHARED)` 对文件的支持。

**修复建议**: 实现基于文件的 `mmap(MAP_SHARED)`。

#### 4.3.5 需要多用户/权限模型

| 测试 | 需要的功能 |
|------|-----------|
| `execve02`, `execve03` | `seteuid()`, `getpwnam("nobody")`, 权限检查 |
| `mkdir02` | `setregid/setreuid`, SGID 继承 |
| `chdir01` | `.mount_device`, `.all_filesystems`, root 权限 |

**修复建议**: 这些测试需要较完整的用户/组权限系统，优先级较低。

#### 4.3.6 brk 语义不兼容

| 测试 | 问题 |
|------|------|
| `brk01`, `brk02` | 内核 syscall 214 映射到 xv6 风格的 `sbrk(increment)` 而非 Linux 标准的 `brk(addr)` |

**根因**: Linux `brk(addr)` 设置程序 break 到指定地址并返回新的 break 地址；xv6 `sbrk(increment)` 增加/减少 break 并返回旧地址。LTP 测试使用 Linux 标准语义。

**修复建议**: 重写 `sys_sbrk` 为标准 Linux `brk` 语义：
```rust
pub fn sys_brk(addr: usize) -> isize {
    if addr == 0 {
        return current_brk();  // 查询当前 brk
    }
    set_brk(addr);  // 设置新的 brk
    return current_brk();  // 返回实际 brk 地址
}
```

#### 4.3.7 symlink 未实现

| 测试 | 问题 |
|------|------|
| `openat02` | `symlink()` 返回 ENOSYS |
| `openat03` | `write()` 返回 0 (可能是文件系统问题) |

**修复建议**: 实现 `symlinkat` (syscall 36)。

#### 4.3.8 测试卡住（超时/阻塞）

| 测试 | 问题分析 |
|------|---------|
| `chdir01` | 使用 guarded buffers (mmap+mprotect PROT_NONE)，可能在信号处理或 `.mount_device` 逻辑中卡住 |
| `pipe2_01` | 可能在等待子进程中阻塞 |

---

## 5. 未实现的关键系统调用汇总

以下是 LTP 测试中遇到的所有返回 ENOSYS 的系统调用，按影响范围排序：

| 系统调用 | 编号 | 影响测试数 | 实现难度 | 说明 |
|---------|------|-----------|---------|------|
| ~~fchmodat~~ | 53 | ~70 | **已修复** (stub) | |
| ~~fchownat~~ | 54 | ~10 | **已修复** (stub) | |
| ~~setpgid~~ | 154 | ~80 | **已修复** (stub) | |
| `mknodat` | 33 | 5+ | 中等 | 创建设备文件/FIFO |
| `symlinkat` | 36 | 5+ | 中等 | 创建符号链接 |
| `brk` (语义) | 214 | 2 | 中等 | 需改为 Linux 标准语义 |
| `seteuid/setreuid` | 145/147 | 5+ | 高 | 多用户权限模型 |
| `umask` | 166 | 3+ | 低 | 文件创建掩码 |

---

## 6. 建议的后续修复优先级

### P0 — 高收益，低难度
1. **创建 `/proc/sys/kernel/pid_max` 静态文件** — 解锁 getpid01, getppid01, getppid02
2. **实现 `mknodat` (syscall 33)** — 解锁 read03, lseek02 等
3. **补充 SD 卡镜像中缺失的 LTP 二进制** — 解锁 fork02, mkdir01 等约 9 个测试

### P1 — 中等收益，中等难度
4. **实现 Linux 标准 `brk` 语义** — 解锁 brk01, brk02
5. **实现 `symlinkat` (syscall 36)** — 解锁 openat02 等
6. **实现基于文件的 `mmap(MAP_SHARED)`** — 解锁 checkpoint 机制，解锁 pipe02, fork04, execve05

### P2 — 低优先级，高难度
7. **实现 procfs** — 完整的 /proc 文件系统支持
8. **实现多用户权限模型** — seteuid, setregid, getpwnam 等
9. **实现 mount_device / loopback** — 解锁 chdir01 等需要挂载设备的测试

---

## 7. 总结

通过本次 LTP 适配工作，共新增/修改 6 个文件，实现了 5 个系统调用 stub（fchmodat, fchmod, fchownat, fchown, setpgid），完成了 initcode 的 LTP 直测改造。

当前已验证通过 **28 个 LTP 测试用例**，覆盖了进程管理（fork/wait/exit/clone）和基本 I/O（read/write/open/close/dup/pipe/lseek）两大核心功能区域。

剩余失败用例的主要瓶颈为：缺少 procfs、brk 语义不兼容、缺少 symlink/mknod 系统调用、以及 LTP checkpoint 机制依赖的共享内存支持。这些问题都有明确的修复路径，可按优先级逐步解决。
