# musl pthread libcbench SIGSEGV 调试与 libcbench 提分修复记录

日期: 2026/3/28

---

## 0. 本次已完成的修复总览

本次工作修复了 libcbench 评测中的 **14 个 0 分项**，以下是三项已提交的改动及其效果：

### 0.1 实现 `memfd_create` + `O_TMPFILE` 拦截

**问题**：glibc 版 `libc-bench` 的 `b_stdio_putcgetc` 和 `b_stdio_putcgetc_unlocked` 在调用 `tmpfile()` 时，glibc 使用 `open("/tmp", O_TMPFILE|O_RDWR)` 创建匿名临时文件。内核不支持 `O_TMPFILE` flag（值 `0x410000`），`sys_openat` 将未知 flag 截断后走 ext4 路径打开 `/tmp` 目录失败（`ext4_fopen: /tmp, rc=2`），glibc 无限重试导致进程卡死。后续的 `b_regex_compile` 和 `b_regex_search` ×2 也因此无法执行。

**修复内容**：

| 文件 | 改动 |
|------|------|
| `os/src/fs/memfd.rs`（新增） | 实现 `MemFdInode`（VfsInode trait）+ `MemFdFile`（File trait），内存匿名文件，支持 `read_at`/`write_at`/`size`/`truncate`/`get_offset`/`set_offset`，使 `sys_lseek`/`sys_pread64`/`sys_fstat` 正常工作 |
| `os/src/fs/mod.rs` | 注册 `mod memfd`，导出 `MemFdFile` |
| `os/src/fs/vfs/mod.rs` | 导出 `VfsNodeKind`（memfd.rs 需要） |
| `os/src/syscall/mod.rs` | 添加 `SYSCALL_MEMFD_CREATE = 279` 常量及 dispatch |
| `os/src/syscall/fs.rs` | 添加 `sys_memfd_create` 函数；在 `sys_openat` 中检测 `O_TMPFILE`（`flags & 0x410000 == 0x410000`），直接返回 memfd 而非走 ext4 |

**为什么这样做**：
- `memfd_create(2)` 是 Linux 3.17 引入的系统调用，返回一个匿名内存文件 fd，支持 seek/read/write，musl 和 glibc 的 `tmpfile()` 都会优先尝试
- glibc 实际上不用 `memfd_create`，而是用 `O_TMPFILE`（Linux 3.11 特性），所以仅实现 `memfd_create` 不够——必须在 `sys_openat` 中拦截 `O_TMPFILE` flag
- `MemFdFile` 实现了 `inode()` 方法返回自身的 `MemFdInode`，这是关键：`sys_lseek` 依赖 `file.inode().is_some()` 判断文件是否可 seek，没有 inode 的文件会返回 `ESPIPE`

**效果**：glibc stdio 从卡死（0 分）→ 1.3s 完成（1.0 分），regex×3 从被阻塞（0 分）→ 正常执行（1.47~1.91 分）。glibc `b_malloc_thread_stress`（RV 上之前 0 分）也恢复。

### 0.2 COW per-page TLB 刷新

**问题**：LA（LoongArch）上 musl `libc-bench` 的 6 个 malloc 测试（`b_malloc_big1/big2/bubble/sparse/tiny1/tiny2`）得 0 分——测试有输出但耗时是 RV 的 5~19 倍，超出评测时间限制或 judge 计算得分为 0。

根因是 COW page fault handler 中每次处理一个页的 fault 后调用全量 TLB 刷新：
```rust
// 修改前
fn flush_tlb() {
    #[cfg(target_arch = "loongarch64")]
    unsafe { core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0") }  // 刷新所有 TLB
}
```

malloc 密集测试做数百次 `mmap/munmap` 循环，每次 COW fault 都全量刷新 TLB，导致 LA 上 TLB miss rate 极高。

**修复内容**：

| 文件 | 改动 |
|------|------|
| `os/src/mm/memory_set.rs` | 新增 `flush_tlb_page(va)` 函数；`handle_cow_fault` 的两处 `flush_tlb()` 改为 `flush_tlb_page(fault_vpn.0 << 12)` |

```rust
// 新增
fn flush_tlb_page(va: usize) {
    #[cfg(target_arch = "riscv64")]
    unsafe { core::arch::asm!("sfence.vma {}, zero", in(reg) va) }  // 只刷该地址
    #[cfg(target_arch = "loongarch64")]
    unsafe { core::arch::asm!("dbar 0; invtlb 0x06, $r0, {va}", va = in(reg) va) }  // 按 VA 刷单条
}
```

**为什么这样做**：
- COW fault 只修改了一个 VPN 的 PTE，没必要刷新整个 TLB
- RV 的 `sfence.vma addr` 和 LA 的 `invtlb 0x06, $r0, va` 都支持按虚拟地址刷新单条 TLB 条目
- `invtlb 0x06` 的语义是"清除所有 ASID 中匹配该 VA 的 TLB 条目"，适合当前无 ASID 管理的场景
- COW fork 路径（`from_existed_user`）保留全量刷新，因为它修改了大量 PTE

**效果**：LA musl malloc×6 从 0 分 → 有输出（score ≥ 1.0）。整体 LA 性能显著提升。

### 0.3 本次提交汇总

分支：`fix/libcbench-score`，commit `3440ee4`

```
fix(libcbench): memfd_create + O_TMPFILE + per-page TLB flush，libcbench 0分项清零
```

本地评测对比：

| 变体 | 之前（远端） | 本次（本地） | 变化 |
|------|------------|------------|------|
| musl-rv | 24.17 | 33.67 | +9.50 |
| glibc-rv | 23.55 | 33.77 | +10.22 |
| musl-la | 18.26 | 36.32 | +18.06 |
| glibc-la | 25.10 | 36.32 | +11.22 |
| **总计** | **91.08** | **~140** | **~+49** |

0 分项：14 个 → **3 个**（仅剩 musl pthread×3，见下方未解决问题）。

---

## 1. 未解决问题：musl pthread SIGSEGV

### 问题现象

musl 版 `libc-bench` 的三个 pthread 测试在 RV 和 LA 上均 SIGSEGV（score=0）：

- `b_pthread_createjoin_serial1`
- `b_pthread_createjoin_serial2`
- `b_pthread_create_serial1`

RV 崩溃日志：
```
[ERROR] trap_handler: page fault addr=0x10 sepc=0x1c74c ra=0x1c6f8 sp=0x6004c8e08 tp=0x6004c8fe8
```

LA 崩溃日志：
```
[ERROR] trap_handler: page fault addr=0x8 pid=10 tid=1 name=libc-bench sepc=0x12000eabc
```

注意：**glibc 版 pthread 测试全部通过**（包括 RV 和 LA），问题仅出现在 musl 静态链接版本。

---

## 2. 调试过程

### 1. 崩溃点反汇编

从 sdcard 提取 musl `libc-bench` 二进制，反汇编 `sepc=0x1c74c`：

```asm
1c6bc:  mv s0, tp              # s0 = tp（线程指针）
1c6c8:  addi s2, s0, -224      # s2 = tp - 224 = &pthread_struct（self 指针）
1c6f8:  ld a4, -200(s0)        # a4 = *(tp-200) = self.prev
1c6fc:  bne a4, s2, 0x1c734    # if prev != self → 需要 unlink（进入链表摘除路径）
...
1c748:  ld a5, -208(s0)        # a5 = *(tp-208) = 某个链表字段
1c74c:  sd a5, 16(a4)          # *(prev + 16) = a5 → CRASH: a4=0 即 *(NULL+16)
```

**结论**：崩溃在 musl `__pthread_exit` 的线程链表 unlink 路径。`*(tp-200)` 即 `self.prev` 值为 0（NULL），导致写入 `*(NULL + 16)` 触发 page fault。

### 2. 确认 TLS 布局

musl RISC-V TLS Variant I 布局：
- `tp` = `pthread_struct + 224`（由 `__clone` 的 tls 参数决定）
- `*(tp - 200)` = `pthread_struct[24]` = `prev` 字段
- `*(tp - 208)` = `pthread_struct[16]` = `next` 字段
- `*(tp - 224)` = `pthread_struct[0]` = `self` 指针

`__pthread_exit` 的检查逻辑：
```c
// 伪代码
if (self->prev == self) {
    // 自引用环 → 只有自己一个线程，跳过 unlink
} else {
    // 需要从链表中摘除自己
    self->prev->next = self->next;  // ← 这里 crash（prev=NULL）
}
```

### 3. 在 sys_clone 中验证 TLS 字段值

在内核 `sys_clone` 中添加诊断代码，读取新线程 TLS 区域的 `prev` 和 `next` 字段：

```rust
// os/src/syscall/process.rs — sys_clone 中 SETTLS 处理
let page_table = PageTable::from_token(token);
let prev_val = page_table.translate_va(VirtAddr::from(tls_addr - 200))
    .map(|pa| unsafe { *(pa.0 as *const usize) });
let next_val = page_table.translate_va(VirtAddr::from(tls_addr - 192))
    .map(|pa| unsafe { *(pa.0 as *const usize) });
warn!("[clone-tls] tls={:#x} prev={:x?} next={:x?}", tls_addr, prev_val, next_val);
```

输出：
```
[clone-tls] pid=4 tid=1 tls=0x600022fe8 *(tp-200)prev=Some(0) *(tp-192)next=Some(0)
[clone-tls] pid=4 tid=2 tls=0x600045fe8 *(tp-200)prev=Some(0) *(tp-192)next=Some(0)
```

**关键发现**：在 `clone()` 系统调用被执行的时刻，新线程 TLS 中的 `prev` 和 `next` 字段**已经是 0**。说明 musl 没有在调用 clone 之前写入这两个字段。

### 4. 反汇编 `pthread_create` 全流程

反汇编 musl 的 `pthread_create` 函数（0x1c93c 开始），完整调用顺序：

```
__copy_tls(new_thread)     # 0x1caac: 初始化 TLS 区域
  → 设置 self=a0, dtv, locale 等
  → 不设置 prev/next（保持 mmap 后的 0 值）

pthread_create 自身:
  → 设置 startlock、sigmask、user_fn、user_arg
  → __tl_lock()            # 0x1cb50: 获取全局线程锁
  → threads_minus_1 += 1   # 0x1cb54-0x1cb60: 增加线程计数
  → __clone(start, stack, flags, arg, ptid, tls, ctid)  # 0x1cb88
  → 后续处理...
```

**关键发现**：musl 在 `__tl_lock()` 和 `__clone()` 之间**只增加了计数器，没有执行链表链入操作**（没有 `new->prev = self; new->next = ...` 之类的指令）。

### 5. 反汇编新线程 start 函数

`pthread_create` 传给 `__clone` 的线程入口函数有两个版本：

**版本 A（0x1c860，带 startlock 同步）**：
```
wait_startlock:
    lr.w a4, (startlock_addr)  # 等待 startlock 变为 0
    if a4 == 1: sc.w a2, 2, (startlock_addr); futex_wait
    if a4 == 2: set_tid_address(); exit()  # cancel 路径
    if startlock == 0:
        sigprocmask(SIG_SETMASK, &new->sigmask)
        user_fn(user_arg)            # 调用用户函数
        __pthread_exit()             # 退出
```

**版本 B（0x1c900，简化版）**：
```
    user_fn(user_arg)
    __pthread_exit()
```

**两个版本都没有链表链入代码。** `__pthread_exit` 直接访问 `prev/next`，假设它们已有效。

### 6. 尝试修复：初始化 prev 为 self

在 `sys_clone` 中设置 `*(tp-200) = tp - 224`（让 prev 指向 self，形成自引用环）：

```rust
let self_ptr = tls_addr.wrapping_sub(224);
let page_table = PageTable::from_token(token);
if let Some(pa) = page_table.translate_va(VirtAddr::from(tls_addr.wrapping_sub(200))) {
    unsafe { *(pa.0 as *mut usize) = self_ptr; }
}
```

**结果**：SIGSEGV 消失，但线程在 `b_pthread_createjoin_serial1` 中**死锁（hang）**。

分析：设置 `prev = self` 后 `__pthread_exit` 的 unlink 路径被跳过，但这破坏了 musl 线程管理的假设——musl 期望 `prev/next` 在线程被链入链表后才是有效值。把 prev 设为 self 后，`__tl_lock` 中的线程计数和锁状态可能与实际链表不一致，导致后续操作死锁。

---

## 3. 根因分析（已解决 ✅）

### 真正的根因：`sys_set_tid_address` 返回 TID=0

经过深入的二进制反汇编分析，发现崩溃的根本原因是**内核的 `sys_set_tid_address` 和 `sys_gettid` 对主线程返回 TID=0**。

#### musl 的 `__tl_lock` 实现

musl 的线程链表锁 `__tl_lock` 使用 CAS 原子操作保护全局链表：

```c
// musl __tl_lock 核心逻辑（从二进制反汇编还原）
void __tl_lock(void) {
    int tid = self->tid;           // 从 TLS 读取线程 TID
    if (count && lock == tid) {    // 重入检测：count>0 且 lock 值等于自己的 TID
        ++count;
        return;                    // 认为自己已持有锁，跳过 CAS
    }
    while (a_cas(&lock, 0, tid))   // CAS: 期望 0（未锁），写入 tid（已锁）
        __wait(&lock, 0, tid, 0);
    count = 1;
}
```

**关键设计假设**：`0` 是"未锁"哨兵值，TID 必须 > 0。Linux 上所有线程的 TID 都是正整数（主线程 TID = PID）。

#### 我们的内核返回 TID=0 的后果

1. **互斥失效**：主线程 `CAS(&lock, 0, 0)` → lock 值仍为 0 → 其他线程看不出锁已被持有 → `CAS(&lock, 0, N)` 成功 → 两个线程同时进入临界区

2. **重入误判**：当 PID 与某个新线程的 internal_tid 碰撞时（例如 pid=4 的主线程 TID=4 和第 4 个创建的线程 internal_tid=4），`__tl_lock` 的重入检测 `count && lock == tid` 误判为重入 → 新线程跳过 CAS → 无锁进入临界区

#### 崩溃链

```
主线程 __tl_lock(tid=0)              新线程 __pthread_exit
   CAS(&lock, 0, 0) → 成功               ↓
   lock = 0 (看起来仍未锁!)          __tl_lock(tid=N)
   ↓                                     CAS(&lock, 0, N) → 成功!(lock=0=未锁)
   clone() → 创建新线程                  ↓
   [尚未执行链入代码]                 ld a4, *(tp-200)  → a4 = next = 0
                                      sd a5, 16(a4)     → *(0 + 16) = prev
                                                         → PAGE FAULT at addr=0x10!
```

### 为什么 glibc 不受影响

glibc 的 `NPTL` 实现不依赖 `__tl_lock` 的 CAS(0, tid) 模式。NPTL 使用不同的内部锁和线程管理机制，不会因为 TID=0 而破坏互斥。

### 为什么 `b_pthread_uselesslock` 不受影响

`b_pthread_uselesslock` 创建线程后只做 `pthread_mutex_lock/unlock`，**不调用 `pthread_join`**。线程退出时通过 `exit_group` 直接终止整个进程，不走 `__pthread_exit` 的 unlink 路径。

### 为什么 `serial1` 比 `serial2` 更容易通过

- `serial1`：每次只有 1 个活跃线程。主线程的 TID 与线程 TID 碰撞概率低（仅当 PID == internal_tid）
- `serial2`：同时创建 50 个线程（TID 1~50），当 PID 落在 1~50 范围内时必然碰撞

---

## 4. 修复方案

### 修复内容

使用 `system_tid = internal_tid + 1` 作为所有线程的系统 TID，确保 TID > 0 且进程内唯一：

| 文件 | 改动 |
|------|------|
| `os/src/syscall/thread.rs` | `sys_gettid` 返回 `internal_tid + 1`；`sys_set_tid_address` 返回 `internal_tid + 1` |
| `os/src/syscall/process.rs` | `sys_clone` 的 `PARENT_SETTID`/`CHILD_SETTID` 写入 `internal_tid + 1`；`sys_clone` 返回值改为 `system_tid`；`sys_tkill` 将系统 TID 映射回内部索引（`-1`） |

### 为什么这样做

- `internal_tid + 1` 保证所有 TID ≥ 1（主线程 0→1，线程 1→2，…），不会与 CAS 哨兵值 0 碰撞
- 同一进程内 internal_tid 唯一 → `+1` 后仍唯一，不会触发 `__tl_lock` 重入误判
- 改动最小（仅 3 个 syscall），不需要修改 PID 分配器或 TaskControlBlock 结构

### 效果

| 测试 | 修复前 | 修复后 |
|------|--------|--------|
| musl `b_pthread_createjoin_serial1` | SIGSEGV (0分) | ✅ 通过 (5.0s) |
| musl `b_pthread_createjoin_serial2` | SIGSEGV (0分) | ✅ 通过 (5.0s) |
| musl `b_pthread_create_serial1` | SIGSEGV (0分) | ✅ 通过 (1.2s) |
| glibc pthread×3 | ✅ 通过 | ✅ 通过（无回归） |
| RV + LA 双架构 | ❌ 3项×2arch=6项失败 | ✅ 全部通过 |

分支：`fix/libcbench-score`，commit `657fcca`

---

## 5. 完整调试过程总结

### 5.1 反汇编分析路径

1. 从 sdcard 提取 musl `libc-bench` 二进制
2. 反汇编 `__pthread_exit`（0x1c6a4）→ 确认 `__tl_lock` 在读取 `next` 之前调用
3. 反汇编 `pthread_create`（0x1c93c）→ 找到父线程链入代码（0x1cc14-0x1cc2c），确认在 clone 返回后执行
4. 反汇编线程 `start` 函数（0x1c860）→ 发现 joinable 线程**没有 startlock 同步**，直接执行用户函数
5. 分析 `__tl_lock`（0x1c570）→ 确认使用 `self->tid` 作为锁值，0 = 未锁哨兵

### 5.2 关键转折点

- 第一轮诊断（addr=0x10, next=0x0）→ 初步认为是 prev/next 未初始化
- 设置 `prev=self` 修复 → 消除 SIGSEGV 但引入死锁 → 排除简单初始化方案
- 添加 TLS 诊断到 page fault handler → 发现 `self->tid` 值与 crash 模式的关联
- 发现 TID=0 → 理解 `__tl_lock` 的 CAS 哨兵设计 → 确认 TID=0 破坏互斥
- PID 碰撞问题（TID=PID=4 vs internal_tid=4）→ 改为 `+1` 方案彻底消除碰撞

---

## 6. 影响评估

- 修复 memfd/O_TMPFILE/TLB + pthread TID 后，libcbench **0 分项从 14 个降为 0 个**
- musl pthread×3 在 RV 和 LA 上恢复，每项约 1.0+ 分 ≈ **+6 分**
- 总计本次修复（含 0.1 memfd + 0.2 TLB + 本次 pthread）提升约 **+55 分**
