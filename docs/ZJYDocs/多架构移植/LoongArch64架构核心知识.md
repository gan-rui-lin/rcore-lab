# LoongArch64 架构核心知识——从 RISC-V 视角理解龙芯

**日期**: 2026/3/9

**背景**: 本文档面向已有 rCore-Tutorial (RISC-V) 经验的 OS 开发者，系统梳理 LoongArch64（龙芯架构）在操作系统开发中最需要掌握的核心知识。文档结合 `OSKernel2025-rustoswhu/arch/src/loongarch64/` 参考实现和 `rcore-lab/os/src/arch/loongarch64/` 移植代码进行讲解。

---

## 目录

1. [架构概览](#1-架构概览)
2. [寄存器体系](#2-寄存器体系)
3. [特权级与运行模式](#3-特权级与运行模式)
4. [内存管理：DMW 与页表](#4-内存管理dmw-与页表)
5. [TLB 管理——与 RISC-V 的最大差异](#5-tlb-管理与-risc-v-的最大差异)
6. [异常与中断处理](#6-异常与中断处理)
7. [上下文切换](#7-上下文切换)
8. [指令集速查](#8-指令集速查)
9. [启动流程](#9-启动流程)
10. [与 RISC-V 对比速查表](#10-与-risc-v-对比速查表)
11. [移植要点与陷阱](#11-移植要点与陷阱)

---

## 1. 架构概览

LoongArch（龙芯架构）是龙芯中科于 2021 年推出的自主指令集架构，分为 32 位（LA32）和 64 位（LA64）两个版本。它吸收了 MIPS、ARM、RISC-V 等架构的设计经验，但**不是任何已有架构的扩展**。

与 RISC-V 的相似点：
- 都是 RISC 风格的 load-store 架构
- 都有 32 个通用寄存器
- 都采用分页虚拟内存
- 都有多级特权系统

与 RISC-V 的关键不同：
- **硬件 TLB 重填**：LoongArch 有专用的 TLB 重填异常和硬件指令（`lddir`/`ldpte`/`tlbfill`），而 RISC-V 的 Sv39 是纯硬件页表遍历（硬件自动 page walk）
- **直接映射窗口（DMW）**：内核地址空间通过 DMW 寄存器直接映射到物理内存，不需要建立内核页表
- **CSR 编号体系完全不同**：需要重新记忆
- **中断模型不同**：LoongArch 使用线级中断（Line-Based Interrupt），而非 RISC-V 的 PLIC/CLINT

---

## 2. 寄存器体系

### 2.1 通用寄存器（GPR）

LoongArch64 拥有 32 个 64 位通用寄存器 `$r0`–`$r31`：

| 寄存器 | 别名 | 用途 | 对应 RISC-V |
|--------|------|------|------------|
| `$r0` | `$zero` | 硬件零寄存器 | `x0/zero` |
| `$r1` | `$ra` | 返回地址 | `x1/ra` |
| `$r2` | `$tp` | 线程指针（TLS） | `x4/tp` |
| `$r3` | `$sp` | 栈指针 | `x2/sp` |
| `$r4`–`$r9` | `$a0`–`$a5` | 函数参数 / 系统调用参数 | `x10-x15/a0-a5` |
| `$r10`–`$r11` | `$a6`–`$a7` | 函数参数 | `x16-x17/a6-a7` |
| `$r12`–`$r20` | `$t0`–`$t8` | 临时寄存器（caller-saved） | `x5-x7,x28-x31/t0-t6` |
| `$r21` | `$r21` | 保留寄存器 | — |
| `$r22` | `$fp`/`$s9` | 帧指针 / 被调用者保存 | `x8/s0/fp` |
| `$r23`–`$r31` | `$s0`–`$s8` | 被调用者保存（callee-saved） | `x9,x18-x25/s1-s9` |

**关键差异**：
- RISC-V 的系统调用号在 `a7`（`x17`），LoongArch 的系统调用号也在 `$a7`（`$r11`），但在我们的内核实现中通过 `TrapFrameArgs::SYSCALL` 映射到 `x[11]`
- LoongArch 的 `$tp`（`$r2`）位于第 2 号寄存器，RISC-V 的 `tp` 在第 4 号
- LoongArch 的 `$sp`（`$r3`）位于第 3 号寄存器，RISC-V 的 `sp` 在第 2 号

### 2.2 浮点寄存器（FPR）

32 个 64 位浮点寄存器 `$f0`–`$f31`，通过 EUEN 寄存器的 FPE 位启用。

在 OS 中的关键用途：非对齐访问模拟时需要读写 FPR（通过 `movgr2fr.d` 和 `movfr2gr.d` 指令）。

### 2.3 CSR（控制状态寄存器）

这是 OS 开发中最核心的部分。LoongArch 的 CSR 编号与 RISC-V **完全不同**：

| CSR 编号 | 名称 | 用途 | RISC-V 对应 |
|---------|------|------|-------------|
| `0x00` | **CRMD** | 当前运行模式（PLV/IE/PG） | `sstatus` 的部分功能 |
| `0x01` | **PRMD** | 前一运行模式（保存陷入前的 PLV/PIE） | `sstatus`（SPP/SPIE） |
| `0x02` | **EUEN** | 扩展功能使能（FPU/SIMD） | — |
| `0x04` | **ECFG** | 异常配置（中断使能位） | `sie` |
| `0x05` | **ESTAT** | 异常状态（异常原因码） | `scause` |
| `0x06` | **ERA** | 异常返回地址 | `sepc` |
| `0x07` | **BADV** | 出错虚拟地址 | `stval` |
| `0x0C` | **EENTRY** | 异常入口地址 | `stvec` |
| `0x19` | **PGDL** | 页表基址（VA[47]=0） | `satp` 的 PPN 部分 |
| `0x1A` | **PGDH** | 页表基址（VA[47]=1） | — |
| `0x1B` | **PGD** | 当前页表基址（自动选择） | — |
| `0x20` | **CPUID** | CPU 编号 | `mhartid`（M 态） |
| `0x30` | **SAVE0** | 暂存寄存器 0（内核 SP 暂存） | `sscratch` |
| `0x31` | **SAVE1** | 暂存寄存器 1（上下文地址暂存） | — |
| `0x32` | **SAVE2** | 暂存寄存器 2（用户 SP 暂存） | — |
| `0x44` | **TICLR** | 定时器中断清除 | — |
| `0x88` | **TLBRENTRY** | TLB 重填异常入口 | — |
| `0x8B` | **TLBRSAVE** | TLB 重填暂存 | — |
| `0x180` | **DMW0** | 直接映射窗口 0 | — |
| `0x181` | **DMW1** | 直接映射窗口 1 | — |

**重要差异**：
- RISC-V 只有一个 `sscratch` 暂存寄存器，LoongArch 有 **多个 SAVE 寄存器**（`SAVE0`–`SAVE3`），这使得陷入处理更方便——不需要像 RISC-V 那样用 `csrrw` 交换 `sp` 和 `sscratch`
- LoongArch 没有 `satp` 寄存器，取而代之的是 `PGDL`/`PGDH` 分别管理低半/高半虚拟地址空间的页表基址
- LoongArch 的 `EENTRY` 要求 **4096 字节对齐**（`.balign 4096`），而 RISC-V 的 `stvec` 仅需 4 字节对齐

---

## 3. 特权级与运行模式

### 3.1 特权级（PLV）

LoongArch 定义了 4 个特权级：

| PLV | 含义 | RISC-V 对应 |
|-----|------|-------------|
| PLV0 | 内核态（最高特权） | S-mode |
| PLV1 | 保留（通常不用） | — |
| PLV2 | 保留（通常不用） | — |
| PLV3 | 用户态（最低特权） | U-mode |

实际 OS 开发中只使用 PLV0（内核）和 PLV3（用户），与 RISC-V 的 S/U 模式对应。

### 3.2 CRMD 寄存器（当前运行模式）

```
CRMD:
  [1:0] PLV  — 当前特权级
  [2]   IE   — 全局中断使能
  [3]   DA   — 直接地址翻译模式
  [4]   PG   — 映射地址翻译模式（分页）
  [5]   DATF — 取指地址翻译模式
  [6]   DATM — 访存地址翻译模式
```

启动时我们设置 `CRMD = 0xB0`，即 `PG=1, DATF=1, DATM=1, PLV=0`，开启分页模式。

### 3.3 PRMD 寄存器（前一运行模式）

```
PRMD:
  [1:0] PPLV — 陷入前的特权级
  [2]   PIE  — 陷入前的中断使能状态
  [3]   PWE  — 陷入前的监视点使能
```

当异常发生时，硬件自动将 CRMD 的 PLV/IE 保存到 PRMD 的 PPLV/PIE 中，并将 PLV 设为 0、IE 设为 0。`ertn` 指令会将 PRMD 恢复到 CRMD，完成特权级切换。

**对应 RISC-V**：这相当于 RISC-V `sstatus` 中的 `SPP`（保存之前的特权级）和 `SPIE`（保存之前的中断使能状态）。

用户进程初始化时，`PRMD = 0b0111`（PPLV=3/用户态, PIE=1/中断使能, PWE=1）：
```rust
// context.rs
pub fn new() -> Self {
    Self {
        prmd: 0b0111,  // PLV=3, PIE=1, PWE=1
        ..Default::default()
    }
}
```

---

## 4. 内存管理：DMW 与页表

### 4.1 直接映射窗口（DMW）——LoongArch 独有

这是与 RISC-V **最大的架构差异之一**。LoongArch 通过 DMW（Direct Mapped Window）寄存器提供内核地址到物理地址的**直接映射**，无需建立内核页表。

```
DMW0 (CSR 0x180): UC 模式, PLV0
  虚拟地址 0x8000_xxxx_xxxx_xxxx → 物理地址 0x0000_xxxx_xxxx_xxxx
  (Uncached，用于 MMIO 设备访问)

DMW1 (CSR 0x181): CA 模式, PLV0
  虚拟地址 0x9000_xxxx_xxxx_xxxx → 物理地址 0x0000_xxxx_xxxx_xxxx
  (Cached，用于正常内核内存访问)
```

**这意味着**：
- 内核代码和数据通过 `VIRT_ADDR_START = 0x9000_0000_0000_0000` 偏移来访问物理内存
- 物理地址 → 内核虚拟地址：`va = pa | 0x9000_0000_0000_0000`
- 内核虚拟地址 → 物理地址：`pa = va & !0x9000_0000_0000_0000`
- **不需要像 RISC-V 那样在内核页表中映射内核地址空间**

在代码中可以看到这种转换：
```rust
// PhysAddr → 内核可访问的虚拟地址
pub fn get_ref<T>(&self) -> &'static T {
    unsafe { ((self.0 | VIRT_ADDR_START) as *const T).as_ref().unwrap() }
}
```

**对比 RISC-V**：RISC-V 的 Sv39 方案中，内核通常通过修改 `satp` 指向一个包含内核映射的页表来访问物理内存。LoongArch 则绕过了页表，直接通过 DMW 完成内核地址翻译，效率更高且实现更简单。

### 4.2 用户态页表

LoongArch 的用户态地址空间仍然使用标准的多级页表进行翻译。

**页表结构**（Sv39 等效方案）：
- **三级页表**，每级 512 个条目（9 位索引）
- **4KB 页面**（12 位页内偏移）
- **39 位虚拟地址空间**（512GB）

```
39-bit VA: [VPN[2]: 9bit] [VPN[1]: 9bit] [VPN[0]: 9bit] [offset: 12bit]
```

这和 RISC-V Sv39 的结构**几乎一致**！索引提取方式：
```rust
pub fn indexes(&self) -> [usize; 3] {
    let mut vpn = self.0;
    let mut idx = [0usize; 3];
    for i in (0..3).rev() {
        idx[i] = vpn & 511;
        vpn >>= 9;
    }
    idx
}
```

### 4.3 PTE 格式——重大差异

虽然页表结构相似，但 **PTE（页表项）的位定义完全不同**：

#### RISC-V Sv39 PTE 格式：
```
[63:54] Reserved
[53:10] PPN (44 bits)
[9:8]   RSW
[7]     D (Dirty)
[6]     A (Accessed)
[5]     G (Global)
[4]     U (User)
[3]     X (Executable)
[2]     W (Writable)
[1]     R (Readable)
[0]     V (Valid)
```
PPN 提取：`pte >> 10`

#### LoongArch PTE 格式：
```
[63]    RPLV (限制特权级)
[58]    cow  (Copy-on-Write，软件定义)
[12]    NX   (不可执行)
[11]    NR   (不可读)
[10]    G    (全局)
[8]     W    (可写)
[7]     P    (存在)
[6]     GH   (巨页/全局)
[5:4]   MAT  (存储访问类型: 00=强序, 01=非缓存, 10/11=缓存)
[3:2]   PLV  (特权级: 11=用户态可访问)
[1]     D    (脏页)
[0]     V    (有效)
[47:12] PPN  (物理页号，直接存储物理地址！)
```
物理地址提取：`pte & 0xFFFF_FFFF_F000`（**不需要移位！直接掩码**）

**关键差异总结**：

| 对比项 | RISC-V Sv39 | LoongArch |
|--------|-------------|-----------|
| PPN 存储位置 | `[53:10]`，提取需右移 10 位 | `[47:12]`，直接掩码 |
| 权限表达 | 正向（R/W/X = 可读/可写/可执行） | **反向**（NR/NX = 不可读/不可执行），W 仍为正向 |
| 用户态权限 | U 位 = 1 | PLV 字段 = 0b11 |
| 内存类型 | 无（由 PMA 决定） | MAT 字段（4 种类型） |
| 脏页标记 | D 位（bit 7） | D 位（bit 1） |

**在 rcore-lab 的移植中**，为了简化，page_table.rs 仍然使用 RISC-V 风格的 PTEFlags（V/R/W/X/U），PPN 也用右移 10 位的方式存储。**这是一个需要注意的适配层**——当前的 page_table.rs 保持了 RISC-V 的语义，真正写入硬件前需要转换为 LoongArch 的 PTE 格式。参考项目 `OSKernel2025-rustoswhu` 则直接使用了 LoongArch 原生的 PTE 格式。

### 4.4 页表基址寄存器

| LoongArch | 含义 | RISC-V 对应 |
|-----------|------|-------------|
| PGDL (`0x19`) | VA[47]=0 时的页表基址 | `satp.PPN`（统一） |
| PGDH (`0x1A`) | VA[47]=1 时的页表基址 | — |
| PGD (`0x1B`) | 自动根据 VA[47] 选择 PGDL/PGDH | — |

切换页表的方式：
```rust
pub fn activate_page_table(token: usize) {
    pgdl::set_base(token);
    unsafe {
        core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0");
    }
}
```

注意 `token` 是**页表根的物理地址**（不像 RISC-V 需要构造含 MODE 位的 `satp` 值）。切换后必须执行 `dbar 0`（内存屏障）+ `invtlb`（刷新 TLB）。

---

## 5. TLB 管理——与 RISC-V 的最大差异

**这是从 RISC-V 移植到 LoongArch 时最需要理解的核心差异。**

### 5.1 RISC-V 的 TLB 管理

RISC-V Sv39 中，TLB 对软件几乎**完全透明**：
- 硬件自动进行 page walk（遍历三级页表）
- TLB miss 时硬件自动加载
- 软件只需在修改页表后执行 `sfence.vma` 刷新 TLB
- **没有 TLB miss 异常**，一切由硬件完成

### 5.2 LoongArch 的 TLB 管理

LoongArch 的 TLB 采用**软件辅助重填**模型：
- TLB miss 时产生 **TLB 重填异常**（专用异常，有独立入口 `TLBRENTRY`）
- 软件（OS）需要编写 TLB 重填处理程序
- 但 LoongArch 提供了**硬件加速指令**来简化这个过程

### 5.3 TLB 重填处理程序

这是一段**极其关键**的汇编代码，必须正确实现：

```asm
.balign 4096
tlb_fill:
    csrwr   $t0, LA_CSR_TLBRSAVE     // 保存 $t0 到暂存 CSR
    csrrd   $t0, LA_CSR_PGD          // 读取当前页表基址（硬件自动选择 PGDL/PGDH）
    lddir   $t0, $t0, 3              // 遍历第 3 级目录（最高级）
    lddir   $t0, $t0, 1              // 遍历第 1 级目录
    ldpte   $t0, 0                   // 加载偶数页 PTE
    ldpte   $t0, 1                   // 加载奇数页 PTE
    tlbfill                           // 将 PTE 填入 TLB
    csrrd   $t0, LA_CSR_TLBRSAVE     // 恢复 $t0
    ertn                              // 返回
```

**硬件加速指令解读**：

| 指令 | 功能 | 解释 |
|------|------|------|
| `lddir $t0, $t0, level` | 加载页目录项 | 根据出错地址自动索引第 level 级页目录 |
| `ldpte $t0, n` | 加载页表项 | 加载第 n 个 PTE（0=偶数页, 1=奇数页） |
| `tlbfill` | 填充 TLB | 将刚加载的 PTE 对写入 TLB |

LoongArch 的 TLB 条目是**成对的**（dual PTE），每个 TLB 条目同时映射相邻的两个虚拟页（偶数页和奇数页）。这就是为什么需要 `ldpte $t0, 0` 和 `ldpte $t0, 1` 两条指令。

### 5.4 TLB 初始化

在使用 TLB 之前，必须正确配置页表遍历控制寄存器：

```rust
pub fn tlb_init(tlbrentry: usize) {
    tlbidx::set_ps(PS_4K);       // TLB 页面大小 = 4KB
    stlbps::set_ps(PS_4K);       // STLB 页面大小 = 4KB
    tlbrehi::set_ps(PS_4K);      // TLB 重填页面大小 = 4KB

    // 页表遍历控制——低位
    pwcl::set_pte_width(8);                          // PTE 宽度 = 8 字节
    pwcl::set_ptbase(PAGE_SIZE_SHIFT);               // PT 基址位 = 12
    pwcl::set_ptwidth(PAGE_SIZE_SHIFT - 3);           // PT 索引宽度 = 9

    pwcl::set_dir1_base(PAGE_SIZE_SHIFT + PAGE_SIZE_SHIFT - 3);   // Dir1 基址 = 21
    pwcl::set_dir1_width(PAGE_SIZE_SHIFT - 3);                     // Dir1 宽度 = 9

    // 页表遍历控制——高位
    pwch::set_dir3_base(PAGE_SIZE_SHIFT + (PAGE_SIZE_SHIFT - 3) * 2); // Dir3 基址 = 30
    pwch::set_dir3_width(PAGE_SIZE_SHIFT - 3);                         // Dir3 宽度 = 9

    set_tlb_refill(tlbrentry);   // 设置 TLB 重填入口地址
}
```

这组配置告诉硬件页表的层级结构，使 `lddir`/`ldpte` 指令能够正确遍历页表。

### 5.5 TLB 刷新指令

```asm
invtlb  op, $rj, $rk
```

| op | 功能 |
|----|------|
| `0x00` | 清除所有 TLB 条目 |
| `0x05` | 清除指定虚拟地址的 TLB 条目 |

```rust
// 刷新所有 TLB
core::arch::asm!("dbar 0; invtlb 0x00, $r0, $r0");

// 刷新指定地址的 TLB
core::arch::asm!("dbar 0; invtlb 0x05, $r0, {reg}", reg = in(reg) vaddr);
```

**注意**：刷新前必须加 `dbar 0`（数据屏障），确保之前的内存操作已完成。这类似于 RISC-V 的 `sfence.vma`，但需要手动指定操作类型。

---

## 6. 异常与中断处理

### 6.1 异常类型

LoongArch 通过 **ESTAT** 寄存器（CSR 0x05）报告异常原因：

| 异常 | 含义 | RISC-V 对应 |
|------|------|-------------|
| `Syscall` | 系统调用 | `ecall` from U-mode |
| `Breakpoint` | 断点 | `ebreak` |
| `AddressNotAligned` | 地址未对齐 | Load/Store address misaligned |
| `InstructionNotExist` | 非法指令 | Illegal instruction |
| `LoadPageFault` | 加载页错误 | Load page fault |
| `StorePageFault` | 存储页错误 | Store page fault |
| `FetchPageFault` | 取指页错误 | Instruction page fault |
| `PageModifyFault` | 页修改错误（脏页） | — |
| `PagePrivilegeIllegal` | 页特权级错误 | — |
| `PageNonReadableFault` | 页不可读错误 | — |
| `Interrupt(n)` | 第 n 号中断 | External/Timer/Software interrupt |

**特有异常**：
- **`PageModifyFault`**：写一个 D（Dirty）位未设置的页时触发。LoongArch 的 D 位不会由硬件自动设置，需要软件在此异常中设置
- **`PagePrivilegeIllegal`**：访问权限不足的页（PLV 检查失败）
- **`AddressNotAligned`**：LoongArch 默认**不支持非对齐内存访问**，需要软件模拟

### 6.2 系统调用机制

| 对比项 | RISC-V | LoongArch |
|--------|--------|-----------|
| 发起指令 | `ecall` | `syscall 0` |
| 系统调用号 | `a7` (`x17`) | `$a7` (`$r11`) |
| 参数传递 | `a0–a5` | `$a0–$a5` |
| 返回值 | `a0` | `$a0`（`$r4`） |
| PC 推进 | 硬件自动（`sepc` 已指向 `ecall`） | 需要手动 +4 |

**重要**：LoongArch 的 `syscall` 指令执行后，ERA（Exception Return Address）指向的是 `syscall` 指令本身，所以陷入处理程序中必须手动将 ERA +4：

```rust
TrapType::UserEnvCall => {
    trap_cx.syscall_ok();  // sepc += 4
    let result = syscall(trap_cx.x[11], trap_cx.args());
    trap_cx.x[4] = result as usize;
}
```

### 6.3 中断配置

LoongArch 使用**线级中断**模型，通过 ECFG 寄存器配置：

```rust
let inter = LineBasedInterrupt::TIMER    // 第 11 号：定时器中断
    | LineBasedInterrupt::SWI0           // 第 0 号：软件中断 0
    | LineBasedInterrupt::SWI1           // 第 1 号：软件中断 1
    | LineBasedInterrupt::HWI0;          // 第 2 号：硬件中断 0
ecfg::set_lie(inter);
```

定时器中断号为 **11**，处理后需要手动清除：
```rust
11 => {
    ticlr::clear_timer_interrupt();
    TrapType::Time
}
```

### 6.4 异常入口

```
trap_vector_base:                    # 通用异常入口（EENTRY, CSR 0x0C）
    .balign 4096                     # 必须 4096 字节对齐！
    csrwr   $sp, KSAVE_USP          # 保存用户 SP
    csrrd   $sp, 0x1                # 读 PRMD
    andi    $sp, $sp, 0x3           # 提取 PLV
    bnez    $sp, user_vec           # PLV != 0 → 来自用户态

    # 来自内核态：直接在内核栈上分配空间保存上下文
    csrrd   $sp, KSAVE_USP          # 恢复内核 SP
    addi.d  $sp, $sp, -TRAPFRAME_SIZE
    SAVE_REGS
    ...

tlb_fill:                            # TLB 重填异常入口（TLBRENTRY, CSR 0x88）
    .balign 4096                     # 也必须 4096 字节对齐
    ...
```

**与 RISC-V 的差异**：
- LoongArch 有**两个独立的异常入口**：通用异常入口（EENTRY）和 TLB 重填入口（TLBRENTRY）
- RISC-V 只有一个 `stvec` 入口
- LoongArch 的对齐要求是 4096 字节，远大于 RISC-V 的 4 字节

### 6.5 异常返回

```asm
ertn    # Exception ReTurN
```

等效于 RISC-V 的 `sret`。执行时硬件自动将 PRMD 恢复到 CRMD（恢复特权级和中断状态），并跳转到 ERA 所指地址。

---

## 7. 上下文切换

### 7.1 用户态陷入（Trap In）

当用户态程序触发异常时的保存流程：

```
1. 硬件自动：
   - CRMD.PLV/IE → PRMD.PPLV/PIE（保存当前模式）
   - CRMD.PLV = 0, CRMD.IE = 0（切换到内核态，关中断）
   - PC → ERA（保存异常地址）
   - 跳转到 EENTRY

2. 软件（trap_vector_base）：
   - csrwr $sp, KSAVE_USP          // 用户 SP → SAVE2
   - 检查 PRMD.PLV 判断来源
   - 如果来自用户态：
     - csrrd $sp, KSAVE_CTX        // 加载 TrapFrame 地址
     - SAVE_REGS                   // 保存所有 32 个 GPR + PRMD + ERA
     - csrrd $sp, KSAVE_KSP        // 恢复内核栈指针
     - 恢复内核 callee-saved 寄存器
     - ret → 回到 trap_handler
```

**SAVE 寄存器用途**（暂存 CSR）：
- `SAVE0`（`0x30`）→ KSAVE_KSP：内核栈指针
- `SAVE1`（`0x31`）→ KSAVE_CTX：用户 TrapFrame 地址
- `SAVE2`（`0x32`）→ KSAVE_USP：用户栈指针

这比 RISC-V 的单个 `sscratch` 方便很多——不需要复杂的 `csrrw` 交换操作。

### 7.2 返回用户态（Trap Out）

```rust
pub fn trap_return() -> ! {
    let trap_cx_ptr = current_trap_cx_user_va();
    let user_token = current_user_token();
    activate_page_table(user_token);  // 切换到用户页表
    // 跳转到 user_restore
    asm!("move $a0, {trap_cx}", "jr {restore}",
         trap_cx = in(reg) trap_cx_ptr,
         restore = in(reg) user_restore as usize,
         options(noreturn));
}
```

`user_restore` 的流程：
```
1. 保存内核 callee-saved 寄存器到内核栈
2. csrwr $sp, KSAVE_KSP   // 保存内核 SP
3. move $sp, $a0           // SP = TrapFrame 地址
4. csrwr $a0, KSAVE_CTX   // 保存 TrapFrame 地址
5. LOAD_REGS               // 恢复所有用户寄存器 + PRMD + ERA
6. ertn                    // 返回用户态
```

### 7.3 内核任务切换

```rust
#[repr(C)]
pub struct KContext {
    ksp: usize,           // offset 0:  内核栈指针
    ktp: usize,           // offset 8:  线程指针
    _sregs: [usize; 10],  // offset 16: s9, s0–s8（10个 callee-saved 寄存器）
    kpc: usize,           // offset 104: 返回地址（实际存 $ra）
}
```

`context_switch(from, to)` 的汇编：
```asm
# 保存当前任务
st.d $sp, $a0, 0*8     # ksp
st.d $tp, $a0, 1*8     # ktp
st.d $s9, $a0, 2*8     # callee-saved
st.d $s0-$s8, ...      # callee-saved
st.d $ra, $a0, 12*8    # kpc (返回地址)

# 恢复新任务
ld.d $sp, $a1, 0*8
ld.d $tp, $a1, 1*8
ld.d $s9-$s8, ...
ld.d $ra, $a1, 12*8
ret                     # 跳转到新任务的 kpc
```

这与 RISC-V 的 `__switch` 非常相似，结构几乎一一对应：

| KContext 字段 | LoongArch 寄存器 | RISC-V 对应 |
|--------------|-----------------|-------------|
| ksp | `$sp` (`$r3`) | `sp` (`x2`) |
| ktp | `$tp` (`$r2`) | `tp` (`x4`) |
| _sregs | `$s0–$s9` | `s0–s11` |
| kpc | `$ra` (`$r1`) | `ra` (`x1`) |

---

## 8. 指令集速查

### 8.1 常用指令对照

| 操作 | LoongArch | RISC-V |
|------|-----------|--------|
| 加载双字 | `ld.d $rd, $rj, imm` | `ld rd, imm(rs1)` |
| 存储双字 | `st.d $rd, $rj, imm` | `sd rs2, imm(rs1)` |
| 加载字 | `ld.w $rd, $rj, imm` | `lw rd, imm(rs1)` |
| 加法 | `add.d $rd, $rj, $rk` | `add rd, rs1, rs2` |
| 立即数加法 | `addi.d $rd, $rj, imm` | `addi rd, rs1, imm` |
| 逻辑或立即数 | `ori $rd, $rj, imm` | `ori rd, rs1, imm` |
| 加载立即数 | `li.d $rd, imm` | `li rd, imm`（伪指令） |
| 函数调用 | `bl offset` | `jal offset` |
| 间接跳转 | `jirl $rd, $rj, offset` | `jalr rd, offset(rs1)` |
| 返回 | `ret`（= `jirl $zero, $ra, 0`） | `ret`（= `jalr zero, 0(ra)`） |
| 条件分支（非零） | `bnez $rj, offset` | `bnez rs1, offset` |
| CSR 读 | `csrrd $rd, csr` | `csrr rd, csr` |
| CSR 写 | `csrwr $rd, csr` | `csrw csr, rs1` |
| CSR 读写交换 | `csrxchg $rd, $rj, csr` | `csrrw rd, csr, rs1` |
| 内存屏障 | `dbar 0` | `fence` |
| 异常返回 | `ertn` | `sret` |
| TLB 刷新 | `invtlb op, $rj, $rk` | `sfence.vma rs1, rs2` |
| 系统调用 | `syscall 0` | `ecall` |
| 断点 | `break code` | `ebreak` |

### 8.2 加载大立即数

LoongArch 没有 RISC-V 的 `lui`+`addi` 组合，而是使用一系列位拼接指令：

```asm
lu12i.w  $rd, imm20     # rd[31:12] = imm20, rd[11:0] = 0, 符号扩展到 64 位
ori      $rd, $rd, imm12 # rd |= imm12
lu32i.d  $rd, imm20     # rd[51:32] = imm20
lu52i.d  $rd, $rj, imm12 # rd[63:52] = imm12, rd[51:0] = rj[51:0]
```

例如设置 DMW0 = `0x8000_0000_0000_0001`：
```asm
ori      $t0, $zero, 0x1     # t0 = 0x0000_0000_0000_0001
lu52i.d  $t0, $t0, -2048     # t0[63:52] = 0x800 → t0 = 0x8000_0000_0000_0001
```

### 8.3 全局地址加载

```asm
la.global $rd, symbol    # 加载符号的虚拟地址到 $rd（伪指令，展开为多条）
```

---

## 9. 启动流程

### 9.1 entry.S

```asm
_start:
    # 1. 配置 DMW（直接映射窗口）
    ori      $t0, $zero, 0x1
    lu52i.d  $t0, $t0, -2048       # DMW0: UC, PLV0, 0x8000...
    csrwr    $t0, 0x180

    ori      $t0, $zero, 0x11
    lu52i.d  $t0, $t0, -1792       # DMW1: CA, PLV0, 0x9000...
    csrwr    $t0, 0x181

    # 2. 启用分页模式
    li.w     $t0, 0xb0             # CRMD: PLV=0, PG=1
    csrwr    $t0, 0x0

    # 3. 初始化 PRMD
    li.w     $t0, 0x00             # PRMD: PLV=0, PIE=0
    csrwr    $t0, 0x1

    # 4. 禁用 FPU/SIMD
    li.w     $t0, 0x00
    csrwr    $t0, 0x2              # EUEN

    # 5. 设置栈指针
    la.global $sp, boot_stack_top

    # 6. 读取 CPU ID
    csrrd    $a0, 0x20

    # 7. 跳转到 Rust 入口
    la.global $t0, rust_main
    jirl     $zero, $t0, 0
```

### 9.2 rust_main

```rust
pub fn rust_main() -> ! {
    clear_bss();                    // 清零 BSS 段
    console_init();                 // 初始化 UART
    logging::init();                // 初始化日志
    mm::init();                     // 初始化内存管理
    trap_init();                    // 设置异常入口 + TLB 初始化
    trap_enable_timer_interrupt();  // 开启定时器中断
    set_next_trigger();             // 设置下一次定时器
    sigtrx::init();                 // 初始化信号 trampoline
    task::add_initproc();           // 添加初始进程
    task::run_tasks();              // 开始任务调度
}
```

---

## 10. 与 RISC-V 对比速查表

| 维度 | RISC-V (Sv39) | LoongArch64 |
|------|--------------|-------------|
| **特权级** | M/S/U 三级 | PLV0–PLV3 四级（实际用 0/3） |
| **内核地址映射** | 页表映射 | DMW 直接映射（无需页表） |
| **页表结构** | 三级，512 项/级 | 三级，512 项/级（相同） |
| **PTE 格式** | PPN 在 [53:10] | PPN 在 [47:12]，物理地址直接存放 |
| **TLB 管理** | 硬件自动 page walk | 软件重填（硬件加速指令） |
| **异常入口** | 1 个（stvec） | 2 个（EENTRY + TLBRENTRY） |
| **入口对齐** | 4 字节 | 4096 字节 |
| **暂存 CSR** | 1 个（sscratch） | 4 个（SAVE0–SAVE3） |
| **异常返回** | `sret` | `ertn` |
| **内存屏障** | `fence` / `sfence.vma` | `dbar` / `invtlb` |
| **系统调用指令** | `ecall` | `syscall 0` |
| **时钟中断** | CLINT / SBI 触发 | 内置定时器（TCFG 寄存器） |
| **外部中断** | PLIC | EIOINTC / EXTIOI |
| **非对齐访问** | 通常硬件支持 | 默认不支持，需软件模拟 |
| **浮点控制** | mstatus.FS | EUEN.FPE |

---

## 11. 移植要点与陷阱

### 11.1 必须注意的陷阱

1. **TLB 重填必须正确**：这是 LoongArch 与 RISC-V 最大的差异。如果 `tlb_fill` 处理程序有 bug，系统会在第一次 TLB miss 时崩溃。确保 PWCL/PWCH 配置与页表层级完全匹配。

2. **PTE 格式不同**：如果直接复用 RISC-V 的 PTE 格式（PPN 右移 10 位），需要在写入页表和 TLB 操作时做转换。建议尽早统一为 LoongArch 原生格式。

3. **异常入口对齐**：`trap_vector_base` 和 `tlb_fill` 都必须 `.balign 4096`。如果对齐不正确，`csrwr` 写入 EENTRY/TLBRENTRY 时会截断低位导致跳转到错误地址。

4. **ERA 需要手动推进**：处理 `syscall` 和 `break` 后必须 `ERA += 4`。RISC-V 的 `ecall` 也需要 `sepc += 4`，但有些 RISC-V 实现会自动完成。

5. **脏页（D 位）由软件管理**：LoongArch 不会自动设置 PTE 的 D 位。首次写入一个 D=0 的页会触发 `PageModifyFault`，需要在异常处理中设置 D 位并重填 TLB。

6. **页表切换必须刷新 TLB**：每次修改 PGDL 后必须执行 `dbar 0; invtlb 0x00, $r0, $r0`。忘记刷新会导致旧的 TLB 条目继续生效，产生难以调试的映射错误。

7. **非对齐访问异常**：如果用户态程序（如 musl libc）进行非对齐内存访问，会触发 `AddressNotAligned` 异常，需要在内核中模拟。这在 RISC-V 上通常不是问题。

### 11.2 从 rCore (RISC-V) 移植的关键修改点

| 模块 | 需要修改的内容 |
|------|---------------|
| `entry.S` | 完全重写：DMW 配置、CRMD/PRMD 初始化、启动栈设置 |
| `trap.rs` | 重写异常入口汇编、异常分发逻辑、中断处理 |
| `context.rs` | TrapFrame 字段名（`sstatus`→`prmd`, `sepc`→`era`）、寄存器编号映射 |
| `kcontext.rs` | KContext 的 callee-saved 寄存器组不同、`context_switch` 汇编重写 |
| `page_table.rs` | PTE 标志位定义重写、物理地址提取逻辑、页表切换函数 |
| `timer.rs` | 使用 TCFG/TICLR 替代 SBI timer |
| `mm/` | 内核地址映射改为 DMW 方式，`PhysAddr ↔ VirtAddr` 转换逻辑修改 |
| 新增 `unaligned.rs` | 非对齐访问模拟（RISC-V 不需要） |
| 新增 `sigtrx.rs` | 信号 trampoline（原理相同但实现不同） |

### 11.3 调试建议

1. **优先验证 TLB 重填**：在其他功能之前，确保内核能正确处理用户态 TLB miss
2. **检查 BADV 寄存器**：页错误时 `BADV`（CSR 0x07）保存了出错地址，等效于 RISC-V 的 `stval`
3. **检查 ESTAT**：异常原因码在 `ESTAT`（CSR 0x05），查看 `estat.cause()` 确认异常类型
4. **使用 SAVE 寄存器调试**：可以在异常处理程序中用多余的 SAVE 寄存器暂存调试值
5. **QEMU 调试**：LoongArch QEMU (`qemu-system-loongarch64`) 支持 GDB stub，调试方法与 RISC-V 类似

---

## 参考资料

- [龙芯架构参考手册（卷一）](https://loongson.github.io/LoongArch-Documentation/LoongArch-Vol1-CN.html)
- [Linux 内核 LoongArch 实现](https://github.com/torvalds/linux/tree/master/arch/loongarch)
- OSKernel2025-rustoswhu 参考实现：`/Users/mac/Desktop/project/OSKernel2025-rustoswhu/arch/src/loongarch64/`
- rcore-lab LoongArch64 移植：`os/src/arch/loongarch64/`
