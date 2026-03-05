# 多架构移植：Trap 处理如何屏蔽架构差异

**日期**: 2026/3/6
**父文档**: [多架构移植分析_OSKernel2025-rustoswhu.md](多架构移植分析_OSKernel2025-rustoswhu.md)

---

## 一、问题域：为什么 Trap 是最难抽象的部分

Trap（陷入）是 CPU 从当前执行流切换到内核处理程序的机制，包括系统调用、异常（页错误、非法指令）和中断（时钟、外部设备）。不同架构在以下三个维度上完全不同：

1. **上下文保存格式**：各架构的通用寄存器数量、名字、用途不同
2. **异常分类方式**：读取异常原因的寄存器和编码完全不同
3. **入口/出口汇编**：保存恢复上下文的指令序列架构相关

rustoswhu 的 Trap 抽象方案可以概括为三层：**汇编入口** → **异常分类** → **统一回调**。

---

## 二、抽象层设计

### 2.1 统一枚举定义

在 `arch/src/lib.rs` 中定义两个关键枚举，这是整个抽象的核心接口：

```rust
// 统一的寄存器语义名——内核不需要知道 "a7 是 RISC-V 的系统调用号寄存器"
pub enum TrapFrameArgs {
    SEPC,       // 异常发生时的 PC
    RA,         // 返回地址
    SP,         // 栈指针
    RET,        // 返回值寄存器
    ARG0,       // 系统调用第 1 个参数
    ARG1,       // 系统调用第 2 个参数
    ARG2,       // 系统调用第 3 个参数
    TLS,        // 线程本地存储指针
    SYSCALL,    // 系统调用号
}

// 统一的异常类型——内核不需要知道 "scause=8 是 RISC-V 的 UserEnvCall"
pub enum TrapType {
    Breakpoint,
    UserEnvCall,
    Time,
    Unknown,
    SupervisorExternal,
    StorePageFault(usize),      // 参数是错误地址
    LoadPageFault(usize),
    InstructionPageFault(usize),
    IllegalInstruction(usize),
}
```

### 2.2 三层处理流程

```
┌──────────────────────────────────────────────────────────┐
│ 第一层：汇编入口（每架构独立实现）                          │
│                                                          │
│ RISC-V: kernelvec/uservec  (inline asm in interrupt.rs)  │
│ x86-64: trap.S + syscall_entry                           │
│ AArch64: trap.S 异常向量表                                │
│ LoongArch: trap_vector_base  (inline asm in trap.rs)     │
│                                                          │
│ 职责：保存所有寄存器到 TrapFrame，切换到内核栈              │
├──────────────────────────────────────────────────────────┤
│ 第二层：异常分类（每架构的 kernel_callback / trap_handler）│
│                                                          │
│ 读取架构特定 CSR/寄存器，转换为 TrapType enum              │
│ RISC-V: scause + stval → TrapType                        │
│ x86-64: vector + CR2 → TrapType                          │
│ AArch64: ESR_EL1 + FAR_EL1 → TrapType                   │
│ LoongArch: ESTAT + BADV → TrapType                       │
├──────────────────────────────────────────────────────────┤
│ 第三层：统一回调（os/src/main.rs，架构无关）               │
│                                                          │
│ ArchInterface::kernel_interrupt(ctx, trap_type)           │
│ 通过 ctx[TrapFrameArgs::SYSCALL] 读系统调用号              │
│ 通过 ctx[TrapFrameArgs::RET] = result 写返回值            │
└──────────────────────────────────────────────────────────┘
```

---

## 三、TrapFrame 结构体——各架构实现对比

### 3.1 RISC-V 64

```rust
// arch/src/riscv64/context.rs
pub struct TrapFrame {
    pub x: [usize; 32],      // 32 个通用寄存器
    pub sstatus: Sstatus,     // 特权状态寄存器
    pub sepc: usize,          // 异常 PC
    pub fsx: [usize; 2],     // 浮点状态（fs0, fs1）
}
// 总大小: 32*8 + 8 + 8 + 2*8 = 288 bytes
```

**寄存器映射**：

| TrapFrameArgs | RISC-V 寄存器 | 字段访问 |
|---------------|-------------|---------|
| SEPC | sepc (CSR) | `self.sepc` |
| RA | x1 (ra) | `self.x[1]` |
| SP | x2 (sp) | `self.x[2]` |
| RET | x10 (a0) | `self.x[10]` |
| ARG0 | x10 (a0) | `self.x[10]` |
| ARG1 | x11 (a1) | `self.x[11]` |
| ARG2 | x12 (a2) | `self.x[12]` |
| TLS | x4 (tp) | `self.x[4]` |
| SYSCALL | x17 (a7) | `self.x[17]` |

注意 RET 和 ARG0 指向同一个寄存器——这符合 RISC-V 的调用约定（a0 既是第一个参数也是返回值）。

### 3.2 AArch64

```rust
// arch/src/aarch64/context.rs
pub struct TrapFrame {
    pub regs: [usize; 31],   // x0-x30
    pub sp: usize,           // 栈指针（独立于通用寄存器）
    pub elr: usize,          // Exception Link Register
    pub spsr: usize,         // Saved Program Status Register
    pub tpidr: usize,        // Thread Pointer ID Register
}
// 总大小: 31*8 + 4*8 = 280 bytes
```

**寄存器映射**：

| TrapFrameArgs | AArch64 寄存器 | 字段访问 |
|---------------|--------------|---------|
| SEPC | ELR_EL1 | `self.elr` |
| RA | x30 (LR) | `self.regs[30]` |
| SP | SP_EL0 | `self.sp` |
| RET | x0 | `self.regs[0]` |
| ARG0 | x0 | `self.regs[0]` |
| ARG1 | x1 | `self.regs[1]` |
| ARG2 | x2 | `self.regs[2]` |
| TLS | TPIDR_EL0 | `self.tpidr` |
| SYSCALL | x8 | `self.regs[8]` |

关键差异：AArch64 的 SP 不是通用寄存器之一（只有 x0-x30），需要单独字段。

### 3.3 x86-64

```rust
// arch/src/x86_64/context.rs
#[repr(C, align(16))]
pub struct TrapFrame {
    pub rax: usize, pub rcx: usize, pub rdx: usize, pub rbx: usize,
    pub rbp: usize, pub rsi: usize, pub rdi: usize,
    pub r8: usize, pub r9: usize, pub r10: usize, pub r11: usize,
    pub r12: usize, pub r13: usize, pub r14: usize, pub r15: usize,
    pub fs_base: usize,    // TLS (IA32_FS_BASE MSR)
    pub gs_base: usize,    // GS base
    pub vector: usize,     // 中断向量号
    pub error_code: usize, // 硬件推送的错误码
    pub rip: usize,        // 指令指针
    pub cs: usize, pub rflags: usize, pub rsp: usize, pub ss: usize,
    pub fx_area: FxsaveArea,  // 512 bytes 浮点/SIMD 状态
}
// 总大小: 24*8 + 512 = 704 bytes
```

**寄存器映射**：

| TrapFrameArgs | x86-64 寄存器 | 字段访问 |
|---------------|-------------|---------|
| SEPC | RIP | `self.rip` |
| RA | (无直接对应) | `self.rip` (x86 用栈) |
| SP | RSP | `self.rsp` |
| RET | RAX | `self.rax` |
| ARG0 | RDI | `self.rdi` |
| ARG1 | RSI | `self.rsi` |
| ARG2 | RDX | `self.rdx` |
| TLS | FS_BASE | `self.fs_base` |
| SYSCALL | RAX | `self.rax` |

关键差异：x86-64 没有 link register（RA），返回地址在栈上。TLS 不是寄存器而是 MSR。

### 3.4 LoongArch64

```rust
// arch/src/loongarch64/context.rs
pub struct TrapFrame {
    pub regs: [usize; 32],   // $0-$31
    pub prmd: usize,         // Pre-exception Mode info
    pub era: usize,          // Exception Return Address
}
// 总大小: 32*8 + 2*8 = 272 bytes
```

**寄存器映射**：

| TrapFrameArgs | LoongArch 寄存器 | 字段访问 |
|---------------|---------------|---------|
| SEPC | ERA (CSR) | `self.era` |
| RA | $ra ($1) | `self.regs[1]` |
| SP | $sp ($3) | `self.regs[3]` |
| RET | $a0 ($4) | `self.regs[4]` |
| ARG0 | $a0 ($4) | `self.regs[4]` |
| ARG1 | $a1 ($5) | `self.regs[5]` |
| ARG2 | $a2 ($6) | `self.regs[6]` |
| TLS | $tp ($2) | `self.regs[2]` |
| SYSCALL | $a7 ($11) | `self.regs[11]` |

---

## 四、Index trait 实现模式

这是让内核代码完全架构无关的关键。以 RISC-V 为例：

```rust
impl Index<TrapFrameArgs> for TrapFrame {
    type Output = usize;
    fn index(&self, index: TrapFrameArgs) -> &Self::Output {
        match index {
            TrapFrameArgs::SEPC    => &self.sepc,
            TrapFrameArgs::RA      => &self.x[1],
            TrapFrameArgs::SP      => &self.x[2],
            TrapFrameArgs::RET     => &self.x[10],
            TrapFrameArgs::ARG0    => &self.x[10],
            TrapFrameArgs::ARG1    => &self.x[11],
            TrapFrameArgs::ARG2    => &self.x[12],
            TrapFrameArgs::TLS     => &self.x[4],
            TrapFrameArgs::SYSCALL => &self.x[17],
        }
    }
}

// IndexMut 同理，用于写入
impl IndexMut<TrapFrameArgs> for TrapFrame { ... }
```

每个架构都实现相同的 `Index<TrapFrameArgs>` 和 `IndexMut<TrapFrameArgs>` trait，从而让内核可以写出完全不关心架构的代码：

```rust
// os/src/main.rs - 架构无关的中断处理
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    match trap_type {
        TrapType::UserEnvCall => {
            ctx.syscall_ok();
            let id = ctx[TrapFrameArgs::SYSCALL];
            let args = ctx.args();
            let result = syscall(id, [args[0], args[1], args[2], ...]);
            ctx[TrapFrameArgs::RET] = result as usize;
        }
        TrapType::StorePageFault(addr) | TrapType::LoadPageFault(addr) => {
            // 处理页错误，addr 是错误地址
        }
        TrapType::IllegalInstruction(addr) => {
            // 处理非法指令
        }
        _ => {}
    }
}
```

---

## 五、异常分类——各架构如何将硬件编码映射到 TrapType

### 5.1 RISC-V：scause + stval

```rust
// arch/src/riscv64/interrupt.rs
fn kernel_callback(context: &mut TrapFrame) -> TrapType {
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Exception(Exception::Breakpoint)          => TrapType::Breakpoint,
        Trap::Exception(Exception::UserEnvCall)          => TrapType::UserEnvCall,
        Trap::Interrupt(Interrupt::SupervisorTimer)      => TrapType::Time,
        Trap::Exception(Exception::StorePageFault)       => TrapType::StorePageFault(stval),
        Trap::Exception(Exception::StoreFault)           => TrapType::StorePageFault(stval),
        Trap::Exception(Exception::LoadPageFault)        => TrapType::LoadPageFault(stval),
        Trap::Exception(Exception::InstructionPageFault) => TrapType::InstructionPageFault(stval),
        Trap::Exception(Exception::IllegalInstruction)   => TrapType::IllegalInstruction(stval),
        Trap::Interrupt(Interrupt::SupervisorExternal)   => TrapType::SupervisorExternal,
        _ => panic!("未知中断"),
    }
}
```

### 5.2 x86-64：vector number + CR2

```rust
// arch/src/x86_64/interrupt.rs
fn kernel_callback(context: &mut TrapFrame) -> TrapType {
    match context.vector {
        PAGE_FAULT_VECTOR => {
            let cr2 = x86::controlregs::cr2();  // 页错误地址
            // 通过 error_code 区分读/写/执行
            if error_code & 0x10 != 0 {
                TrapType::InstructionPageFault(cr2)
            } else if error_code & 0x2 != 0 {
                TrapType::StorePageFault(cr2)
            } else {
                TrapType::LoadPageFault(cr2)
            }
        }
        BREAKPOINT_VECTOR  => TrapType::Breakpoint,
        APIC_TIMER_VECTOR  => TrapType::Time,
        GENERAL_PROTECTION_FAULT_VECTOR => panic!("GPF"),
        _ => TrapType::Unknown,
    }
}
```

### 5.3 AArch64：ESR_EL1 + FAR_EL1

```rust
// arch/src/aarch64/trap.rs
fn handle_exception(tf: &mut TrapFrame, kind: TrapKind, source: TrapSource) -> TrapType {
    match ESR_EL1.read(ESR_EL1::EC) {
        EC::Brk64                => TrapType::Breakpoint,
        EC::SVC64                => TrapType::UserEnvCall,
        EC::DataAbortLowerEL     => {
            let far = FAR_EL1.get() as usize;
            // 根据 ISS 字段区分读/写
            TrapType::StorePageFault(far) // 或 LoadPageFault
        }
        EC::InstrAbortLowerEL    => TrapType::InstructionPageFault(FAR_EL1.get()),
        _ => TrapType::Unknown,
    }
}
```

### 5.4 LoongArch64：ESTAT + BADV

```rust
// arch/src/loongarch64/trap.rs
fn loongarch64_trap_handler(tf: &mut TrapFrame) -> TrapType {
    let badv = badv::read();
    match estat.cause() {
        Trap::Exception(Breakpoint)         => TrapType::Breakpoint,
        Trap::Exception(Syscall)            => TrapType::UserEnvCall,
        Trap::Exception(StorePageFault)     => TrapType::StorePageFault(badv),
        Trap::Exception(LoadPageFault)      => TrapType::LoadPageFault(badv),
        Trap::Exception(FetchPageFault)     => TrapType::InstructionPageFault(badv),
        Trap::Exception(InstructionNotExist)=> TrapType::IllegalInstruction(badv),
        Trap::Interrupt(timer_irq_11)       => TrapType::Time,
        _ => TrapType::Unknown,
    }
}
```

### 异常源寄存器对比总结

| 功能 | RISC-V | x86-64 | AArch64 | LoongArch |
|------|--------|--------|---------|-----------|
| 异常原因 | scause CSR | vector (栈上) | ESR_EL1 | ESTAT CSR |
| 错误地址 | stval CSR | CR2 寄存器 | FAR_EL1 | BADV CSR |
| 异常 PC | sepc CSR | RIP (栈上) | ELR_EL1 | ERA CSR |
| 特权状态 | sstatus CSR | CS + RFLAGS | SPSR_EL1 | PRMD CSR |

---

## 六、汇编入口——上下文保存/恢复

### 6.1 RISC-V：`kernelvec` / `uservec` / `user_restore`

RISC-V 使用 `sscratch` CSR 来区分内核态和用户态的 trap：

```asm
# kernelvec 入口（arch/src/riscv64/interrupt.rs）
csrrw   sp, sscratch, sp       # 交换 sp 和 sscratch
bnez    sp, uservec            # 如果原 sscratch != 0，说明从用户态来
csrr    sp, sscratch           # 从内核态来，恢复 sp
addi    sp, sp, -TRAPFRAME_SIZE
SAVE_GENERAL_REGS              # 保存 x1-x31, sstatus, sepc
csrw    sscratch, x0           # 标记为内核态
call    kernel_callback        # 调用 Rust 分类函数
LOAD_GENERAL_REGS              # 恢复寄存器
sret                           # 返回
```

**用户态恢复** (`user_restore`):
1. 先保存内核 callee-saved 寄存器到内核栈
2. 将内核栈指针写入 TrapFrame 的第 0 个位置
3. 将 TrapFrame 地址写入 `sscratch`
4. 从 TrapFrame 恢复所有用户寄存器
5. `sret` 返回用户态

### 6.2 x86-64：IDT + SYSCALL 双路径

x86-64 有两条进入内核的路径：

**路径 A：中断/异常（通过 IDT）**
```asm
# trap.S — 256 个中断向量的处理入口
.Ltrap_handler_N:
    push error_code   # 有些异常自动推送
    push vector_num
    jmp .Ltrap_common

.Ltrap_common:
    push rax, rcx, rdx, rbx, rbp, rsi, rdi, r8-r15
    call kernel_callback
    pop r15-r8, rdi, rsi, rbp, rbx, rdx, rcx, rax
    iretq             # 中断返回
```

**路径 B：系统调用（通过 SYSCALL 指令）**
```asm
# syscall_entry — 快速系统调用路径
swapgs                             # 切换 GS 到内核
mov rsp, gs:[KERNEL_RSP]           # 加载内核栈
# 构建 TrapFrame
# ...保存寄存器...
call kernel_callback
# 恢复寄存器
sysretq                            # 快速返回用户态
```

x86-64 的特殊之处是用 `swapgs` 和 per-CPU 变量来找到内核栈，而不是像 RISC-V 用 `sscratch`。

### 6.3 AArch64：异常向量表

ARM 使用 2KB 对齐的异常向量表，每 4 个异常类型 × 4 个来源 = 16 个入口：

```asm
# trap.S
.balign 2048
el1_vector_table:
    # SP_EL0 当前异常级别
    INVALID_EXCP 0 0    # Synchronous
    INVALID_EXCP 1 0    # IRQ
    INVALID_EXCP 2 0    # FIQ
    INVALID_EXCP 3 0    # SError
    # SP_ELx 当前异常级别
    INVALID_EXCP 0 1
    ...
    # Lower EL AArch64
    USER_TRAP 0 2       # 用户态同步异常 → handle_exception
    USER_TRAP 1 2       # 用户态 IRQ
    ...
```

每个入口保存 x0-x30 + SP + ELR + SPSR + TPIDR，然后跳转到 Rust 处理函数。

### 6.4 LoongArch64：CSR-based trap vector

```asm
# trap.rs 中的内联汇编
.balign 4096
trap_vector_base:
    csrrd $sp, 0x1           # 读 PRMD 检查特权级
    andi  $sp, $sp, 0x3
    bnez  $sp, user_vec      # 非零 → 来自用户态

    # 内核态 trap
    csrrd $sp, KSAVE_USP
    SAVE_REGS                # 保存 $1-$31 + PRMD + ERA
    bl    trap_handler
    LOAD_REGS
    ertn                     # 异常返回
```

LoongArch64 的特点是使用 KSAVE CSR（类似 RISC-V 的 sscratch）来临时保存寄存器。

---

## 七、`syscall_ok()` 方法——推进 PC 的差异

系统调用后需要将 PC 推进到下一条指令，否则会无限循环执行 ecall/syscall：

| 架构 | PC 推进方式 | 原因 |
|------|-----------|------|
| RISC-V | `self.sepc += 4` | ecall 固定 4 字节 |
| AArch64 | `self.elr += 4` | svc 固定 4 字节 |
| LoongArch | `self.era += 4` | syscall 固定 4 字节 |
| x86-64 | 不需要（硬件自动） | SYSCALL 指令执行时 RCX 已保存下一条 PC |

---

## 八、与 rcore-lab 的对比

rcore-lab 当前的 trap 处理（`os/src/trap/mod.rs`）直接耦合了 RISC-V：

```rust
// 当前 rcore-lab 的写法（耦合 RISC-V）
pub fn trap_handler() -> ! {
    let scause = scause::read();     // 直接读 RISC-V CSR
    let stval = stval::read();       // 直接读 RISC-V CSR
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            cx.sepc += 4;            // 直接操作 sepc 字段
            cx.x[10] = syscall(cx.x[17], [cx.x[10], cx.x[11], ...]);
            //         ^直接用寄存器编号
        }
        ...
    }
}
```

**改造后应该长这样**：

```rust
// 改造后的写法（架构无关）
fn kernel_interrupt(ctx: &mut TrapFrame, trap_type: TrapType) {
    match trap_type {
        TrapType::UserEnvCall => {
            ctx.syscall_ok();                        // 各架构自行推进 PC
            let id = ctx[TrapFrameArgs::SYSCALL];    // 通过枚举访问
            let args = ctx.args();                   // 通过方法访问
            ctx[TrapFrameArgs::RET] = syscall(id, args) as usize;
        }
        TrapType::StorePageFault(addr) => { ... }
        ...
    }
}
```

**改造要点**：
1. 将 `TrapContext` 重命名/重构为 `TrapFrame`，加上 `Index<TrapFrameArgs>` 实现
2. 将 `scause`/`stval` 读取逻辑移到 `arch/` 层，暴露 `TrapType`
3. 将 `trap.S` 移到 `arch/src/riscv64/` 下
4. `os/src/trap/mod.rs` 只保留架构无关的 `TrapType` match 逻辑
