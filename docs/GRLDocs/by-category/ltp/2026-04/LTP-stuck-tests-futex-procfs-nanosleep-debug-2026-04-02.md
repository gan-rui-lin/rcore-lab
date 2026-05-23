# LTP 测试卡死问题深度调试报告

**日期**: 2026/4/2  
**问题类型**: 测试卡死、Futex 队列失配、/proc 状态异常、参数校验缺失  
**影响范围**: futex_wait03、futex_wake02、clock_nanosleep01 等 LTP 核心测试

---

## 一、问题现象

在运行 LTP (Linux Test Project) 测试套件时，多个测试用例出现"卡住"现象，即使设置了 60 秒超时仍无法完成，表现为：

1. **futex_wake02**: 测试创建 55 个线程后卡死，既无 TPASS 也无超时退出
2. **futex_wait03**: 测试启动后立即卡住，子进程无限等待父进程状态变化
3. **clock_nanosleep01**: 第一个测试用例（`tv_nsec = -1`）直接睡死，30 秒超时后被 LTP 框架 SIGKILL

初始日志关键线索：
```
clock_nanosleep01.c:139: TINFO: case NORMAL
[ WARN] [itimer] pid=3 SIGALRM fired, expire=36455 now=36463
Test timeouted, sending SIGKILL!
tst_test.c:1654: TBROK: waitpid(4,0x7fffffcc8,0) failed: EINTR (4)
```

---

## 二、调试过程与根因分析

### 2.1 futex_wake02 卡死：Private Futex Key 失配

#### 问题定位

在 `futex_wake02` 中添加定向日志后发现关键异常：

```
[sys_futex] pid=5 tid=1 name=futex_wake02 cmd=FUTEX_WAIT uaddr1=0x40028cd0 pa=0x887cbcd0 private=true val=0
[sys_futex] pid=5 tid=2 name=futex_wake02 cmd=FUTEX_WAIT uaddr1=0x40028cd0 pa=0x887cbcd0 private=true val=0
...
[sys_futex] pid=5 tid=0 name=futex_wake02 cmd=FUTEX_WAKE uaddr1=0x40028cd0 pa=0x890eecd0 private=true val=1
[futex] wake call pid=5 tid=0 key=(0x40028cd0,5) max=1 bitset=0xffffffff woke=1 qlen_before=55
```

**关键发现**：
- **Wait 时**：`uaddr=0x40028cd0` 映射到 `pa=0x887cbcd0`（55 个线程）
- **Wake 时**：同样的 `uaddr=0x40028cd0` 映射到 `pa=0x890eecd0`（主线程）
- 结果：`woke=1`（只唤醒了 1 个线程，其余 54 个线程的队列 key 不匹配）

#### 根因分析

rCore-lab 的 futex 实现使用 **物理地址 + pid** 作为队列 key：

```rust
// 旧代码 (os/src/syscall/process.rs:333)
let pa = page_table.translate_va(VirtAddr::from(uaddr1 as usize))?;
let pid = if private { current_process().pid.0 } else { 0 };
let key = FutexKey::new(pa, pid);  // ❌ private futex 也用物理地址
```

但在多线程场景下，**COW (Copy-On-Write) 机制**会导致物理页变化：

1. `clone(CLONE_VM)` 创建线程时，虚拟地址空间共享，页表项设为只读并标记 COW
2. 主线程在 futex word 所在页首次写入时，触发 COW，内核分配新物理页 `0x890ee000`
3. 子线程仍映射到旧物理页 `0x887cb000`
4. Wake 时主线程用新 PA 查队列，子线程在旧 PA 队列中无法被唤醒

#### 修复方案

Linux 的 private futex 语义是"**进程内虚拟地址唯一标识**"，不应受物理页变化影响：

```rust
// 修复后 (os/src/syscall/process.rs:336-340)
let key_addr = if private {
    crate::mm::PhysAddr::from(uaddr1 as usize)  // ✅ private 用虚拟地址
} else {
    pa  // shared futex 仍用物理地址（跨进程共享）
};
let key = FutexKey::new(key_addr, pid);
```

**技术要点**：
- Private futex 的 `FUTEX_PRIVATE_FLAG` 标志位意味着"仅当前进程可见"
- 虚拟地址在进程内唯一，不受页表映射变化影响
- Shared futex 必须用物理地址，因为不同进程的虚拟地址可能映射到同一共享内存

---

### 2.2 futex_wait03 卡死：/proc 状态导出错误

#### 问题定位

`futex_wait03` 的测试逻辑是：
1. 主线程调用 `futex_wait` 进入阻塞
2. 子线程轮询 `/proc/<pid>/stat` 等待主线程状态变为 `S` (Sleeping)
3. 确认主线程已阻塞后，调用 `futex_wake` 唤醒

但实际观察到：

```c
// LTP 源码片段 (tst_sig_proc.h)
TST_PROCESS_STATE_WAIT(getpid(), 'S', 10000);  // 等待进程变成 S 状态
// 轮询逻辑：while (read_proc_state(pid) != 'S') usleep(1000);
```

日志显示子线程一直轮询，永远等不到 `S` 状态。

#### 根因分析

查看 `/proc/<pid>/stat` 的实现发现问题：

```rust
// 旧代码 (os/src/fs/vfs/procfs.rs)
let state = if inner.tasks.iter().any(|t| 
    t.inner_exclusive_access().task_status == TaskStatus::Running
) {
    'R'  // ❌ 只要有一个线程 Running，整个进程就是 R
} else {
    'S'
};
```

**问题**：
- `futex_wait03` 有 2 个线程：主线程 (tid=0) 阻塞在 futex，子线程 (tid=1) Running 并轮询
- 由于子线程是 Running，`/proc/<pid>/stat` 始终返回 `R`
- 子线程等不到 `S`，形成死循环

**正确语义**：
- `/proc/<pid>/stat` 应该反映**线程组 leader (tid==0)** 的状态
- `/proc/<pid>/task/<tid>/stat` 反映单个线程状态
- Linux 的 `TST_PROCESS_STATE_WAIT` 期望的是主线程状态，不是整个进程

#### 修复方案

1. **修改 `/proc/<pid>/stat` 导出逻辑**（仅导出 leader 状态）：

```rust
// 修复后
let state = if let Some(leader_task) = inner.tasks.iter().find(|t| {
    t.inner_exclusive_access().res.as_ref().map(|r| r.tid == 0).unwrap_or(false)
}) {
    match leader_task.inner_exclusive_access().task_status {
        TaskStatus::Running => 'R',
        _ => 'S',
    }
} else {
    'S'
};
```

2. **新增 `/proc/<pid>/task/<tid>/stat` 支持**（满足线程级查询）：

```rust
// 新增路径解析
if path_parts.len() == 5 && path_parts[2] == "task" {
    let tid: usize = path_parts[3].parse().ok()?;
    // 返回指定 tid 的 stat
}
```

---

### 2.3 clock_nanosleep01 睡死：参数校验缺失

#### 问题定位

测试第一个用例就卡住，日志显示进程持续运行直到 LTP 30 秒超时杀死：

```
clock_nanosleep01.c:139: TINFO: case NORMAL
[ WARN] [itimer] pid=3 SIGALRM fired, expire=36455 now=36463
Test timeouted, sending SIGKILL!
```

查看测试用例源码发现触发条件：

```c
// ltp-full-20240524/testcases/kernel/syscalls/clock_nanosleep/clock_nanosleep01.c:52-59
{
    TYPE_NAME(NORMAL),
    .clk_id = CLOCK_REALTIME,
    .flags = 0,
    .tv_sec = 0,
    .tv_nsec = -1,  // ❌ 非法值，期望返回 EINVAL
    .exp_ret = -1,
    .exp_err = EINVAL,
}
```

#### 根因分析

rCore-lab 的 `TimeSpec` 定义使用 `usize`（无符号类型）：

```rust
#[repr(C)]
pub struct TimeSpec {
    pub tv_sec: usize,
    pub tv_nsec: usize,  // ❌ usize 无法表示负数
}
```

当 C 代码传入 `tv_nsec = -1` 时：
- 二进制表示为 `0xFFFFFFFFFFFFFFFF`
- 被解释为 `usize::MAX` (约 18 艾字节纳秒)
- 转换为微秒后仍是天文数字，导致 `while get_time_us() < target` 无限循环

**旧代码缺失校验**：

```rust
// os/src/syscall/process.rs:1766
pub fn sys_nanosleep(req: *const TimeSpec, rem: *mut TimeSpec) -> isize {
    let req = read_from_user(token, req)?;
    // ❌ 没有检查 tv_nsec 是否 >= 1e9
    let sleep_us = req.tv_sec * 1_000_000 + req.tv_nsec / 1_000;
```

#### 修复方案

1. **添加参数校验**（符合 POSIX 标准）：

```rust
pub fn sys_nanosleep(req: *const TimeSpec, rem: *mut TimeSpec) -> isize {
    let req = read_from_user(token, req)?;
    if req.tv_nsec >= 1_000_000_000 {
        return errno(EINVAL);  // ✅ 拒绝非法纳秒值
    }
    // ...
}
```

2. **增强 `sys_clock_nanosleep` 校验**：

```rust
pub fn sys_clock_nanosleep(
    clock_id: usize,
    flags: usize,
    req: *const TimeSpec,
    rem: *mut TimeSpec,
) -> isize {
    match clock_id {
        0..=11 => {}  // ✅ 支持常见时钟 ID
        _ => return errno(EINVAL),
    }
    if clock_id == 3 {  // CLOCK_THREAD_CPUTIME_ID
        return errno(ENOTSUP);  // ✅ 明确拒绝不支持的时钟
    }
    if flags != 0 {
        return errno(EINVAL);  // ✅ 当前仅支持相对睡眠
    }
    sys_nanosleep(req, rem)
}
```

3. **修复 EINTR 路径的 EFAULT 处理**：

```rust
// 旧代码在信号中断时吞掉了 copy_to_user 错误
if !rem.is_null() {
    let _ = copy_to_user(token, rem, &remain)?;  // ❌ 忽略 EFAULT
}
return errno(EINTR);

// 修复后
if !rem.is_null() {
    if let Err(err) = copy_to_user(token, rem, &remain) {
        return err;  // ✅ EFAULT 优先于 EINTR
    }
}
return errno(EINTR);
```

**LTP 测试期望**：
- `BAD_TS_ADDR_REM` 用例传入非法 `rem` 指针，触发信号中断后应返回 `EFAULT` 而非 `EINTR`
- 这符合 Linux 的错误优先级：地址错误 > 信号中断

---

### 2.4 waitpid EINTR 异常：LTP 超时机制失效

#### 问题定位

`clock_nanosleep01` 修复后仍偶尔出现：

```
tst_test.c:1654: TBROK: waitpid(4,0x7fffffcc8,0) failed: EINTR (4)
```

这是 LTP 框架层的问题。查看 LTP 源码：

```c
// lib/tst_test.c:1631
static int fork_testrun(void) {
    alarm(results->timeout);  // 设置 30 秒超时
    test_pid = fork();
    if (!test_pid) {
        testrun();  // 子进程运行测试
    }
    SAFE_WAITPID(test_pid, &status, 0);  // ❌ 父进程等待
    alarm(0);
    // ...
}
```

**预期行为**：
- 30 秒后 `SIGALRM` 触发，`alarm_handler` 向子进程发送 `SIGKILL`
- 子进程退出后 `waitpid` 正常返回
- 父进程检查 `WIFSIGNALED(status) && WTERMSIG(status) == SIGKILL`

**实际行为**：
- `SIGALRM` 到达后，`waitpid` 被中断并返回 `-EINTR`
- LTP 框架误判为系统调用失败，报 `TBROK`

#### 根因分析

rCore-lab 的 `sys_waitpid` 实现过于激进地返回 `EINTR`：

```rust
// 旧代码片段
loop {
    // 检查是否有僵尸子进程
    if let Some(child) = find_zombie() {
        return reap(child);  // 正常返回
    }
    suspend_current_and_run_next();  // 让出 CPU
    
    // ❌ 只要有未屏蔽信号就返回 EINTR
    if has_pending_signal() {
        return errno(EINTR);
    }
}
```

**问题**：
- `SIGCHLD`（子进程状态变化信号）会打断 `waitpid`
- `SIGALRM` 即使设置了 `SA_RESTART` 也会中断
- LTP 的超时机制依赖 `SIGALRM` 能触发 handler 杀死子进程，但 `waitpid` 提前返回导致流程异常

#### 修复方案

参考 Linux 语义，`waitpid` 返回 `EINTR` 的条件应该非常严格：

```rust
// 修复后 (os/src/syscall/process.rs:1586-1622)
let unmasked = (process_pending | task_pending) & !signal_mask;
if !unmasked.is_empty() {
    let actions = &process_inner.signal_actions;
    let mut needs_eintr = false;
    
    for bit in 0..64 {
        if raw & (1u64 << bit) != 0 {
            let signum = bit + 1;
            let action = &actions.table[signum];
            
            // 1. 忽略 SIG_DFL 和 SIG_IGN
            if action.handler == SIG_DFL || action.handler == SIG_IGN {
                continue;
            }
            
            // 2. SIGCHLD 不应中断 waitpid（正在等待子进程）
            if signum == SIGCHLD {
                continue;
            }
            
            // 3. 定时信号必须中断（LTP 超时机制依赖此行为）
            if signum == SIGALRM || signum == SIGVTALRM || signum == SIGPROF {
                needs_eintr = true;
                break;
            }
            
            // 4. 其他信号检查 SA_RESTART 标志
            if (action.flags & SA_RESTART) != 0 {
                continue;  // 不中断，让 waitpid 自动重启
            }
            
            needs_eintr = true;
            break;
        }
    }
    
    if needs_eintr {
        return errno(EINTR);
    }
}
```

**关键点**：
- **定时信号特殊处理**：`SIGALRM` 等必须强制返回 `EINTR`，确保用户态 handler 能执行
- **SIGCHLD 过滤**：`waitpid` 本身就是在等子进程事件，不应被 `SIGCHLD` 打断
- **SA_RESTART 语义**：设置了该标志的信号应该让系统调用自动重启，不返回 `EINTR`

---

## 三、技术背景知识

### 3.1 Futex (Fast Userspace Mutex) 机制

Futex 是 Linux 实现高效同步的核心机制：

**Private vs Shared**:
- **Private** (`FUTEX_PRIVATE_FLAG`): 仅当前进程内可见，用虚拟地址唯一标识
- **Shared**: 跨进程共享（如共享内存），用物理地址标识

**COW 与 Futex 的交互**:
```
进程 A fork() -> 进程 B
初始：A 和 B 共享只读页表项（COW 标记）
       PA=0x1000 <-- A:VA=0x4000, B:VA=0x4000

A 写入 futex word:
       PA=0x1000 <-- B:VA=0x4000
       PA=0x2000 <-- A:VA=0x4000  (新分配)

如果用 PA 做 key:
  A 的 FUTEX_WAKE(VA=0x4000) -> key=(PA=0x2000, pid=A)
  B 的 FUTEX_WAIT(VA=0x4000)  -> key=(PA=0x1000, pid=B)
  结果：唤醒失败！
```

### 3.2 Linux /proc 文件系统约定

- `/proc/<pid>/stat`: 线程组 leader 的状态（主线程）
- `/proc/<pid>/task/<tid>/stat`: 单个线程状态
- `/proc/<pid>/status`: 进程级汇总信息

**状态字符映射**:
```
R: Running (on CPU)
S: Sleeping (可中断睡眠)
D: Disk sleep (不可中断睡眠，如等待 I/O)
Z: Zombie (已退出但未被回收)
T: Stopped (被信号停止)
```

### 3.3 POSIX 时间接口规范

**nanosleep / clock_nanosleep 参数约束**:
```c
struct timespec {
    time_t tv_sec;   // 秒，可以是负数（表示过去时间）
    long tv_nsec;    // 纳秒，必须在 [0, 999999999] 范围内
};
```

**错误码优先级**:
1. `EFAULT`: 地址错误（最高优先级）
2. `EINVAL`: 参数无效
3. `EINTR`: 被信号中断（最低优先级）

---

## 四、验证结果

### 最终测试执行日志

```bash
SINGLE_TEST=tmp-ltp-stuck LOG=INFO timeout 60 bash run.sh -f sdcard-rv.img
```

**futex_wait03** (1 TPASS):
```
futex_wait03.c:56: TPASS: futex_wait() woken up
```

**futex_wake02** (12 TPASS):
```
futex_wake02.c:91: TPASS: futex_wake() woken up 1 threads
futex_wake02.c:91: TPASS: futex_wake() woken up 2 threads
...
futex_wake02.c:91: TPASS: futex_wake() woken up 10 threads
futex_wake02.c:103: TPASS: futex_wake() woken up 0 threads
```

**clock_nanosleep01** (10 TPASS):
```
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EINVAL (22)   // tv_nsec = -1
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EINVAL (22)   // tv_nsec = 1e9
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EOPNOTSUPP (95) // CLOCK_THREAD_CPUTIME_ID
clock_nanosleep01.c:208: TPASS: Timespec updated correctly                    // EINTR 路径
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EINTR (4)
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EFAULT (14)   // BAD_TS_ADDR_REQ
clock_nanosleep01.c:218: TPASS: clock_nanosleep() failed with: EFAULT (14)   // BAD_TS_ADDR_REM
```

**脚本执行统计**:
- 运行时间: 约 45 秒（未触发 60 秒超时）
- 退出状态: `=== ltp-stuck debug done ===`
- 错误日志: 无 `IllegalInstruction`、`StorePageFault`、`Panicked`
- 剩余问题: `futex_cmp_requeue02` 的 2 个 TFAIL（独立语义问题，非卡死）

---

## 五、修改文件清单

| 文件路径 | 修改内容 | 行数 |
|---------|---------|------|
| `os/src/syscall/process.rs` | Private futex 改用虚拟地址 key | ~15 |
| `os/src/syscall/process.rs` | sys_nanosleep 参数校验 | ~5 |
| `os/src/syscall/process.rs` | sys_clock_nanosleep 增强校验 | ~20 |
| `os/src/syscall/process.rs` | waitpid EINTR 条件细化 | ~40 |
| `os/src/syscall/process.rs` | EINTR 路径 rem EFAULT 处理 | ~5 |
| `os/src/fs/vfs/procfs.rs` | /proc/stat 按 leader 导出 | ~30 |
| `os/src/fs/vfs/procfs.rs` | 新增 /proc/task/tid/stat | ~60 |
| `os/src/timer.rs` | check_itimers 唤醒阻塞任务 | ~35 |
| `user/src/bin/initcode.rs` | 添加 TMP_LTP_STUCK_SCRIPT | ~25 |

**总代码变更**: 约 235 行

---

## 六、经验总结

### 关键教训

1. **futex key 设计**：Private 语义必须对 COW 免疫，虚拟地址是唯一正确选择
2. **多线程状态导出**：/proc 接口应区分进程级和线程级视图
3. **参数校验的重要性**：usize 与有符号整数的语义鸿沟需要显式桥接
4. **系统调用中断语义**：EINTR 返回条件极其微妙，需要精确匹配 POSIX 标准

### 调试技巧

1. **定向日志 + 进程名过滤**：避免海量日志淹没关键信息
   ```rust
   if name == "futex_wake02" { info!("..."); }
   ```

2. **物理地址追踪**：在怀疑页表映射问题时，同时打印 VA 和 PA
   ```rust
   info!("uaddr={:#x} pa={:#x}", uaddr, pa.0);
   ```

3. **参考实现对比**：与成熟内核 (`T202410487992457-1800`) 逐行对比关键路径

4. **测试源码溯源**：理解测试期望比猜测内核行为更高效
   ```
   /home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524/testcases/...
   ```

### 后续优化方向

1. **Futex 性能优化**：当前每次 wait/wake 都加全局锁，可改用分段锁
2. **/proc 接口完善**：实现 `/proc/<pid>/task/` 目录列表（当前仅支持单文件）
3. **EINTR 自动重启**：在 trap 层统一处理 `SA_RESTART`，减少 syscall 层重复逻辑
4. **参数校验框架**：抽取通用校验函数（如 `validate_timespec`）
