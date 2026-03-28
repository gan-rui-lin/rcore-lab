# musl pthread TID=0 互斥锁缺陷修复

日期: 2026/3/28

## 罪魁祸首

**musl libcbench 的 3 个 pthread 测试 SIGSEGV 的根本原因是：内核 `sys_set_tid_address` 和 `sys_gettid` 对主线程返回 TID=0，而 musl 的 `__tl_lock` 使用 `CAS(&lock, 0, tid)` 实现互斥——`0` 是"未锁"哨兵值。当 `tid=0` 时，"已锁"状态与"未锁"状态不可区分，互斥彻底失效。**

这不是 musl 的 bug，也不是 COW 或信号的问题。是内核违反了 Linux 语义中"TID 必须为正整数"的隐含契约。

---

## 1. 问题背景

### 1.1 libcbench 的执行模型

libcbench 包含 27 个子测试，每个在 `fork()` 出的子进程中运行：

```c
int run_bench(const char *label, size_t (*bench)(void *), void *params) {
    pid_t p = fork();          // 每个 benchmark fork 一个子进程
    if (p) { wait(&status); return status; }
    bench(params);             // 子进程执行 benchmark
    exit(0);
}
```

其中 4 个 pthread 测试的行为差异对触发 bug 至关重要：

| 测试 | 线程模式 | 是否触发 bug |
|------|---------|-------------|
| `createjoin_serial1` | 创建 1 个线程 → join → 重复 2500 次 | 偶发（PID 碰撞时） |
| `createjoin_serial2` | 批量创建 50 个线程 → join 50 个 → 重复 50 次 | 必现 |
| `create_serial1` | 创建 2500 个线程（不 join，16KB 小栈） | 必现 |
| `uselesslock` | 创建线程做 mutex lock/unlock，`exit_group` 退出 | 不触发（不走 `__pthread_exit`） |

### 1.2 崩溃现象

RV 崩溃日志：
```
[ERROR] trap_handler: page fault addr=0x10 sepc=0x1c74c ra=0x1c6f8 sp=0x6004c8e08 tp=0x6004c8fe8
```

LA 崩溃日志：
```
[ERROR] trap_handler: page fault addr=0x8 pid=10 tid=1 name=libc-bench sepc=0x12000eabc
```

地址 `0x10` (RV) 和 `0x8` (LA) 说明是对 NULL 指针加偏移的访问——`*(NULL + 16)` 或 `*(NULL + 8)`。

### 1.3 前期排查（已排除的方向）

在找到真正根因之前，经历了以下排查：

1. **怀疑 prev/next 未初始化**：在 `sys_clone` 中读取新线程 TLS 区域，确认 clone 时 `prev=0, next=0`。这是正常的——musl 在 `pthread_create` 的父线程侧、clone 返回后才链入。
2. **尝试设置 prev=self**：在 `sys_clone` 中将新线程的 `prev` 初始化为 `self`（形成自引用环），SIGSEGV 消失但导致**死锁**——破坏了 musl 线程链表的管理假设。
3. **怀疑 COW 与 lr/sc 交互**：猜测 COW page fault 破坏 RISC-V lr/sc 原子操作的 reservation。但链入代码使用的是普通 `sd`（store doubleword），不涉及 lr/sc。

---

## 2. 二进制反汇编分析

由于 musl libc-bench 是**全裸二进制**（静态链接、完全 stripped），需要从 sdcard 镜像提取后逐条反汇编。

### 2.1 `__pthread_exit` 的完整流程（0x1c6a4）

```asm
1c6a4: addi sp, sp, -176         # 栈帧
1c6bc: mv   s0, tp               # s0 = tp（线程指针）
1c6c8: addi s2, s0, -224         # s2 = tp - 224 = self（struct pthread 基址）
1c6d0: sd   a0, -104(s0)         # 保存退出值

     # ... cleanup handlers ...

1c6f4: jal  0x1c570              # ★ __tl_lock() — 获取线程链表锁
1c6f8: ld   a4, -200(s0)         # a4 = *(tp-200) = self->next（锁内读取！）
1c6fc: bne  a4, s2, 0x1c734      # if next != self → 进入 unlink 路径

     # === unlink 路径（next != self）===
1c734: addi a5, gp, -1344        # libc 全局结构
1c738: lw   a3, 12(a5)           # threads_minus_1
1c740: addiw a3, a3, -1          # threads_minus_1--
1c744: sw   a3, 12(a5)
1c748: ld   a5, -208(s0)         # a5 = self->prev
1c74c: sd   a5, 16(a4)           # ★ *(next + 16) = prev ← 崩溃点！a4=0 时写 0x10
```

**关键发现**：`self->next` 的读取（0x1c6f8）发生在 `__tl_lock()` 之后（0x1c6f4）。如果锁正常工作，父线程在释放锁之前已完成链入，`next` 不可能为 0。

### 2.2 `pthread_create` 的链入代码（0x1cc14）

父线程在 `__clone()` 返回后、释放 `__tl_lock` 之前执行链入：

```asm
     # 父线程持有 __tl_lock，s6=current_tp，s0=new thread base
1cc14: ld  a5, -200(s6)          # a5 = current->next
1cc18: addi s6, s6, -224         # s6 = current base
1cc1c: sd  s6, 16(s0)            # new->prev = current
1cc20: sd  a5, 24(s0)            # new->next = old current->next
1cc24: sd  s0, 16(a5)            # old_next->prev = new
1cc28: ld  a5, 16(s0)            # reload new->prev
1cc2c: sd  s0, 24(a5)            # current->next = new
1cc30: jal 0x1c5fc               # __tl_unlock()
```

链入在锁保护下进行。新线程的 `__pthread_exit` 在获取同一把锁后读取 `next`，理论上应该看到有效值。

### 2.3 `__tl_lock` 的实现（还原自 musl 1.2.5）

```c
// 全局变量
static volatile int lock;      // gp-1448：锁字（0=未锁，tid=已锁）
static int count;               // gp-1472：重入计数

void __tl_lock(void) {
    int tid = __pthread_self()->tid;  // 读取 *(tp-168)
    if (count && lock == tid) {       // 重入检测
        ++count;
        return;                       // 同一线程再次加锁 → 跳过 CAS
    }
    while (a_cas(&lock, 0, tid))      // CAS: 期望 0 → 写入 tid
        __wait(&lock, 0, tid, 0);     // 失败则 futex 等待
    count = 1;
}
```

**两个关键设计假设**：
1. `0` 是"未锁"哨兵值，`tid` 是"已锁"标识
2. `tid` 必须 > 0 且进程内唯一（用于重入检测 `lock == tid`）

---

## 3. 根因定位

### 3.1 TID=0 破坏 CAS 互斥

我们的内核中：
- `sys_gettid()` 返回 `res.tid`（进程内线程索引），主线程为 **0**
- `sys_set_tid_address()` 同样返回 `res.tid = 0`
- musl 在 `__init_tp` 和 `__post_Fork` 中调用 `self->tid = __syscall(SYS_set_tid_address, &self->tid)`

当主线程 `self->tid = 0` 时：

```
主线程 __tl_lock:
  CAS(&lock, 0, 0)   →  lock = 0（写入 0，与"未锁"无法区分！）
  count = 1

新线程 __tl_lock:
  CAS(&lock, 0, N)   →  old = 0 = expected → 成功！（误以为锁未被持有）
  count = 1           →  覆盖主线程的 count（全局变量）

→ 两个线程同时进入临界区，互斥彻底失效
```

### 3.2 TID 碰撞导致重入误判

更进一步，即使用 PID 作为主线程 TID，当 `PID == internal_tid` 时仍会碰撞：

```
进程 PID=4 的主线程: set_tid_address → self->tid = 4
pthread_create 创建的第 4 个线程: PARENT_SETTID → self->tid = 4

主线程 __tl_lock:
  lock = 4, count = 1

线程 4 __tl_lock:
  tid = 4
  检查: count(1) && lock(4) == tid(4) → TRUE!
  → 重入误判，跳过 CAS，直接进入临界区
  → 读取 next = 0（父线程尚未链入）
  → *(0 + 16) = prev → PAGE FAULT at addr=0x10
```

**这解释了为什么 `serial1`（1 个线程）偶尔通过而 `serial2`（50 个线程）必定崩溃**：serial2 创建线程 1~50，当 PID 在 1~50 范围内时必然碰撞。通过内核诊断日志验证：

```
[pthread-diag] pid=4 itid=4 tid=4 next=0x0 prev=0x0
```

crashing thread 的 `itid=4`（内部线程索引 4）与 `pid=4` 碰撞，`self->tid = 4` 等于锁中存的值。

### 3.3 Linux 的 TID 语义

在 Linux 中，每个线程都有一个**全局唯一的正整数 TID**：
- 主线程的 TID = 进程 PID（通过 `clone()` 创建进程时分配）
- 每个新线程的 TID = 从全局 PID 分配器分配的唯一值
- TID **永远不为 0**（0 在 Linux 中保留，表示"无效"）

musl 的 `__tl_lock` 依赖这个语义。我们的内核使用进程内部的线程索引（从 0 开始）作为 TID，违反了这个隐含契约。

---

## 4. 修复方案

### 4.1 设计思路

使用 `system_tid = internal_tid + 1` 作为所有线程对用户空间可见的 TID：

- 主线程：`internal_tid = 0` → `system_tid = 1`
- 线程 1：`internal_tid = 1` → `system_tid = 2`
- 线程 N：`internal_tid = N` → `system_tid = N + 1`

保证：
1. **TID > 0**：所有线程的 system_tid ≥ 1，不会与 CAS 哨兵值 0 碰撞
2. **进程内唯一**：`internal_tid` 本身唯一，`+1` 后仍唯一，不会触发重入误判
3. **改动最小**：仅修改 3 个 syscall 的返回值/写入值，无需修改 PID 分配器或 TaskControlBlock

### 4.2 具体改动

| 文件 | 改动 |
|------|------|
| `os/src/syscall/thread.rs` | `sys_gettid`: 返回 `internal_tid + 1`；`sys_set_tid_address`: 返回 `internal_tid + 1` |
| `os/src/syscall/process.rs` | `sys_clone`: `PARENT_SETTID` 和 `CHILD_SETTID` 写入 `internal_tid + 1`；返回值改为 `system_tid`；`sys_tkill`: 将用户传入的 system_tid 映射回内部索引（`tid - 1`） |

关键代码（`sys_set_tid_address`）：

```rust
pub fn sys_set_tid_address(tidptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let mut task_inner = task.inner_exclusive_access();
    let tid = task_inner.res.as_ref().unwrap().tid;
    task_inner.clear_child_tid = tidptr as usize;
    drop(task_inner);
    let system_tid = tid + 1;  // 保证 > 0
    system_tid as isize
}
```

关键代码（`sys_clone` 中的 PARENT_SETTID）：

```rust
let system_tid = (new_task_tid + 1) as i32;
if clone_flags.contains(CloneFlags::PARENT_SETTID) && !ptid.is_null() {
    *translated_refmut(token, ptid) = system_tid;
}
```

关键代码（`sys_tkill` 的反向映射）：

```rust
let internal_tid = (tid as usize).wrapping_sub(1);
if internal_tid >= inner.tasks.len() || inner.tasks[internal_tid].is_none() {
    return errno(ESRCH);
}
```

### 4.3 为什么不用 PID 作为主线程 TID

第一版修复尝试让主线程返回 PID 作为 TID。对于 `serial1`（单线程）有效，但 `serial2`（50 线程）仍崩溃：新线程的 `internal_tid`（1~50）可能与 PID 碰撞。

`+1` 方案彻底消除碰撞：所有 system_tid 在 `[1, max_threads+1]` 范围内，且与 PID 无关。

---

## 5. 验证

### 5.1 RV 测试结果

修复前：
```
b_pthread_createjoin_serial1 (0)
[ERROR] trap_handler: page fault addr=0x10 sepc=0x1c74c ...   （SIGSEGV）

b_pthread_createjoin_serial2 (0)
[ERROR] trap_handler: page fault addr=0x10 sepc=0x1c74c ...   （SIGSEGV）

b_pthread_create_serial1 (0)
[ERROR] trap_handler: page fault addr=0x10 sepc=0x1c74c ...   （SIGSEGV）
```

修复后：
```
b_pthread_createjoin_serial1 (0)
  time: 5.000219000, virt: 0, res: 0, dirty: 0     ← ✅ 通过

b_pthread_createjoin_serial2 (0)
  time: 5.009791000, virt: 0, res: 0, dirty: 0     ← ✅ 通过

b_pthread_create_serial1 (0)
  time: 1.223649000, virt: 0, res: 0, dirty: 0     ← ✅ 通过
```

### 5.2 LA 测试结果

修复后 LA musl pthread×3 全部通过，glibc pthread 无回归。

```
b_pthread_createjoin_serial1 (0)    ← ✅ 通过
b_pthread_createjoin_serial2 (0)    ← ✅ 通过
b_pthread_create_serial1 (0)        ← ✅ 通过
b_pthread_uselesslock (0)           ← ✅ 通过（本来就通过）
```

### 5.3 无回归验证

glibc 版 pthread 测试（RV + LA）全部通过，`b_pthread_uselesslock` 也正常。

---

## 6. 调试方法论总结

### 6.1 完整调试路径

```
崩溃日志 (addr=0x10)
  ↓ 反汇编 sepc=0x1c74c
`sd a5, 16(a4)` where a4=next=0
  ↓ 反汇编 __pthread_exit 全流程
发现 __tl_lock 在 next 读取之前
  ↓ 反汇编 __tl_lock
理解 CAS(&lock, 0, tid) 设计
  ↓ 反汇编 pthread_create 链入代码
确认链入在 __tl_lock 保护下
  ↓ 推理：lock 应该保护 next 读取
"如果锁正常，next 不可能为 0" → 锁坏了！
  ↓ 检查 self->tid 来源
sys_set_tid_address 返回 0 (internal_tid=0)
  ↓ 理解 CAS(0,0) 的后果
TID=0 → 互斥失效 → 竞态 → next=0 → SIGSEGV
  ↓ 第一版修复：PID 作 TID
serial1 通过，serial2 仍崩溃
  ↓ 内核诊断日志
发现 itid=4, tid=4, pid=4 → TID 碰撞 → 重入误判
  ↓ 最终修复：internal_tid + 1
全部通过
```

### 6.2 关键转折

1. **从"数据未初始化"到"锁失效"**：最初以为 `prev/next = 0` 是因为 musl 未初始化。但反汇编证明读取发生在锁之后——问题不在数据，而在锁本身。
2. **从"PID 碰撞"到"+1 方案"**：第一版修复（PID 作 TID）只解决了 TID=0 问题，但引入了 PID 与 internal_tid 碰撞的新问题。内核诊断日志（`itid=4, tid=4, pid=4`）直接指出碰撞点。

### 6.3 复用的调试工具

- **riscv64-unknown-elf-objdump**：反汇编 stripped binary 的唯一手段
- **内核 page fault handler 诊断**：在 `handle_user_page_fault` 中添加 TLS 字段读取，将崩溃现场的 `self->tid`, `next`, `prev` 值直接输出
- **debugfs**：从 ext4 sdcard 镜像中提取二进制（macOS 不能直接 mount ext4）

---

## 7. 影响评估

| 项 | 数值 |
|----|------|
| 修复的 0 分项 | musl pthread×3 on RV + LA = **6 项** |
| 预估提分 | 6 项 × ~1.0 分 ≈ **+6 分** |
| 结合前序修复（memfd + TLB） | libcbench 0 分项从 14 → **0** |
| 总提分（本分支全部修复） | 约 **+55 分** |

分支 `fix/libcbench-score`，commit `657fcca`。
