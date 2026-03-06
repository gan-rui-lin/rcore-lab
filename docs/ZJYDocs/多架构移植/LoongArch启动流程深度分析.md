# LoongArch64 vs RISC-V64 启动流程深度对比

**日期**: 2026/3/6
**父文档**: [多架构移植分析_OSKernel2025-rustoswhu.md](多架构移植分析_OSKernel2025-rustoswhu.md)

---

## 一、核心结论：两个架构的启动哲学完全不同

RISC-V 的启动核心动作是**手动建页表 → 写 satp 启用 MMU → 跳到虚拟地址**。
LoongArch 的启动核心动作是**配置 DMW 直接映射窗口 → 置 PG 位启用分页 → 直接在高地址运行**。

两者最本质的区别在于：**RISC-V 把虚拟地址翻译完全交给页表，而 LoongArch 提供了一条不经过页表的硬件快速通道（DMW）**。

---

## 二、RISC-V64 启动：传统的 "建页表启 MMU" 模型

### 2.1 启动全流程

```
OpenSBI (M-mode) 跳转到 0x80200000
    │
    ▼
_start (S-mode, 物理地址空间, PC = 0x80200000)
    │
    │ ① 设置栈指针
    │    la   sp, boot_stack
    │    add  sp, sp, STACK_SIZE
    │    or   sp, sp, 0xffff_ffc0_0000_0000    ← 提前加上虚拟偏移
    │
    │ ② 建立初始页表 PAGE_TABLE（编译期 const 初始化）
    │    PAGE_TABLE[2]    = 0x8000_0000 → 0x8000_0000  (恒等映射, 1GB 大页)
    │    PAGE_TABLE[0x100]= 0x0000_0000 → 0xffff_ffc0_0000_0000 (高半)
    │    PAGE_TABLE[0x101]= 0x4000_0000 → 0xffff_ffc0_4000_0000 (高半)
    │    PAGE_TABLE[0x102]= 0x8000_0000 → 0xffff_ffc0_8000_0000 (高半)
    │
    │ ③ 启用 Sv39 分页
    │    satp = (8 << 60) | (PAGE_TABLE_PHYS >> 12)
    │    sfence.vma        ← 刷新 TLB
    │
    │ ④ 此时 MMU 开启，当前 PC 仍是物理地址 0x8020xxxx
    │    但因为 PAGE_TABLE[2] 恒等映射了 0x8000_0000，所以不会 fault
    │
    │ ⑤ 跳转到虚拟地址
    │    la   a2, rust_main
    │    or   a2, a2, 0xffff_ffc0_0000_0000    ← 把链接地址转为高半虚拟地址
    │    jalr a2
    │
    ▼
rust_main (S-mode, 虚拟地址空间, PC = 0xffff_ffc0_8020xxxx)
    │ clear_bss → init_allocator → init_logging → init_interrupt
    │ 解析设备树 → add_memory_region(0x80200000 | VIRT, 0xC0000000 | VIRT)
    │ prepare_drivers
    ▼
ArchInterface::main(hartid)
```

### 2.2 为什么需要恒等映射

启用 MMU 的瞬间，CPU 的下一条取指会经过 MMU 翻译。如果当前 PC=`0x80200100`，MMU 会去页表里查 VA `0x80200100` 对应的 PA。如果没有这个映射，CPU 会立即 page fault——还没来得及跳到高半地址就死了。

所以 RISC-V 必须在初始页表中**同时**建立：
- **恒等映射**: `PAGE_TABLE[2]` → VA `0x8000_0000` = PA `0x8000_0000`（让当前 PC 能继续跑）
- **高半映射**: `PAGE_TABLE[0x102]` → VA `0xffff_ffc0_8000_0000` = PA `0x8000_0000`（最终目标地址）

启用 MMU 后，先在恒等映射下执行几条指令完成跳转，然后跳到高半地址，之后恒等映射就不再需要了。

### 2.3 关键代码

```rust
// arch/src/riscv64/entry.rs — 编译期建立初始页表
pub(crate) static mut PAGE_TABLE: [PTE; 512] = {
    let mut arr = [PTE(0); 512];
    // 恒等映射：index 2 → VA 0x8000_0000..0xC000_0000 → PA 0x8000_0000
    arr[2] = PTE::from_addr(0x8000_0000, PTEFlags::ADVRWX);
    // 高半映射：index 0x100..0x102 → VA 0xffff_ffc0_xxxx_xxxx → PA 0x0000_0000..
    arr[0x100] = PTE::from_addr(0x0000_0000, PTEFlags::ADGVRWX);
    arr[0x101] = PTE::from_addr(0x4000_0000, PTEFlags::ADGVRWX);
    arr[0x102] = PTE::from_addr(0x8000_0000, PTEFlags::ADGVRWX);
    arr
};
```

这里用的是 Sv39 的**一级页表大页映射**（每个 PTE 映射 1GB）。index 2 对应 VA `2 * 1GB = 0x8000_0000`，index 0x100 对应 VA `0x100 * 1GB = 0xffff_ffc0_0000_0000`（高半核地址）。

---

## 三、LoongArch64 启动：DMW 直接映射窗口模型

### 3.1 启动全流程

```
固件跳转到内核入口
    │
    ▼
_start (PLV0, 物理地址空间)
    │
    │ ① 配置直接映射窗口 DMW0 和 DMW1
    │    DMW0 = 0x8000_xxxx_xxxx_xxxx → PA（非缓存, UC）
    │    DMW1 = 0x9000_xxxx_xxxx_xxxx → PA（缓存, CA）
    │
    │ ② 置 CRMD.PG=1 启用分页
    │    同时设 PLV=0, IE=0
    │
    │ ③ 设置栈指针（链接地址已经是 0x9000... 开头）
    │
    │ ④ jirl 跳转到 rust_tmp_main
    │    （此时代码已经在 DMW1 窗口下运行——VA 0x9000... = PA 0x0000...）
    │
    ▼
rust_tmp_main (PLV0, 虚拟地址空间)
    │ clear_bss → console_init → init_logging → init_allocator
    │ set_trap_vector_base → sigtrx::init
    │ add_memory_region(VIRT | 0x9000_0000, VIRT | 0xB000_0000)
    │ prepare_drivers → enable FPU → init_timer
    ▼
ArchInterface::main(0)
```

### 3.2 DMW 是什么——LoongArch 的硬件"作弊码"

DMW（Direct Mapping Window，直接映射窗口）是 LoongArch 独有的硬件机制。它的规则极其简单：

```
如果 VA[63:60] 匹配 DMW 寄存器配置的高 4 位，
则 PA = VA[物理地址位宽-1:0]，绕过页表翻译。
```

具体来说：

**DMW0 配置** (CSR 0x180):
```
ori   $t0, $zero, 0x1      # PLV0 可访问
lu52i.d $t0, $t0, -2048    # 高位设为 0x8000
csrwr $t0, 0x180           # 写入 DMWIN0
```
结果：`VA 0x8000_xxxx_xxxx_xxxx → PA 0x0000_xxxx_xxxx_xxxx`（非缓存模式 UC）

**DMW1 配置** (CSR 0x181):
```
ori   $t0, $zero, 0x11     # MAT=1(CC) | PLV0 可访问
lu52i.d $t0, $t0, -1792    # 高位设为 0x9000
csrwr $t0, 0x181           # 写入 DMWIN1
```
结果：`VA 0x9000_xxxx_xxxx_xxxx → PA 0x0000_xxxx_xxxx_xxxx`（缓存模式 CA/CC）

### 3.3 为什么 LoongArch 不需要建初始页表

有了 DMW，内核只要把自己的链接地址设为 `0x9000_0000_9000_0000`，CPU 取指时看到 VA 高 4 位是 `0x9`，自动走 DMW1 窗口，直接得到 PA `0x0000_0000_9000_0000`，**完全不经过页表**。

所以 LoongArch 启动时：
- 不需要建立初始页表
- 不需要恒等映射
- 不需要"跳转到虚拟地址"的过渡步骤
- 甚至 `PageTable::change()` 切到新页表后，内核代码依然通过 DMW 窗口访问，不受页表影响

这就是为什么 LoongArch 的 `_start` 只有 12 行汇编，而 RISC-V 的 `_start` 需要建页表、写 satp、做跳转。

### 3.4 关键代码逐行分析

```rust
// arch/src/loongarch64/boot.rs
unsafe extern "C" fn _start() -> ! {
    asm!("
        // ---- 第一步：配置 DMW0（非缓存直接映射）----
        ori         $t0, $zero, 0x1     // t0 = 0x0000_0000_0000_0001 (PLV0 可访问)
        lu52i.d     $t0, $t0, -2048     // t0 = 0x8000_0000_0000_0001
                                        // lu52i.d 将立即数 << 52 后 OR 到 t0 高位
                                        // -2048 的补码 = 0x800, 即 0x800 << 52 = 0x8000...
        csrwr       $t0, 0x180          // 写 DMWIN0: VA 0x8xxx → PA (UC 非缓存)

        // ---- 第二步：配置 DMW1（缓存直接映射）----
        ori         $t0, $zero, 0x11    // t0 = 0x11 (MAT=0b01=CC 缓存 | PLV0)
        lu52i.d     $t0, $t0, -1792     // t0 = 0x9000_0000_0000_0011
                                        // -1792 补码 = 0x900, 即 0x900 << 52 = 0x9000...
        csrwr       $t0, 0x181          // 写 DMWIN1: VA 0x9xxx → PA (CA 缓存)

        // ---- 第三步：启用分页模式 ----
        li.w        $t0, 0xb0           // CRMD = 0xb0 = 0b1011_0000
                                        //   PG=1 (bit7), DA=0 (bit6->此处bit5)
                                        //   PLV=00 (bit0-1), IE=0 (bit2)
        csrwr       $t0, 0x0            // 写 CRMD: 启用分页, 特权级 0, 中断关闭

        li.w        $t0, 0x00
        csrwr       $t0, 0x1            // 写 PRMD: PIE=0, PWE=0
        li.w        $t0, 0x00
        csrwr       $t0, 0x2            // 写 EUEN: 关闭 FPU 等扩展

        // ---- 第四步：设置栈并跳转到 Rust ----
        la.global   $sp, {boot_stack}
        li.d        $t0, {boot_stack_size}
        add.d       $sp, $sp, $t0       // SP = boot_stack + STACK_SIZE
        csrrd       $a0, 0x20           // a0 = CPU ID
        la.global   $t0, {entry}        // 加载 rust_tmp_main 地址
        jirl        $zero, $t0, 0       // 跳转（不用 bl 因为地址可能超出直接跳转范围）
    ")
}
```

**为什么用 `jirl` 而不是 `bl`？** 注释里写了：`We can't use bl to jump to higher address`。`bl` 是 PC 相对跳转，范围有限（±128MB）。内核链接地址在 `0x9000_0000_9000_0000`，从当前物理地址跳过去超出 `bl` 的范围，所以必须用寄存器间接跳转 `jirl`。

---

## 四、差异根源：硬件设计哲学

### 4.1 RISC-V：纯软件页表，硬件只做翻译

RISC-V 的 MMU 设计极其简洁：

```
VA → 查 satp 指向的页表 → PA
```

没有任何"旁路"。所有虚拟地址翻译都必须通过页表。这意味着：
- **启动时必须先建页表**才能开 MMU
- **内核自身的访问也走页表**（需要在每个进程页表中保留内核映射）
- **没有硬件提供的直接映射**，所以用 `VIRT_ADDR_START | phys_addr` 来构造虚拟地址

RISC-V 的 `VIRT_ADDR_START = 0xffff_ffc0_0000_0000` 是通过页表映射实现的——初始页表的 `arr[0x100..0x102]` 把这段虚拟地址映射到物理 0x0。

### 4.2 LoongArch：DMW 旁路 + 软件 TLB

LoongArch 的 MMU 有两条翻译路径：

```
VA → 检查 DMW0/DMW1 是否匹配 → 匹配则直接 PA（旁路）
     │
     └→ 不匹配 → 查 TLB → miss → 硬件 lddir/ldpte 走页表 → 填 TLB → PA
```

DMW 是**硬件级别的直接映射窗口**，优先级高于 TLB/页表。内核代码和数据都在 DMW 窗口内，所以：
- **启动时不需要页表**（DMW 负责翻译）
- **内核访问不经过页表**（走 DMW 旁路，速度更快）
- **只有用户态地址才走页表**（高 4 位不匹配 DMW）
- **TLB miss 由硬件自动处理**（lddir/ldpte 指令直接遍历页表并填充 TLB）

LoongArch 的 `VIRT_ADDR_START = 0x9000_0000_0000_0000` 不是页表映射的结果，而是 **DMW1 窗口的硬件规则**。

### 4.3 带来的连锁差异

| 维度 | RISC-V | LoongArch | 根本原因 |
|------|--------|-----------|---------|
| 初始页表 | 必须建立 | 不需要 | DMW 旁路 vs 纯页表 |
| 恒等映射 | 必须有（启 MMU 过渡用） | 不需要 | DMW 天然恒等映射 |
| 内核地址访问路径 | 走页表 | 走 DMW（旁路） | 硬件设计不同 |
| `VIRT_ADDR_START` | 页表映射实现 | DMW 硬件规则 | 软件 vs 硬件 |
| TLB miss 处理 | 硬件自动遍历页表（PTW） | **也是硬件**（lddir/ldpte） | 两者都有硬件 PTW |
| TLB refill 入口 | 统一 trap 处理 | **专用 tlbrentry CSR** | LoongArch 有独立快速路径 |
| 启动代码量 | ~60 行汇编 + 页表初始化 | ~12 行汇编 | DMW 省去了大量工作 |
| 启动后切页表 | 内核也受影响 | 只影响用户地址 | 内核走 DMW 不经过页表 |

---

## 五、TLB 管理的差异

### 5.1 RISC-V：硬件 Page Table Walker

RISC-V 的 MMU 在 TLB miss 时会**自动遍历 satp 指向的页表**（hardware page table walk），找到 PTE 后填入 TLB。软件不需要干预 TLB miss（除非触发 page fault）。

```
TLB miss → 硬件自动走页表 → 找到 PTE → 填入 TLB → 继续执行
                              → 无效 PTE → 触发 page fault → 软件处理
```

### 5.2 LoongArch：硬件 lddir/ldpte + 专用 refill 入口

LoongArch 同样有硬件辅助的页表遍历，但机制不同：

1. TLB miss 时触发 **TLB Refill Exception**（不是普通异常）
2. CPU 跳转到 **tlbrentry CSR** 指定的地址（专用入口，不是通用 trap 向量）
3. 该入口处的代码使用 **lddir/ldpte** 指令让硬件遍历页表并加载 PTE
4. 执行 **tlbfill** 指令将 PTE 写入 TLB
5. 执行 **ertn** 返回原始指令重试

```rust
// arch/src/loongarch64/trap.rs
#[naked]
pub unsafe extern "C" fn tlb_fill() {
    asm!("
        .balign 4096
        csrwr   $t0, LA_CSR_TLBRSAVE   // 保存 t0 到专用 CSR
        csrrd   $t0, LA_CSR_PGD        // 读取页目录基址
        lddir   $t0, $t0, 3            // 硬件遍历第 3 级
        lddir   $t0, $t0, 1            // 硬件遍历第 1 级
        ldpte   $t0, 0                 // 加载偶数页 PTE
        ldpte   $t0, 1                 // 加载奇数页 PTE
        tlbfill                        // 填充 TLB
        csrrd   $t0, LA_CSR_TLBRSAVE   // 恢复 t0
        ertn                           // 返回
    ")
}
```

**`lddir` / `ldpte`** 是 LoongArch 的专用指令：
- `lddir $t0, $t0, level`：从 `$t0` 指向的页目录中，根据 badv（故障地址）自动索引，读出下一级目录地址到 `$t0`
- `ldpte $t0, index`：从最终页表中加载 PTE 到硬件 TLB 寄存器

这些指令让 TLB refill 只需 **8 条指令**，极其高效。RISC-V 的硬件 PTW 是全自动的黑盒，LoongArch 则是半自动——给了软件一定的控制权（比如可以在 refill 时做额外检查），但代价是需要写 refill handler。

### 5.3 TLB 初始化代码

```rust
// arch/src/loongarch64/trap.rs
pub fn tlb_init(tlbrentry: usize) {
    // 设置 TLB 页面大小为 4KB
    tlbidx::set_ps(PS_4K);     // TLB 索引寄存器
    stlbps::set_ps(PS_4K);     // STLB 页面大小
    tlbrehi::set_ps(PS_4K);    // TLB Refill Exception 页面大小

    // 配置页表遍历控制器（PWCL/PWCH）
    pwcl::set_pte_width(8);           // PTE 宽度 8 字节（64-bit）
    pwcl::set_ptbase(12);             // 页表基址从 bit12 开始（4KB 页）
    pwcl::set_ptwidth(9);             // 每级 9 位索引（512 个 PTE）
    pwcl::set_dir1_base(21);          // 第 1 级目录从 bit21 开始
    pwcl::set_dir1_width(9);          // 第 1 级 9 位索引
    pwch::set_dir3_base(30);          // 第 3 级目录从 bit30 开始
    pwch::set_dir3_width(9);          // 第 3 级 9 位索引

    // 注册 TLB refill 入口
    set_tlb_refill(tlbrentry);
}
```

PWCL/PWCH（Page Walk Control）寄存器告诉硬件页表的布局格式，这样 `lddir`/`ldpte` 指令才能正确遍历。RISC-V 不需要这些配置，因为 Sv39 的格式是硬编码在硬件中的。

---

## 六、内存区域通告方式的差异

### RISC-V：设备树 (FDT)

```rust
// arch/src/riscv64/mod.rs（被注释掉了，改用硬编码）
let fdt = unsafe { Fdt::from_ptr(device_tree as *const u8).unwrap() };
fdt.memory().regions().for_each(|x| {
    ArchInterface::add_memory_region(
        x.starting_address as usize | VIRT_ADDR_START,
        (x.starting_address as usize + x.size.unwrap()) | VIRT_ADDR_START,
    );
});
// 实际使用的硬编码版本：
ArchInterface::add_memory_region(0x80200000 | VIRT_ADDR_START, 0xC0000000 | VIRT_ADDR_START);
```

RISC-V 启动时 OpenSBI 会通过 `a1` 寄存器传递设备树地址，内核可以解析设备树获取内存布局。

### LoongArch：硬编码

```rust
// arch/src/loongarch64/mod.rs
ArchInterface::add_memory_region(
    VIRT_ADDR_START | 0x9000_0000,             // 0x9000_0000_9000_0000
    VIRT_ADDR_START | (0x9000_0000 + 0x2000_0000),  // + 512MB
);
```

LoongArch 目前没有解析设备树，直接硬编码内存范围。注意地址是 `VIRT_ADDR_START(0x9000_0000_0000_0000) | 0x9000_0000`——通过 DMW1 窗口，VA `0x9000_0000_9000_0000` 直接映射到 PA `0x9000_0000`。

---

## 七、控制台初始化的差异

### RISC-V：通过 SBI（无需直接操作硬件）

RISC-V 启动时有 OpenSBI 提供控制台服务。内核调用 SBI ecall 即可输出字符，不需要自己初始化 UART 硬件：

```rust
pub fn console_putchar(c: usize) {
    sbi_rt::legacy::console_putchar(c);  // SBI 调用
}
```

### LoongArch：直接操作 UART 硬件

LoongArch 没有类似 SBI 的标准固件接口，必须自己初始化 UART 16550 兼容硬件：

```rust
// arch/src/loongarch64/console.rs
const UART_ADDR: usize = 0x1fe001e0 | VIRT_ADDR_START;  // QEMU
// 或者
const UART_ADDR: usize = 0x800000001fe20000;              // 2K1000 真实硬件

pub fn console_init() {
    COM1.lock().init();  // 配置 MCR、IER 寄存器
}
```

注意 2K1000 板子用的是 DMW0 窗口地址（`0x8000...`，非缓存），因为 MMIO 寄存器必须用非缓存访问。QEMU 用的是 DMW1 窗口地址（`0x9000... | addr`）。

---

## 八、启动顺序差异总结

| 步骤 | RISC-V | LoongArch | 差异原因 |
|------|--------|-----------|---------|
| 1 | 设置栈（需要加 VIRT 偏移） | 配置 DMW0/DMW1 | LA 需要先开 DMW 才能用高地址 |
| 2 | 建立初始页表（编译期 const） | 设置 CRMD（PG=1, PLV=0） | RV 需要页表才能启用 MMU |
| 3 | 写 satp 启用 Sv39 | 设置 PRMD, EUEN | LA 启用分页不需要页表 |
| 4 | sfence.vma 刷新 TLB | 设置栈（链接地址已是高地址） | RV 需要手动刷 TLB |
| 5 | 跳转到虚拟地址（jalr） | jirl 跳转到 Rust 入口 | RV 需要从物理跳到虚拟 |
| 6 | clear_bss | clear_bss | 相同 |
| 7 | init_allocator | console_init（先初始化控制台） | LA 没有 SBI，需要手动 |
| 8 | init_logging | init_logging | 相同 |
| 9 | init_interrupt | init_allocator | 顺序不同 |
| 10 | 解析设备树/硬编码内存 | set_trap_vector_base + TLB init | LA 需要配置 TLB 硬件 |
| 11 | prepare_drivers | 硬编码内存区域 | 相同 |
| 12 | main(hartid) | prepare_drivers → enable FPU → init_timer | LA 还要手动开 FPU |
| 13 | — | main(0) | 汇合 |

### 核心差异归纳

1. **RISC-V 需要建页表，LoongArch 不需要** → DMW 硬件旁路
2. **RISC-V 有 SBI 提供控制台，LoongArch 自己操作 UART** → 固件接口差异
3. **RISC-V 需要恒等映射过渡，LoongArch 直接跑** → MMU 启用方式不同
4. **LoongArch 需要配置 TLB 遍历参数（PWCL/PWCH），RISC-V 不需要** → 软件 TLB 管理 vs 硬件全自动
5. **LoongArch 需要手动开 FPU（EUEN），RISC-V 通过 sstatus.FS 控制** → 协处理器使能机制不同
