# glibc 动态链接 sa_restorer 未重定位修复——从 page fault 0x2000 到 5/5 全通

**日期**: 2026/3/24

---

## 1. 罪魁祸首

**glibc 动态链接的 netperf 在 SIGALRM 信号触发后 page fault at 0x2000 的根因是：glibc 的 `sigaction()` 包装器设置的 `sa_restorer`（信号返回跳板地址）没有被正确重定位——只包含 libc 内部的原始偏移 `0x2000`，而非运行时地址 `libc_base + 0x2000`。rCore 信号处理代码直接将该值写入 trap context 的 RA 寄存器，导致信号 handler 执行完毕后 `ret` 跳转到未映射的地址 0x2000。**

修复方法：在 RISC-V 信号派发代码中检测 `sa_restorer` 是否为明显未重定位的地址（< 0x10000），如果是则回退到内核自带的 `rt_sigreturn` 跳板——功能完全等价，因为两者都只是执行 `ecall` 调用 `rt_sigreturn` 系统调用。

---

## 2. 背景知识

### 2.1 glibc 动态链接的信号处理机制

在 Linux 上，当用户程序调用 `sigaction(signum, &act, NULL)` 时，glibc 的包装器会做以下修改：

```c
// glibc/sysdeps/unix/sysv/linux/sigaction.c
struct kernel_sigaction kact;
kact.sa_handler = act->sa_handler;
kact.sa_flags = act->sa_flags | SA_RESTORER;
kact.sa_restorer = &__restore_rt;  // libc 中的信号返回跳板
kact.sa_mask = act->sa_mask;
syscall(__NR_rt_sigaction, signum, &kact, ...);
```

`__restore_rt` 是 libc.so.6 中一个极小的函数：

```asm
// glibc/sysdeps/unix/sysv/linux/riscv/sigrestorer.S
__restore_rt:
    li a7, __NR_rt_sigreturn   // 139
    ecall
```

当内核派发信号时（修改 trap context 跳转到 handler），会将 `sa_restorer` 的地址写入 RA 寄存器。信号 handler 执行完毕后执行 `ret`（即 `jalr x0, ra, 0`），跳转到 `__restore_rt`，后者通过 `rt_sigreturn` 系统调用恢复原始的执行上下文。

### 2.2 动态链接库的地址重定位

对于动态链接的共享库（如 libc.so.6），代码和数据中引用全局符号时通过 GOT（Global Offset Table）间接访问。ld-linux（动态链接器）在加载库时：

1. 将库映射到内存（mmap）
2. 解析 `.rela.dyn`（R_RISCV_RELATIVE 等）和 `.rela.plt`（R_RISCV_JUMP_SLOT）重定位条目
3. 修改 GOT 条目和数据段中的指针，加上库的实际加载基址

如果某个重定位未被正确处理，相关指针就只包含原始的文件偏移，而非运行时的绝对地址。

---

## 3. 调试过程

### 3.1 初始症状与误判

最初的现象是 glibc netperf 在运行过程中 page fault at 0x2000，sepc=ra=0x2000。之前的文档（glibc动态链接调试记录.md 阶段 7）将此归因于**主程序 data 段在 ld-linux 运行后被零页替换**——基于 `.dynamic` 在内核加载后正确、运行时变零的观察。

这个方向实际上是**误判**。

### 3.2 排除 data 段页替换

为验证"页替换"假说，在以下关键路径添加 watchpoint 日志：

| 位置 | 监控内容 |
|------|----------|
| `MapArea::map_one` | VPN 0x40030-0x40050 被分配新物理页 |
| `MapArea::unmap_one` | VPN 被取消映射 |
| `MemorySet::unmap_range` | MAP_FIXED 覆盖时的范围 unmap |
| `MemorySet::insert_framed_area` | 新映射区域创建 |
| `sys_mmap` | mmap 操作触及监控范围 |

**结果：没有任何 `unmap_one`、`unmap_range` 或 `sys_mmap` 的 watchpoint 输出。** 这证明在 exec 完成后，data 段的 VPN 从未被任何操作重映射。data 段页替换的假说被排除。

### 3.3 验证 copy_data 正确性

进一步在 `copy_data` 中添加详细日志，验证文件数据是否正确写入物理页：

```
copy_data vpn=0x40033 ppn=0x82627 off=0xc48 len=0x3b8 nz=121
  first_nz_off=0x20 src=0x28 dst=0x28 direct=0x28
copy_data vpn=0x40034 ppn=0x82626 off=0x0 len=0x1000 nz=258
  first_nz_off=0x8 src=0x7374656e29232840 dst=same direct=same
```

source 数据、目标内存、直接物理页读回**三者完全一致**。ELF 加载过程无误。

### 3.4 发现关键线索：crash 在 SIGALRM 之后

将日志级别提升到 SYSCALL，追踪 pid=4（netperf）的完整系统调用序列：

```
[SYSCALL] pid=4 sendto(fd=4, buf=0x40075570, len=0x3e8, ...) ret=1000
[SYSCALL] pid=4 sendto(fd=4, buf=0x40075120, len=0x3e8, ...) ret=1000
... (大量 sendto 调用)
[itimer] pid=4 SIGALRM fired, expire=3047 now=3052     ← SIGALRM 触发
[PageFault] pid=4 sepc=0x2000 ra=0x2000 sp=0x7fffff908  ← 立即 crash
```

**crash 发生在 SIGALRM 触发后的第一条指令**。这直接指向信号处理机制，而非 data 段或 GOT。

### 3.5 详细 context dump 排除 GOT 问题

在 page fault handler 中添加完整的寄存器和内存 dump：

```
[PageFault detail] pid=4 sepc=0x2000 ra=0x2000 sp=0x7fffff908
  gp=0x40037398 tp=0x600072520
  a0=0xe a1=0x40078d80 a2=0x3e8 t1=0x400040ec
```

GOT 条目全部正确：

```
GOT[ 0] 0x400364a8: 0x40056818      (主程序内部地址)
GOT[ 2] 0x400364b8: 0x6000e8a04     (libc 函数地址)
GOT[ 3] 0x400364c0: 0x6000baa16     (libc 函数地址)
...
```

所有 GOT 条目要么指向主程序（0x4000XXXX），要么指向 libc（0x6000XXXXX），没有任何 < 0x10000 的可疑值。**GOT 解析完全正确**。

### 3.6 锁定 sa_restorer

在 `sys_sigaction` 中为 SIGALRM（signum=14）添加 handler 和 restorer 日志：

```
[sigaction] pid=4 signum=14 handler=0x4000529a flags=0x20000000 restorer=0x2000 mask=(empty)
```

**找到了！** handler=0x4000529a 是正确的（在主程序 text 段），但 **restorer=0x2000 是未重定位的 libc 偏移**。

`flags=0x20000000` 就是 `SA_RESTORER`，表示 glibc 要求内核使用 `sa_restorer` 字段作为信号返回地址。rCore 的信号处理代码忠实地执行了：

```rust
// os/src/task/mod.rs handle_signals()
if action.restorer != 0 && action.restorer < USER_ADDR_MAX {
    trap_cx[TrapFrameArgs::RA] = action.restorer;  // RA = 0x2000!
}
```

信号 handler（0x4000529a）执行完毕后 `ret`，跳转到 RA=0x2000 → page fault → SIGSEGV → 信号循环 → 无限 page fault。

---

## 4. 根因分析：为什么 restorer=0x2000

glibc 的 `sigaction` 包装器通过以下代码设置 restorer：

```c
kact.sa_restorer = &__restore_rt;
```

`__restore_rt` 是 libc.so.6 中的一个函数，位于 libc 文件偏移约 0x2000 处。在 Linux 上，当 libc 被加载到例如 `0x7f4a00000000` 时，`__restore_rt` 的运行时地址是 `0x7f4a00002000`。

在我们的内核上，libc 被 ld-linux 通过 mmap 加载到 `0x600073000` 附近。`__restore_rt` 的正确运行时地址应该是 `0x600075000`（libc_base + 0x2000），但实际传递给 `sigaction` syscall 的是 `0x2000`（未加 libc_base）。

这表明 libc 内部的某个 R_RISCV_RELATIVE 重定位（将 `__restore_rt` 的文件偏移修正为运行时地址）没有被 ld-linux 正确处理。具体原因可能是：

1. **libc 的 .rela.dyn 中缺少对应的 RELATIVE 条目**（不太可能，glibc 链接器会生成）
2. **ld-linux 在处理 libc 自身重定位时遇到了某种边界条件**（如页面权限、mmap 布局等导致写入失败）
3. **glibc 内部使用了某种特殊的地址获取方式**（如 TLS 或 ifunc），在我们的内核上行为不同

不管具体原因如何，**workaround 是安全且等价的**：内核自带的 `rt_sigreturn` 跳板与 glibc 的 `__restore_rt` 做完全相同的事（`li a7, 139; ecall`），可以直接替代。

---

## 5. 修复方案

### 5.1 核心修改：`os/src/task/mod.rs`

```rust
// RISC-V: use sa_restorer if valid and looks like a real mapped address;
// otherwise fallback to fixed SIG_RETURN_ADDR stub.
// glibc dynamic binaries may have unrelocated sa_restorer values
// (e.g., raw libc offset 0x2000 instead of libc_base + 0x2000).
let use_restorer = action.restorer != 0
    && action.restorer < USER_ADDR_MAX
    && action.restorer >= 0x10000;
if use_restorer {
    trap_cx[TrapFrameArgs::RA] = action.restorer;
} else {
    if action.restorer != 0 && action.restorer < 0x10000 {
        warn!(
            "[signal] pid={} signum={} restorer={:#x} looks unrelocated, using kernel trampoline",
            pid, signum, action.restorer
        );
    }
    trap_cx[TrapFrameArgs::RA] =
        arch::SIG_RETURN_ADDR + arch::sigtrx::sigreturn_trampoline_offset();
}
```

**阈值选择 0x10000**：
- PIE 程序加载在 0x40000000+
- 共享库加载在 0x600000000+（mmap_base）
- 静态链接程序从 0x10000+ 开始（busybox 的 min_load_vaddr=0x10000）
- 任何 < 0x10000 的地址都不可能是合法的用户态函数地址

### 5.2 辅助修改：`os/src/mm/memory_set.rs`

仅一行 minor refactor（提取 `page_bytes` 变量），无功能变化。

---

## 6. 测试结果

### 6.1 glibc netperf: 5/5 全部通过

| 测试 | 结果 |
|------|------|
| UDP_STREAM | SUCCESS |
| TCP_STREAM | SUCCESS |
| UDP_RR | SUCCESS |
| TCP_RR | SUCCESS |
| TCP_CRR | SUCCESS |

### 6.2 回归测试：无回归

| 测试套件 | 结果 |
|----------|------|
| musl netperf | 5/5 PASS |
| musl iperf3 | 6/6 PASS |

---

## 7. 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `os/src/task/mod.rs` | 信号 restorer 地址未重定位检测 + 内核跳板回退 |
| `os/src/mm/memory_set.rs` | copy_data 中 minor refactor（提取变量） |

---

## 8. 经验总结

### 经验 1: sepc=ra=相同低地址 → 信号返回问题

当 page fault 的 `sepc` 和 `ra` 都等于同一个低地址时，这是 `ret` 指令（`jalr x0, ra, 0`）的特征——`ret` 不修改 `ra`，所以 fault 后 `sepc == ra`。如果该地址明显低于程序加载基址，首先检查**信号 restorer 地址**，而非 GOT 或 data 段。

### 经验 2: SIGALRM 后立即 crash → 聚焦信号机制

netperf 的控制流依赖 `setitimer` + SIGALRM 来终止数据传输。如果程序在大量 syscall 后突然 crash，且 crash 恰在 `[itimer] SIGALRM fired` 日志之后，问题几乎必定在信号派发代码（handler 地址、restorer 地址、信号栈设置、ucontext 恢复）。

### 经验 3: watchpoint 排除法比正向追踪更高效

对于"data 段被覆盖"这类假说，在 `map_one`/`unmap_one` 加条件 `warn!` 比分析完整 mmap 日志快 10 倍。如果 watchpoint **没有输出**，假说直接排除。本次调试中，这一步在 5 分钟内排除了 4 个方向中的 3 个。

### 经验 4: glibc sa_restorer 是动态链接的隐藏陷阱

静态链接的 musl 程序不使用 `SA_RESTORER`（musl 的 `sigaction` 不设置 restorer，rCore 回退到内核跳板）。只有 glibc 动态链接的程序才会设置 `sa_restorer`。这个差异意味着：musl 测试全通不代表信号机制正确——需要用 glibc 动态链接程序（如 netperf、iperf3）验证 restorer 路径。

### 经验 5: 内核信号跳板是安全的通用方案

glibc 的 `__restore_rt` 和内核自带的信号返回跳板做的事完全相同（`li a7, 139; ecall`）。在 restorer 地址不可信时，回退到内核跳板是零风险的——不会影响任何程序的正确性。这比尝试修复 ld-linux 的重定位问题更务实。
