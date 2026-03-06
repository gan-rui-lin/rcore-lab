# 多架构移植分析：OSKernel2025-rustoswhu 的 HAL 抽象模式

**日期**: 2026/3/6

**分析目标**: 深入分析 OSKernel2025-rustoswhu 仓库如何实现多架构（RISC-V64 / x86-64 / AArch64 / LoongArch64）支持，提炼其架构抽象设计思路，为 rcore-lab 的潜在多架构移植提供参考。

---

## 子文档索引

本文档为总纲，各模块的详细分析见以下子文档：

| 模块 | 子文档 | 核心内容 |
|------|-------|---------|
| Trap 处理 | [多架构移植_Trap处理抽象.md](多架构移植_Trap处理抽象.md) | TrapFrame 结构体、Index trait、异常分类、汇编入口 |
| 页表 | [多架构移植_页表抽象.md](多架构移植_页表抽象.md) | PTE 编码、MappingFlags 转换、通用遍历算法、TLB 刷新 |
| 启动流程 | [多架构移植_启动流程抽象.md](多架构移植_启动流程抽象.md) | 各架构启动序列、初始页表、链接脚本、内存发现 |
| 上下文切换 | [多架构移植_上下文切换抽象.md](多架构移植_上下文切换抽象.md) | KContext 结构体、callee-saved 寄存器、TLS 机制 |
| 定时器与中断 | [多架构移植_定时器与中断抽象.md](多架构移植_定时器与中断抽象.md) | Time 结构体、PLIC/APIC/GIC、中断控制器 |
| 信号跳板 | [多架构移植_信号跳板抽象.md](多架构移植_信号跳板抽象.md) | sigreturn 调用号、页表映射、SigAction 布局差异 |
| LoongArch 启动深度分析 | [多架构移植_LoongArch启动流程深度分析.md](多架构移植_LoongArch启动流程深度分析.md) | DMW 直接映射窗口、与 RV64 的对比、TLB 管理差异 |

---

## 一、背景知识

### 1.1 为什么需要多架构支持

操作系统内核天然与底层硬件紧密耦合。不同 ISA（指令集架构）在以下方面存在根本差异：

- **寄存器集合与用途**：RISC-V 有 32 个通用寄存器（x0-x31），x86-64 有 16 个（rax, rbx, ...），AArch64 有 31 个（x0-x30）+ SP，LoongArch64 有 32 个
- **特权级模型**：RISC-V 使用 M/S/U 三级特权，x86-64 使用 Ring 0-3，AArch64 使用 EL0-EL3，LoongArch64 使用 PLV0-PLV3
- **页表格式**：RISC-V Sv39 三级页表，x86-64 四级页表（PML4），AArch64 可配置级数，LoongArch64 软件管理 TLB
- **中断机制**：RISC-V 通过 CSR（scause/stval/stvec），x86-64 通过 IDT，AArch64 通过异常向量表，LoongArch64 通过 ESTAT/ERA 等 CSR
- **系统调用 ABI**：调用号寄存器、参数传递寄存器、返回值寄存器各不相同

### 1.2 OSKernel2025-rustoswhu 概况

该项目是参加 2025 年 OS 内核赛的作品，基于 Rust 实现，已成功支持四种架构在 QEMU 上运行。其核心设计思路是**将架构相关代码抽取到独立的 `arch` crate 中**，内核主体（`os/`）完全架构无关。

---

## 二、整体架构设计

### 2.1 项目目录结构

```
OSKernel2025-rustoswhu/
├── arch/                       # 硬件抽象层（HAL）- 核心
│   └── src/
│       ├── lib.rs              # 条件编译入口 + 公共类型定义
│       ├── api.rs              # ArchInterface trait 定义
│       ├── addr.rs             # PhysAddr/VirtAddr/PhysPage/VirtPage
│       ├── pagetable.rs        # 通用页表逻辑（map/unmap/translate）
│       ├── consts.rs           # 全局常量 + bit!() 宏
│       ├── time.rs             # 通用时间接口
│       ├── irq.rs              # 中断接口
│       ├── riscv64/            # RISC-V 64 实现
│       ├── x86_64/             # x86-64 实现
│       ├── aarch64/            # AArch64 实现
│       └── loongarch64/        # LoongArch64 实现
├── os/                         # 内核主体（架构无关）
│   └── src/
│       ├── main.rs             # ArchInterface 实现 + 内核入口
│       ├── task/               # 进程/线程管理
│       ├── mm/                 # 内存管理（使用 arch 提供的 PageTable）
│       ├── syscall/            # 系统调用（架构无关逻辑）
│       ├── fs/                 # 文件系统
│       └── linker-*.ld         # 各架构链接脚本
├── sync/                       # 同步原语
├── vfs/                        # 虚拟文件系统
├── ext4/                       # Ext4 文件系统
└── Cargo.toml                  # 工作空间配置
```

### 2.2 核心分层思想

```
┌───────────────────────────────────────────────┐
│           os/ (架构无关的内核逻辑)               │
│  syscall, task, mm, fs, signal, socket...     │
│                                               │
│  通过 TrapFrame[TrapFrameArgs] 访问寄存器       │
│  通过 PageTable.map_page() 操作页表             │
│  通过 ArchInterface::kernel_interrupt() 处理中断│
├───────────────────────────────────────────────┤
│           arch/ (硬件抽象层)                     │
│                                               │
│  公共接口层:                                    │
│    TrapFrameArgs enum, TrapType enum          │
│    MappingFlags bitflags, PTE struct           │
│    PageTable struct, TLB struct                │
│    ArchInterface trait                         │
│                                               │
│  架构实现层 (编译期选择其一):                     │
│    riscv64/ | x86_64/ | aarch64/ | loongarch64/ │
└───────────────────────────────────────────────┘
```

关键原则：**os/ 中的代码不包含任何 `#[cfg(target_arch)]`，所有架构差异都封装在 arch/ 内部**。

---

## 三、底层细节屏蔽机制

### 3.1 编译期架构选择（`#[cfg_attr]` 模式）

这是整个多架构支持的基石。在 `arch/src/lib.rs` 中：

```rust
#[cfg_attr(target_arch = "riscv64", path = "riscv64/mod.rs")]
#[cfg_attr(target_arch = "aarch64", path = "aarch64/mod.rs")]
#[cfg_attr(target_arch = "x86_64", path = "x86_64/mod.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "loongarch64/mod.rs")]
mod currrent_arch;

pub use currrent_arch::*;
```

**工作原理**：Rust 编译器根据 `--target` 参数自动选择对应的模块文件。每个架构的 `mod.rs` 导出相同名称的类型和函数，上层代码只需 `use arch::*` 就能获得正确的架构实现。

**优势**：
- 零运行时开销（纯编译期分发）
- 未选中的架构代码完全不参与编译
- 类型安全——如果某个架构忘记实现某个接口，编译直接报错

### 3.2 Trap 上下文的统一抽象（Index trait 模式）

这是该项目最精妙的设计之一。不同架构的寄存器命名和编号完全不同，但内核需要统一地读写"返回地址"、"栈指针"、"系统调用号"等。

**抽象层定义** (`arch/src/lib.rs`):

```rust
pub enum TrapFrameArgs {
    SEPC,       // 异常 PC（不叫 sepc/rip/elr，用统一名称）
    RA,         // 返回地址
    SP,         // 栈指针
    RET,        // 返回值寄存器
    ARG0, ARG1, ARG2,  // 系统调用参数
    TLS,        // 线程本地存储指针
    SYSCALL,    // 系统调用号
}
```

**RISC-V 实现** (`arch/src/riscv64/context.rs`):

```rust
pub struct TrapFrame {
    pub x: [usize; 32],    // 32 个通用寄存器
    pub sstatus: Sstatus,
    pub sepc: usize,
    pub fsx: [usize; 2],   // 浮点扩展
}

impl Index<TrapFrameArgs> for TrapFrame {
    type Output = usize;
    fn index(&self, index: TrapFrameArgs) -> &Self::Output {
        match index {
            TrapFrameArgs::SEPC    => &self.sepc,
            TrapFrameArgs::RA      => &self.x[1],   // ra
            TrapFrameArgs::SP      => &self.x[2],   // sp
            TrapFrameArgs::RET     => &self.x[10],  // a0
            TrapFrameArgs::ARG0    => &self.x[10],  // a0
            TrapFrameArgs::ARG1    => &self.x[11],  // a1
            TrapFrameArgs::ARG2    => &self.x[12],  // a2
            TrapFrameArgs::TLS     => &self.x[4],   // tp
            TrapFrameArgs::SYSCALL => &self.x[17],  // a7
        }
    }
}
```

**x86-64 对应映射**（对比）:
- `SEPC` → `self.rip`
- `ARG0/ARG1/ARG2` → `self.rdi / self.rsi / self.rdx`
- `SYSCALL` → `self.rax`

**AArch64 对应映射**:
- `SEPC` → `self.elr`
- `RA` → `self.regs[30]` (x30 即 link register)
- `SYSCALL` → `self.regs[8]` (x8)

**内核使用方式**（完全架构无关）:

```rust
// os/src/main.rs 中的 kernel_interrupt 实现
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    match trap_type {
        TrapType::UserEnvCall => {
            ctx.syscall_ok();                           // 推进 PC
            let id = ctx[TrapFrameArgs::SYSCALL];       // 获取调用号
            let args = ctx.args();                      // 获取参数
            let result = syscall(id, args);
            ctx[TrapFrameArgs::RET] = result as usize;  // 写回返回值
        }
        TrapType::StorePageFault(addr) => { /* ... */ }
        // ...
    }
}
```

### 3.3 Trap 类型的统一分类（TrapType enum）

不同架构的异常/中断编码完全不同，但从内核视角看，需要处理的事件类型是有限且一致的：

```rust
pub enum TrapType {
    Breakpoint,                     // 断点
    UserEnvCall,                    // 系统调用
    Time,                           // 时钟中断
    Unknown,                        // 未知/不处理
    SupervisorExternal,             // 外部中断
    StorePageFault(usize),          // 写页错误（附带错误地址）
    LoadPageFault(usize),           // 读页错误
    InstructionPageFault(usize),    // 取指页错误
    IllegalInstruction(usize),      // 非法指令
}
```

每个架构在自己的 `interrupt.rs` / `trap.rs` 中负责将硬件异常码转换为 `TrapType`：

| 架构 | 异常源寄存器 | 转换位置 |
|------|-------------|---------|
| RISC-V | `scause` + `stval` | `riscv64/interrupt.rs::kernel_callback()` |
| x86-64 | IDT vector number + CR2 | `x86_64/interrupt.rs::kernel_callback()` |
| AArch64 | `ESR_EL1` + `FAR_EL1` | `aarch64/trap.rs::handle_exception()` |
| LoongArch64 | `ESTAT` + `BADV` | `loongarch64/trap.rs::loongarch64_trap_handler()` |

### 3.4 ArchInterface trait（内核-HAL 回调接口）

使用 `crate_interface` crate 实现跨 crate 的 trait 回调：

```rust
// arch/src/api.rs
#[crate_interface::def_interface]
pub trait ArchInterface {
    fn init_allocator();
    fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType);
    fn init_logging();
    fn add_memory_region(start: usize, end: usize);
    fn main(hartid: usize);
    fn frame_alloc_persist() -> PhysPage;
    fn frame_unalloc(ppn: PhysPage);
    fn prepare_drivers();
    fn try_to_add_device(fdtNode: &FdtNode);
}
```

**设计动机**：`arch` 层在初始化过程中需要回调内核的内存分配器、日志系统等，但 `arch` 作为底层 crate 不能直接依赖 `os` crate（会产生循环依赖）。`crate_interface` 通过编译期链接解决了这个问题——`arch` 定义接口，`os` 提供实现，在编译时自动绑定。

---

## 四、需要改动的关键模块

如果要将 rcore-lab 改造为多架构支持，以下模块需要重点改动：

### 4.1 Trap 处理（优先级：最高）

> 详细分析见 **[多架构移植_Trap处理抽象.md](多架构移植_Trap处理抽象.md)**

**当前 rcore-lab 状态**: `os/src/trap/` 目录下有 `trap.S`（RISC-V 汇编）、`context.rs`（TrapContext 定义）、`mod.rs`（trap_handler 逻辑），全部硬编码为 RISC-V。

**需要改动的内容**:

| 组件 | 当前状态 | 改造方向 |
|------|---------|---------|
| `trap.S` | RISC-V 汇编，保存 x0-x31 + sstatus + sepc | 每个架构单独实现 `.S` 文件 |
| `TrapContext` | 直接包含 `x: [usize; 32]`, `sstatus`, `sepc` | 抽取为架构相关结构体 + `Index<TrapFrameArgs>` |
| `trap_handler()` | 直接读取 `scause`、`stval` 等 RISC-V CSR | 改为接收 `TrapType` 参数，由底层转换 |
| `__alltraps` / `__restore` | RISC-V 专用入口 | 每架构独立实现 |
| 信号跳板 | 硬编码 RISC-V 指令 | 每架构提供信号跳板实现 |

**rustoswhu 的做法**: 将 trap 入口点（`kernelvec`/`uservec`/`user_restore`）用 `#[naked]` 函数 + 内联汇编实现在各架构的 `interrupt.rs` 中，然后统一调用 `kernel_callback()` → `ArchInterface::kernel_interrupt()` 回到内核。

### 4.2 页表（优先级：最高）

> 详细分析见 **[多架构移植_页表抽象.md](多架构移植_页表抽象.md)**

**当前 rcore-lab 状态**: `os/src/mm/page_table.rs` 直接操作 RISC-V Sv39 格式的页表项。

**需要改动的内容**:

| 组件 | 当前状态 | 改造方向 |
|------|---------|---------|
| `PageTableEntry` | 包含 RISC-V PTE flags (V/R/W/X/U/A/D) | 抽取为 `PTE` + 架构 `PTEFlags` |
| `PageTable` | 固定三级页表遍历 | 支持可变级数（x86-64 四级，其他三级） |
| `MappingFlags` | 无（直接用架构 flags） | 引入架构无关 `MappingFlags` + 双向转换 |
| `satp` 操作 | 直接操作 RISC-V `satp` CSR | 抽取为 `PageTable::change()` |
| TLB 刷新 | `sfence.vma` | 抽取为 `TLB::flush_vaddr()` / `TLB::flush_all()` |
| 地址类型 | `PhysAddr` / `VirtAddr` 带 RISC-V 偏移 | 通用地址类型 + 架构常量 `VIRT_ADDR_START` |

**rustoswhu 的做法**：
- `pagetable.rs` 中实现**通用的多级页表遍历算法**，通过 `Self::PAGE_LEVEL` 常量区分三级/四级
- 每架构提供 `PTE` 的 `new_page()`/`new_table()`/`flags()`/`is_valid()`/`is_table()`/`address()` 方法
- `MappingFlags` ↔ `PTEFlags` 通过 `From` trait 双向转换
- `TLB` 结构体的 `flush_vaddr()`/`flush_all()` 由各架构实现

**各架构 PTE 格式对比**:

| 架构 | PTE 大小 | 关键 flags | 地址提取方式 |
|------|---------|-----------|------------|
| RISC-V Sv39 | 64-bit | V(0) R(1) W(2) X(3) U(4) A(6) D(7) | bits[53:10] << 12 |
| x86-64 | 64-bit | P(0) RW(1) US(2) A(5) D(6) PS(7) XD(63) | bits[51:12] << 12 |
| AArch64 | 64-bit | VALID(0) AF(10) AP_RO(7) UXN(54) PXN(53) | bits[47:12] << 12 |
| LoongArch64 | 64-bit | V(0) D(1) PLV(2-3) W(8) NR(11) NX(12) | 平台相关 |

### 4.3 启动流程（优先级：高）

> 详细分析见 **[多架构移植_启动流程抽象.md](多架构移植_启动流程抽象.md)**

**需要改动的内容**:

| 组件 | 改造方向 |
|------|---------|
| `entry.asm` | 每架构单独实现（设置栈、启用分页、跳转到 Rust） |
| `linker.ld` | 每架构独立链接脚本（`linker-riscv64.ld`、`linker-x86_64.ld` 等） |
| SBI 调用 | RISC-V 用 SBI，x86 用 BIOS/UEFI，AArch64 用 PSCI |
| 内核地址空间 | `VIRT_ADDR_START` 不同：RISC-V `0xffff_ffc0_0000_0000`，x86 `0xffff_ff80_0000_0000` 等 |

### 4.4 上下文切换（优先级：高）

> 详细分析见 **[多架构移植_上下文切换抽象.md](多架构移植_上下文切换抽象.md)**

**当前 rcore-lab 状态**: `os/src/task/context.rs` 定义 `TaskContext`（ra + sp + s0-s11），`os/src/task/switch.S` 用 RISC-V 汇编实现。

**改造方向**:
- 引入 `KContext` 结构体（各架构只保存 callee-saved 寄存器）
- `KContextArgs` enum 提供统一访问（KSP/KTP/KPC）
- `context_switch()` 用 `#[naked]` 函数实现

**各架构 callee-saved 寄存器对比**:

| 架构 | callee-saved 寄存器 | 总字段数 |
|------|-------------------|---------|
| RISC-V | s0-s11 (12个) + ra + sp + tp | 15 |
| x86-64 | rbx, rbp, r12-r15 (6个) + rsp + kpc | 8+ |
| AArch64 | x19-x29 (11个) + sp + lr + tpidr | 14 |
| LoongArch64 | s0-s8 (9个) + ra + sp + tp | 12 |

### 4.5 定时器与中断控制器（优先级：中）

> 详细分析见 **[多架构移植_定时器与中断抽象.md](多架构移植_定时器与中断抽象.md)**

| 架构 | 定时器 | 中断控制器 |
|------|-------|-----------|
| RISC-V | `rdtime` CSR + SBI `set_timer` | PLIC |
| x86-64 | RDTSC + LAPIC timer | APIC (x2apic) |
| AArch64 | Generic Timer (CNTPCT_EL0) | GICv2/GICv3 |
| LoongArch64 | 稳定计数器 CSR | 内置中断控制器 |

**rustoswhu 的做法**: 在 `arch/src/time.rs` 定义通用 `Time` 结构体，各架构实现 `get_freq()` 和 `now()` 方法。中断控制器在各架构的 `boards/` 模块中初始化。

### 4.6 SBI / 底层接口（优先级：中）

rcore-lab 的 `os/src/sbi.rs` 直接调用 RISC-V SBI。多架构支持时：

| 架构 | 底层接口 | 功能 |
|------|---------|------|
| RISC-V | SBI (ecall) | 控制台输出、设定时钟、关机 |
| x86-64 | BIOS/UEFI + 端口 I/O | 串口、ACPI 关机 |
| AArch64 | PSCI + MMIO | PL011 串口、PSCI 关机 |
| LoongArch64 | CSR + MMIO | 串口、直接操作 |

### 4.7 信号跳板（优先级：中）

> 详细分析见 **[多架构移植_信号跳板抽象.md](多架构移植_信号跳板抽象.md)**

信号处理需要在用户态执行信号 handler 后返回内核。rustoswhu 在每个架构目录下实现 `sigtrx.rs`，将信号跳板代码（包含 `sigreturn` 系统调用的汇编序列）映射到用户地址空间的固定位置 `SIG_RETURN_ADDR`。

| 架构 | 信号跳板指令 |
|------|------------|
| RISC-V | `li a7, 139; ecall` (sys_rt_sigreturn) |
| x86-64 | `mov rax, 15; syscall` |
| AArch64 | `mov x8, 139; svc 0` |
| LoongArch64 | `li.w a7, 139; syscall 0` |

---

## 五、架构抽象设计模式总结

### 5.1 模式一：条件编译模块选择

```rust
// 最基础的模式：编译器根据 target 选择不同的模块文件
#[cfg_attr(target_arch = "riscv64", path = "riscv64/mod.rs")]
#[cfg_attr(target_arch = "aarch64", path = "aarch64/mod.rs")]
mod currrent_arch;
pub use currrent_arch::*;
```

**适用场景**: 大块的架构实现代码（trap 入口、页表操作、启动流程等）。

### 5.2 模式二：Index trait 寄存器抽象

```rust
// 定义统一的寄存器名
enum TrapFrameArgs { SEPC, RA, SP, RET, ARG0, SYSCALL, ... }

// 各架构实现映射
impl Index<TrapFrameArgs> for TrapFrame {
    fn index(&self, arg: TrapFrameArgs) -> &usize {
        match arg {
            SEPC    => &self.sepc,    // RISC-V
            // SEPC => &self.rip,     // x86-64
            // SEPC => &self.elr,     // AArch64
            SYSCALL => &self.x[17],   // a7 on RISC-V
            // SYSCALL => &self.rax,  // x86-64
            ...
        }
    }
}
```

**适用场景**: 需要在架构无关代码中读写特定语义寄存器的场景。

### 5.3 模式三：trait 回调接口（crate_interface）

```rust
// 底层定义接口
#[def_interface]
pub trait ArchInterface {
    fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType);
    fn frame_alloc_persist() -> PhysPage;
    ...
}

// 上层实现接口
#[impl_interface]
impl ArchInterface for KernelImpl {
    fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
        // 内核通用中断处理逻辑
    }
}
```

**适用场景**: 底层（arch）需要调用上层（os）功能的场景，避免循环依赖。

### 5.4 模式四：From trait 标志位转换

```rust
// 通用标志
bitflags! { struct MappingFlags { R, W, X, U, P, ... } }

// RISC-V 架构标志
bitflags! { struct PTEFlags { V, R, W, X, U, A, D, G, ... } }

// 双向转换
impl From<MappingFlags> for PTEFlags { ... }
impl From<PTEFlags> for MappingFlags { ... }
```

**适用场景**: 页表权限标志的统一表示，各架构 PTE 格式不同但语义相似。

### 5.5 模式五：常量参数化

```rust
impl PageTable {
    pub(crate) const PAGE_SIZE: usize = 0x1000;
    pub(crate) const PAGE_LEVEL: usize = 3;  // RISC-V / AArch64 / LoongArch
    //                                    4;  // x86-64
    pub(crate) const VADDR_BITS: usize = 39; // RISC-V
    //                                    48; // x86-64
}
```

配合 `pagetable.rs` 中的通用遍历算法，通过 `if Self::PAGE_LEVEL == 4` 在编译期展开/消除分支。

---

## 六、与 rcore-lab 的对比分析

### 6.1 当前 rcore-lab 的架构耦合点

| 文件 | 耦合内容 | 解耦难度 |
|------|---------|---------|
| `os/src/trap/trap.S` | RISC-V 汇编（保存/恢复 32 个寄存器） | 高（需完全重写） |
| `os/src/trap/context.rs` | `TrapContext { x: [usize; 32], sstatus, sepc }` | 中（引入 Index trait） |
| `os/src/trap/mod.rs` | 直接读取 `scause`/`stval` CSR | 中（引入 TrapType） |
| `os/src/mm/page_table.rs` | Sv39 PTE 格式、`satp` 操作 | 高（需重构整个页表） |
| `os/src/mm/address.rs` | RISC-V 地址空间布局常量 | 低（提取为常量） |
| `os/src/mm/memory_set.rs` | 内核空间映射（MMIO 地址等） | 中（board 相关） |
| `os/src/task/context.rs` | `TaskContext { ra, sp, s0-s11 }` | 中 |
| `os/src/task/switch.S` | RISC-V context switch 汇编 | 高（需完全重写） |
| `os/src/entry.asm` | RISC-V 启动汇编 | 高 |
| `os/src/sbi.rs` | RISC-V SBI 调用 | 中（替换为平台接口） |
| `os/src/timer.rs` | RISC-V SBI 定时器 | 低 |
| `os/src/linker.ld` | RISC-V 地址空间布局 | 低（每架构一份） |

### 6.2 推荐的移植策略

**第一步（基础设施）**: 创建 `arch/` crate，定义公共类型
- `TrapFrameArgs` / `TrapType` 枚举
- `MappingFlags` bitflags
- `PhysAddr` / `VirtAddr` / `PhysPage` / `VirtPage` 类型
- `ArchInterface` trait

**第二步（Trap 子系统）**: 迁移 trap 相关代码
- 将 `trap.S` 移入 `arch/src/riscv64/`
- 将 `TrapContext` 改为 `TrapFrame` + `Index<TrapFrameArgs>` 实现
- 将 `trap_handler` 中的 CSR 读取逻辑移入 `arch/`，只暴露 `TrapType`

**第三步（页表子系统）**: 迁移页表
- 将 PTE 格式相关代码移入 `arch/src/riscv64/page_table/`
- 在 `arch/src/pagetable.rs` 实现通用的 `map_page`/`unmap_page`/`translate`
- 引入 `TLB` 抽象

**第四步（上下文切换）**: 迁移调度
- 将 `switch.S` 移入 `arch/`
- 引入 `KContext` + `KContextArgs`

**第五步（启动和外设）**: 迁移启动流程
- 将 `entry.asm`、`sbi.rs` 移入 `arch/`
- 引入 board 配置机制

---

## 七、构建系统适配

### 7.1 Cargo 条件依赖

```toml
# arch/Cargo.toml
[target.'cfg(target_arch = "riscv64")'.dependencies]
riscv = { version = "0.11", features = ["inline-asm"] }
sbi-rt = { version = "0.0.2", features = ["legacy"] }

[target.'cfg(target_arch = "x86_64")'.dependencies]
x86 = "0.52"
x86_64 = "0.14.12"
```

### 7.2 Makefile 适配

```makefile
ARCH ?= riscv64

ifeq ($(ARCH), riscv64)
    TARGET := riscv64gc-unknown-none-elf
    QEMU := qemu-system-riscv64 -machine virt
else ifeq ($(ARCH), x86_64)
    TARGET := x86_64-unknown-none
    QEMU := qemu-system-x86_64 -machine q35
else ifeq ($(ARCH), aarch64)
    TARGET := aarch64-unknown-none-softfloat
    QEMU := qemu-system-aarch64 -machine virt
endif

# 使用: make run ARCH=aarch64
```

### 7.3 链接脚本

每个架构需要独立的链接脚本，主要差异在于：
- 内核加载地址（RISC-V: `0x80200000`，x86: `0x200000`，AArch64: `0x40080000`）
- 虚拟地址起始（各架构 `VIRT_ADDR_START` 不同）
- 段对齐要求

在 `.cargo/config.toml` 中通过 target-specific rustflags 选择链接脚本：

```toml
[target.riscv64gc-unknown-none-elf]
rustflags = ["-Clink-arg=-Tsrc/linker-riscv64.ld"]

[target.x86_64-unknown-none]
rustflags = ["-Clink-arg=-Tsrc/linker-x86_64.ld"]
```

---

## 八、关键结论

### 8.1 rustoswhu 方案的优点

1. **零抽象开销**: 所有分发在编译期完成，没有虚函数/动态分发
2. **类型安全**: Rust 编译器确保每个架构都正确实现了所有接口
3. **模块化清晰**: `arch/` 和 `os/` 之间有明确的边界
4. **增量移植友好**: 可以先只支持 RISC-V（现状），逐步添加新架构
5. **代码复用最大化**: 页表遍历、地址类型等通用逻辑只写一份

### 8.2 需要注意的陷阱

1. **信号处理的架构差异远比想象中大**: 不仅仅是寄存器映射，还涉及信号栈布局、ucontext 结构、信号跳板代码等
2. **页表格式差异**: x86-64 的 4 级页表 vs 其他架构的 3 级需要在遍历算法中特别处理
3. **syscall ABI 差异**: 参数传递寄存器、返回值约定、错误码约定各不相同
4. **FPU/SIMD 状态**: 各架构的浮点/向量寄存器保存方式差异巨大（RISC-V 的 F/D 扩展 vs x86 的 FXSAVE vs ARM 的 NEON）
5. **内存一致性模型**: RISC-V 弱内存序 vs x86 强内存序，影响同步原语实现

### 8.3 对 rcore-lab 的建议

鉴于 rcore-lab 当前专注于 RISC-V + busybox 功能完善，**短期内不建议立即进行多架构改造**。但可以借鉴以下设计原则进行渐进式重构：

1. 将 `TrapContext` 的字段访问改为通过方法/常量进行，减少直接字段访问
2. 将 trap_handler 中的 CSR 读取逻辑集中到一处
3. 将页表的 PTE 操作封装为方法，避免散落在各处的位运算
4. 将 `VIRT_ADDR_START` 等地址常量集中定义

这些改动即使不进行多架构移植，也能提高代码的可维护性。

---

## 附录：各架构关键差异速查表

| 特性 | RISC-V64 | x86-64 | AArch64 | LoongArch64 |
|------|----------|--------|---------|-------------|
| 通用寄存器数 | 32 (x0-x31) | 16 | 31 (x0-x30) + SP | 32 |
| 页表级数 | 3 (Sv39) | 4 (PML4) | 3-4 | 3 |
| 虚拟地址位数 | 39 | 48 | 39/48 | 39 |
| 异常 PC | sepc | RIP (栈上) | ELR_EL1 | ERA |
| 异常原因 | scause | vector number | ESR_EL1 | ESTAT |
| 错误地址 | stval | CR2 | FAR_EL1 | BADV |
| 系统调用指令 | ecall | syscall | svc #0 | syscall 0 |
| 系统调用号寄存器 | a7 (x17) | rax | x8 | a7 (r11) |
| 返回值寄存器 | a0 (x10) | rax | x0 | a0 (r4) |
| 栈指针 | sp (x2) | rsp | sp | sp (r3) |
| TLB 刷新 | sfence.vma | invlpg | tlbi | invtlb |
| 页表基址寄存器 | satp | CR3 | TTBR0_EL1 | CSR 页表基 |
| 特权级切换 | sret | iretq / sysret | eret | ertn |
| 定时器 | rdtime + SBI | RDTSC + LAPIC | CNTPCT_EL0 | 稳定计数器 |
| 中断控制器 | PLIC | APIC | GIC | 内置 |
