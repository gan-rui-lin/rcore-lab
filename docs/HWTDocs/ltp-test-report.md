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

---

## 6. 2026-03-21 最新进度修正

本节**优先于前文旧统计**。  
原因是 2026-03-17 之后已经更换了新的 SD 镜像，之前关于 `exit01/exit02` 的“镜像读取异常”判断不再成立；同时 `initcode.rs` 的 LTP 驱动也进一步调整了批量运行策略。

### 6.1 新镜像带来的结论修正

- 旧镜像中确实存在 LTP 二进制损坏现象，典型表现是：
  - `getitimer01` / `exit01` 被读成目录
  - `getitimer02` / `exit02` 被读成极小文本文件
- 更换新镜像后，这类问题已经消失。
- 因此，前文把 `exit01` / `exit02` 归因到 ext4 路径解析或 inode 读取错误的结论，**现在应视为已过期**。

### 6.2 新镜像下已确认单测通过

以下用例在新镜像下已经重新单独验证通过，判定口径仍然是 `TPASS/TFAIL/TBROK`：

**进程/时间/系统信息类**
- `waitpid01`
- `waitpid03`
- `clone01`
- `exit01`
- `exit02`
- `times01`
- `sysinfo01`
- `sysinfo02`
- `uname01`
- `uname02`
- `newuname01`
- `clock_gettime01`
- `sched_getaffinity01`
- `setitimer01`
- `setitimer02`
- `getitimer01`
- `getitimer02`

说明：
- `waitpid01` 在新镜像下单测完整通过，日志中可见 `passed 146 / failed 0 / broken 0`
- `exit01` / `exit02` 已经不再受旧镜像损坏影响，当前是可执行且可通过的
- `getitimer01` / `getitimer02` 同样在新镜像下恢复正常

### 6.3 initcode 批量运行策略已更新

当前 [`user/src/bin/initcode.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/user/src/bin/initcode.rs) 中的 LTP 批量驱动做了两点重要调整：

1. 仍保留“只跑当前人工筛过的一批 LTP 用例”的思路。
2. 默认关闭内部 per-test watchdog：
   - `const ENABLE_LTP_WATCHDOG: bool = false;`
   - 原因不是这些用例本身会无限挂住，而是 watchdog 子进程会并发执行 `sleep()/gettimeofday()/yield()`，反而干扰 `waitpid01`、`waitpid03`、`write03` 这类本来单测可过的用例。

这意味着：
- 当前这批 `initcode` 里的 LTP 更适合当作“已筛选稳定集”的顺序回归入口
- 如果后续要继续扩大到更多未知用例，再考虑恢复更稳妥的超时机制

### 6.4 最新批量回归现状

在关闭内部 watchdog 后，批量运行已经稳定越过之前最明显的卡点：

**已确认在批量中继续通过**
- `getpid02`
- `fork01`
- `fork03`
- `wait01`
- `wait02`
- `wait401`
- `waitpid01`
- `waitpid03`
- `clone01`
- `clone02`
- `pipe01`
- `read01`
- `read02`
- `read04`
- `write01`
- `write02`

**最新批量中暴露出的新问题**
- `clone03`
  - 在最近一轮“纯净批量回归”中返回 `status=0x8b`
  - 这表示它当前更像是**批量场景下的回归**，不是单测语义失败
  - 由于此前 `clone03` 单测已经通过，后续应优先按“前序用例污染状态 / 批量上下文交互”来排查

**仍在继续观察的后续批量项**
- `write03`
- `write05`
- `close01`
- `close02`
- `dup01`
- `dup02`
- `dup201`
- `dup202`
- `dup203`
- `open01`
- `lseek01`

说明：
- 这些用例里，有不少此前已经单独验证通过
- 但最新批量日志在 `clone03` 之后开始出现新的不稳定点，因此需要重新做一轮“批量口径”的确认

### 6.5 当前最值得继续投入的方向

1. 先修复 `clone03` 的批量回归。
   - 它目前是“单测能过、批量失败”的典型代表。
   - 一旦这里稳定，后面的 `write03/write05/open01/lseek01` 批量通过率大概率还能再涨一截。

2. 把“单测已通过、批量未重新确认”的用例分层记录。
   - 避免像旧报告那样把“单测通过”和“批量稳定通过”混在一起。

3. 后续如果要扩大 LTP 覆盖面，优先继续捡已有 syscall 适配成果附近的低垂果实：
   - `getitimer/setitimer`
   - `sysinfo/times/uname`
   - `sched_getaffinity`
   - `clock_gettime`

### 6.6 当前阶段结论

到 2026-03-21 为止，可以比较明确地下结论：

- **新镜像已经解决了旧镜像损坏导致的伪失败问题**
- **`waitpid01/waitpid03` 这类长用例的主要障碍一度来自测试驱动 watchdog，而不是内核主体语义**
- **当前真正需要继续攻坚的是“批量上下文下的新回归”，代表用例是 `clone03`**

因此，后续 HWTDocs 中的统计建议分成两栏维护：
- `单测确认通过`
- `批量稳定通过`

这样能更准确反映当前 rCore 的真实适配进度。

## 7. 2026-03-21 批量回归补充定位

### 7.1 本轮新增代码改动

- 顶层 [`Makefile`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/Makefile) 已补上 `OFFLINE=$(OFFLINE)` 透传：
  - `rv`
  - `la`
  - `debug`
- [`user/src/bin/initcode.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/user/src/bin/initcode.rs) 新增 `LTP_PROFILE`：
  - 默认 `stable`
  - `clone-repro`
  - `batch-repro`
- [`user/src/bin/initcode.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/user/src/bin/initcode.rs) 的 LTP 主循环不再为每个用例临时分配 `Vec` 拼接路径和 argv，而是改为栈上固定缓冲区，目的是减少长批量下驱动自身堆分配噪音。

### 7.2 本轮重新确认的单测结果

以下用例本轮都重新单独验证通过：

- `clone03`
- `read04`
- `write05`
- `dup02`

这说明这几项当前的问题都更偏向“批量上下文中的状态污染 / 清理问题”，而不是单个 syscall 基本语义彻底错误。

### 7.3 缩小复现结果

#### `clone-repro`

编译方式：

```bash
LTP_PROFILE=clone-repro OFFLINE=1 make rv
```

对应批量序列：

- `clone01`
- `clone02`
- `clone03`

结果：

- 3 项全部通过

结论：

- `clone03` 的批量异常**不是**由 `clone01/clone02` 两个前序 clone 用例直接触发的。

#### `batch-repro`

编译方式：

```bash
LTP_PROFILE=batch-repro OFFLINE=1 make rv
```

对应批量序列：

- `waitpid01`
- `waitpid03`
- `clone01`
- `clone02`
- `clone03`
- `read04`
- `write05`
- `dup02`

第一次明确复现到的结果是：

- `waitpid01` 通过
- `waitpid03` 通过
- `clone01` 通过
- `clone02` 通过
- `clone03` 通过
- 刚进入 `read04` 时内核 panic

panic 已定位到：

- [`arch/src/riscv64/mm/page_table.rs:190`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/arch/src/riscv64/mm/page_table.rs:190)

该位置对应：

- `PageTable::unmap()` 中对 `find_pte(vpn).unwrap()` 的假设失败
- 实质上是“应该被解除映射的页，在页表里已经不是有效 PTE”这一类不一致

### 7.4 当前更可信的根因方向

基于本轮日志，当前更像是：

- 长批量下某个地址空间区域的 `MapArea` 元数据和真实页表状态发生了不同步

优先怀疑的内核路径：

1. [`os/src/syscall/process.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/syscall/process.rs) 中的 `sys_munmap()`
   - 目前只按 `start_vpn` 删除整个 area
   - 没有真正按 `[start, len)` 处理

2. [`os/src/mm/memory_set.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/mm/memory_set.rs) 中的 `change_protection()`
   - 当前只改覆盖页的 PTE flags
   - 不会拆分 `MapArea`

3. LTP guarded buffer 机制
   - `tst_buffers` 使用 `mmap + mprotect(PROT_NONE) + munmap`
   - 很容易把 area 元数据与页表状态不一致的问题放大出来

### 7.5 当前阶段结论修正

到这一步，之前“`clone03` 是当前主要批量 blocker”的结论需要修正为：

- `clone03` 本身并不是当前最核心的问题
- 更核心的问题是：
  - **长批量回归下的虚存区域清理/解除映射一致性**
- 第一处已经稳定打到的断点是：
  - [`arch/src/riscv64/mm/page_table.rs:190`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/arch/src/riscv64/mm/page_table.rs:190)

说明：

- 本轮最后一次在去掉 `initcode` 内层 `Vec` 分配后继续复跑 `batch-repro`，还在持续观察中。
- 最新一次带该改动的 `batch-repro` 在 `clone02` 处超时，尚未再次跑到之前的 `read04` panic 点。
- 因此这里先只记录**已经确认**的结论，不把尚未跑完的新现象写成定论。

## 8. 2026-03-21 内存权限语义补强

### 8.1 本轮新增代码改动

- [`os/src/mm/memory_set.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/mm/memory_set.rs)
  - 新增 `MmapMeta`，为 `MapArea` 记录 `MAP_SHARED` / 文件映射 / 原始 fd 是否可写
  - 新增 `ProtectError`
  - `change_protection()` 不再只是“扫一遍重写 PTE”，而是会先检查整段是否完整映射，再按需拆分 `MapArea`，保证区域元数据与页表权限同步
- [`os/src/syscall/process.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/syscall/process.rs)
  - `sys_mmap()` 改为走 `insert_mmap_area()`，把映射来源信息带进地址空间
  - `sys_mmap()` 增加了更严格的参数校验，并允许 `/dev/zero` 这类“无普通 inode 的文件映射”成功建立零页映射
  - `sys_mprotect()` 对 `addr == 0` 返回 `ENOMEM`
  - `sys_mprotect()` 现在能区分：
    - 未映射区间 -> `ENOMEM`
    - 只读共享文件映射提权到 `PROT_WRITE` -> `EACCES`
- [`os/src/fs/stdio.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/fs/stdio.rs)
  - `DevNull` / `DevZero` 改为携带“本次打开的读写权限”
- [`os/src/syscall/fs.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/os/src/syscall/fs.rs)
  - `/dev/null`、`/dev/zero` 在 `openat()` 时按 `O_RDONLY/O_WRONLY/O_RDWR` 生成对应权限的设备文件对象
- [`user/src/bin/initcode.rs`](/home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab/user/src/bin/initcode.rs)
  - 已把当前新确认通过的内存管理类用例加入 `stable` 集合：
    - `mmap01`
    - `munmap01`
    - `mprotect01`
    - `mprotect02`
    - `mprotect03`

### 8.2 本轮新增确认通过

以下用例均已重新单独验证，且日志中出现明确 `TPASS`：

- `mmap01`
- `munmap01`
- `mprotect01`
- `mprotect02`
- `mprotect03`

其中 `mprotect01` 三个子场景已经全部对齐：

- `mprotect(NULL, ...)` -> `ENOMEM`
- 非页对齐地址 -> `EINVAL`
- 对 `O_RDONLY` 打开的 `/dev/zero` 的 `MAP_SHARED` 只读映射执行 `mprotect(..., PROT_WRITE)` -> `EACCES`

### 8.3 当前累计进度

基于此前已确认的 28 项，加上本轮新增确认通过的 5 项，当前**至少**已有 33 项 LTP 用例可以稳定单独通过：

- 原有进程管理 + 基本 I/O：28 项
- 新增内存管理：`mmap01`、`munmap01`、`mprotect01`、`mprotect02`、`mprotect03`

### 8.4 仍未纳入统计的项目

- `mmap03`
  - 当前单独运行时退出状态是 `0`
  - 但日志中仍未看到明确 `TPASS/TFAIL/TBROK` 正文
  - 暂不计入通过数，后续继续单独复核
