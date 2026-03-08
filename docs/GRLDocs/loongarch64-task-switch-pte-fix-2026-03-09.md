# LoongArch64 任务切换与页表格式修复记录

**日期**: 2026/3/9
**涉及分支**: `muti-arch` (worktree: `fix-la-arch`)
**参考实现**: OSKernel2025-rustoswhu (`arch/src/loongarch64/`)

---

## 一、问题现象

内核在执行到 `run_tasks()` 后完全卡死，无任何进一步输出。QEMU 不 panic、不退出，只是无限循环。最后一行日志停在：

```
[kernel] Minimal TCB initialized (no PT_TLS): tp = 0x80410ff0
```

## 二、罪魁祸首

**此次卡死涉及 4 个层层递进的独立 bug，它们叠加后导致了完全的死循环。** 单独修复其中任何一个都不够——必须全部修复才能让内核越过 `run_tasks()` 进入用户态。

| # | Bug | 严重等级 | 根因 |
|---|-----|---------|------|
| 1 | 用户态 trap 从未被处理 | 致命 | `trap_return` 用 `jr`（而非 `bl`）调 `user_restore`，syscall/中断永不处理 |
| 2 | PGDH 未设置 | 致命 | `activate_page_table` 只写 PGDL，内核栈在 `0xFFFF…` 走 PGDH 未映射 |
| 3 | PTE 格式为 RISC-V 格式 | 致命 | `ppn << 10` + RISC-V flags，LoongArch 硬件完全不认 |
| 4 | D 位软件管理缺失 + 向量指令未使能 | 致命 | `tlbfill` 不自动设 D=1；EUEN=0 禁用了编译器生成的 LSX 指令 |

以下按调试发现顺序逐一分析。

---

## 三、Bug 1：用户态 trap 从未被处理

### 3.1 背景知识

rCore-Tutorial (RISC-V) 使用 **trampoline 模型**：`trap_return` 切换页表后跳到 trampoline 页的 `__restore` 汇编；用户 trap 时 `__alltraps` 保存上下文并调用 `trap_handler`。trampoline 页同时映射在内核和用户页表中，因此跨页表跳转是安全的。

LoongArch64 没有使用 trampoline，而是使用 rustoswhu 的 **call-based 模型**：`user_restore` 是一个函数调用，它保存内核 callee-saved 寄存器后执行 `ertn` 进入用户态；用户 trap 时 `user_vec` 恢复这些寄存器并 `ret` 返回到 `user_restore` 的调用者。

### 3.2 问题分析

rcore-lab 的 LoongArch64 移植混用了两种模型：

```rust
// trap_return() -> !
pub fn trap_return() -> ! {
    activate_page_table(user_token);
    asm!("move $a0, {trap_cx}", "jr {restore}", ...); // 用 jr 而非 bl!
}
```

`jr` 不设置 RA 寄存器。当 `user_vec` 执行 `ret` 时，返回到一个未定义的地址。更关键的是，**即使返回成功，也没有任何代码处理 trap**——syscall 和时钟中断被完全忽略。用户程序反复触发 syscall → trap → `user_vec` 返回 → 不处理 → 再次 `user_restore` → 同一条 syscall 指令 → 无限循环。

### 3.3 修复方案

采用 rustoswhu 的 `task_entry` 模型：

```
run_tasks() → context_switch_pt → task_entry() → user_restore → 用户态
                                      ↑                          |
                                      └── 处理 trap ← user_vec ←─┘
```

- `goto_trap_return(kstack_top)` 的 KPC 从 `trap_return` 改为 `task_entry`
- `task_entry()` 循环体：激活用户页表 → `user_restore` → `user_vec` 返回 → `loongarch64_trap_handler` 分发 → 回到循环顶部
- `run_tasks()` 和 `schedule()` 使用 `context_switch_pt`（带页表切换），而非 `context_switch`

### 3.4 关键证据

通过 GDB 验证 `context_switch_pt` 确实将 `ra` 设为 `task_entry` 地址：

```
=== after ret ===
pc  = 0x90000000900209a0    ← task_entry 地址
ra  = 0x90000000900209a0
sp  = 0xfffffffffffff000    ← 内核栈顶（TRAMPOLINE）
```

`task_entry` 被成功调用，但 `sp = 0xfffffffffffff000` 引出了 Bug 2。

---

## 四、Bug 2：PGDH 未设置

### 4.1 背景知识

LoongArch64 的虚拟地址空间由 VA[47] 分为两半：

- VA[47]=0 → 用户空间，页表根在 **PGDL** (CSR 0x19)
- VA[47]=1 → 内核空间，页表根在 **PGDH** (CSR 0x1a)

此外，DMW（Direct Mapping Window）提供硬件直接映射：

- DMW0 (CSR 0x180): `VA[63:60]=0x8` → `PA`（不可缓存）
- DMW1 (CSR 0x181): `VA[63:60]=0x9` → `PA`（可缓存）

内核代码/数据在 `0x9000…` 走 DMW1，不需要页表。但内核栈在 `TRAMPOLINE = 0xFFFFFFFFFFFFF000` 附近（rCore-Tutorial 风格），VA[63:60]=0xF 不匹配任何 DMW 窗口，必须走 PGDH 页表。

### 4.2 问题分析

```rust
// activate_page_table 只写 PGDL!
pub fn activate_page_table(token: usize) {
    pgdl::set_base(token);  // ← 只设了 PGDL
    // PGDH 从未被设置
}
```

`KERNEL_SPACE.activate()` 在 `mm::init()` 中被调用，但只设了 PGDL。PGDH 保持为 0（或 boot 默认值）。当 `task_entry` 访问内核栈 `0xFFFFFFFF…` 时，TLB refill 读 PGDH=0，走到物理地址 0 的垃圾数据，填入无效 TLB entry，触发 Page Invalid Exception (exception 2)，然后递归异常。

### 4.3 修复

```rust
pub fn init_kernel_page_table(token: usize) {
    pgdl::set_base(token);
    pgdh::set_base(token);  // ← 关键：同时设置 PGDH
    asm!("dbar 0; invtlb 0x00, $r0, $r0");
}
```

在 `KERNEL_SPACE.activate()` 中调用 `init_kernel_page_table`。之后每次 `activate_page_table(user_token)` 只改 PGDL（用户页表），PGDH 始终指向内核页表。

### 4.4 验证

添加调试打印确认：

```
[kernel] PGDL=0x92081000 PGDH=0x92081000  ← 两者相同，正确
```

但修复后仍然卡死——QEMU 异常日志揭示了 Bug 3。

---

## 五、Bug 3：PTE 格式完全错误

### 5.1 背景知识

**RISC-V Sv39 PTE 格式**：PPN 从 bit 10 开始，flags 在 bits[9:0]。
**LoongArch64 PTE 格式**：PPN 从 bit 12 开始（= 物理地址），flags 在 bits[11:0]。

两者的 flag 位定义也完全不同：

| 含义 | RISC-V 位置 | LoongArch 位置 |
|------|-----------|--------------|
| Valid | bit 0 | bit 0 |
| Read | bit 1 | NR at bit 11（取反） |
| Write | bit 2 | W at bit 8 |
| Execute | bit 3 | NX at bit 12（取反） |
| User | bit 4 | PLV at bits 3:2 |
| Dirty | bit 7 | D at bit 1 |

### 5.2 问题分析

rcore-lab 直接复制了 RISC-V 的实现：

```rust
pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
    PageTableEntry { bits: ppn.0 << 10 | flags.bits as usize }
    //                         ^^^^ RISC-V 偏移！LoongArch 应为 << 12
}
```

QEMU 日志确认了递归 page fault：

```
BADVA ffffffffffc4e000
exception: 2 (Page invalid exception for store)  ← 无限循环 190 万次
```

### 5.3 修复

完全重写 `PageTableEntry`，将软件抽象的 `PTEFlags`（V/R/W/X/U）翻译为 LoongArch 硬件格式：

```rust
pub fn new(ppn: PhysPageNum, flags: PTEFlags) -> Self {
    // 目录项（仅 V flag）：干净的 ppn << 12，无额外 flags
    // 因为 lddir 指令直接用该值做下一级基地址
    if flags == PTEFlags::V {
        return PageTableEntry { bits: ppn.0 << 12 };
    }
    // 叶子 PTE：ppn << 12 | V | P | MAT | D(if W) | W(if W) | PLV(if U)
    let mut hw: usize = ppn.0 << 12;
    hw |= la_pte::V | la_pte::P | la_pte::MAT;
    if flags.contains(PTEFlags::W) { hw |= la_pte::W | la_pte::D; }
    if flags.contains(PTEFlags::U) { hw |= la_pte::PLV; }
    PageTableEntry { bits: hw }
}
```

**特别注意**：NR (bit 11) 和 NX (bit 12) 与 PPN 的低位重叠（PPN 也从 bit 12 开始）。rustoswhu 的解决方案是**注释掉 NR/NX**，所有页面默认可读可执行。我们采用相同策略。

另外，**目录项必须是干净的 `ppn << 12`**——`lddir` 指令直接用目录项的值作为下一级页表的物理基地址，如果低 12 位有 flag 残留（如 V=1, P=1, MAT=0x10），会导致基地址偏移，读到错误的页表数据。修复前的 `assert!(!pte.is_valid())` panic 就是这个原因。

---

## 六、Bug 4：D 位软件管理 + 向量指令禁用

### 6.1 D 位由软件管理

这是 LoongArch 与 RISC-V 最大的差异之一。RISC-V 的 A/D 位由硬件自动设置；LoongArch 的 D 位完全由软件管理。`tlbfill` 指令**不会**从 PTE 复制 D 位到 TLB entry——首次 store 必定触发 PME (Page Modified Exception, exception 4)。

修复 Bug 3 后的 QEMU 异常日志确认了这一点：

```
367845 exception: 4   ← PME 占了所有异常
BADVA fffffffffffff000   ← 内核栈地址
```

### 6.2 修复方案

在 `trap_vector_base` 的内核 trap 路径中添加 PME 快速处理：

```asm
// 检查 ecode 是否为 4 (PME)
csrrd   $sp, 0x5        // ESTAT
srli.d  $sp, $sp, 16
andi    $sp, $sp, 0x3f  // ecode
ori     $t0, $zero, 4
bne     $sp, $t0, 1f    // 不是 PME → 走正常路径

// PME 快速路径：tlbsrch → tlbrd → 设 D 位 → tlbwr
tlbsrch
tlbrd
ori     $t0, $zero, 0x2   // D bit mask
csrrd   $sp, 0x0c          // TLBELO0
or      $sp, $sp, $t0
csrwr   $sp, 0x0c
csrrd   $sp, 0x0d          // TLBELO1
or      $sp, $sp, $t0
csrwr   $sp, 0x0d
tlbwr
csrrd   $sp, KSAVE_USP
ertn
```

关键点：不能用 `tlbfill`（它不设 D），必须用 `tlbsrch` + `tlbrd` + 修改 TLBELO + `tlbwr` 显式覆写 TLB 条目。

### 6.3 向量指令禁用

修复 PME 后，QEMU 异常变为：

```
726776 exception: 16 (128 bit vector instructions Disable exception)
```

编译器在 `-O2` 下会生成 LSX（128 位 SIMD）指令（例如 `SAVE_REGS` 宏中的寄存器保存），但 EUEN (CSR 0x2) = 0 禁用了这些指令。

修复：`entry.S` 中设 `EUEN = 0x07`（使能 FPE + SXE + ASXE）。

---

## 七、修改文件汇总

| 文件 | 修改内容 |
|------|---------|
| `os/src/arch/loongarch64/trap.rs` | 新增 `task_entry()` 循环；PME 快速路径汇编 |
| `os/src/arch/loongarch64/kcontext.rs` | `goto_trap_return` KPC 改为 `task_entry` |
| `os/src/arch/loongarch64/page_table.rs` | PTE 格式重写；新增 `init_kernel_page_table` |
| `os/src/arch/loongarch64/entry.S` | EUEN = 0x07 使能向量指令 |
| `os/src/arch/loongarch64/mod.rs` | 导出 `task_entry` |
| `os/src/arch/mod.rs` | 导出 `task_entry` |
| `os/src/mm/memory_set.rs` | `activate()` 调 `init_kernel_page_table` |
| `os/src/task/processor.rs` | LA64 使用 `context_switch_pt` |
| `os/src/task/mod.rs` | `__switch` 仅 riscv64 导入 |
| `os/src/task/switch.rs` | `__switch` 仅 riscv64 导入 |

---

## 八、当前状态与后续

修复上述 4 个 bug 后，内核成功执行了完整的任务切换流程并进入了用户态（`ertn`）。QEMU 异常日志确认 PME 降至仅 2 次（正常的首次 dirty 标记），向量异常归零。

下一个待解决的问题是用户态首次执行后跳到了错误地址 `0x900000008d038f0a`（Exception 13: Instruction Non-Existent），这属于 trap context 初始化或 `user_restore` 传参的独立问题，需要继续调试。
