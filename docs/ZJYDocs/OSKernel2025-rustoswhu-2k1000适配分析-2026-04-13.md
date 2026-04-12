# OSKernel2025-rustoswhu 龙芯 2K1000 Vision5 开发板适配分析

日期：2026/04/13

---

## 一、背景与项目概述

**OSKernel2025-rustoswhu**（RustOsWhu）是武汉大学参赛队伍在全国大学生操作系统比赛（OSKernel2025）中开发的 Rust 宏内核操作系统。该内核同时支持 RISC-V（riscv64gc）和 LoongArch（loongarch64）两种指令集架构，并且最终完成了在真实物理硬件——**龙芯 2K1000 Vision5 开发板**上的上板运行与测试。

### 1.1 龙芯 2K1000 硬件简介

龙芯 2K1000（LS2K1000）是基于 **LoongArch64** 指令集的片上系统（SoC），主要特性如下：

- **CPU**：双核龙芯 2K1000，主频 1GHz，实现了 LoongArch 规范 v1.0
- **内存**：板载 DDR3，通常为 1GB
- **存储**：SATA 接口的机械/固态硬盘（无 SD 卡槽）
- **串口**：16550 兼容 UART，物理地址位于 `0x1fe20000`（注意与 QEMU 模拟不同）
- **PCI**：PCI 控制器基地址位于 `0xFE0000000`（配置空间）
- **对齐限制**：硬件 **不支持非对齐（Unaligned）内存访问**，访问未对齐地址会触发 `AddressNotAligned` 异常

这些硬件特性与 QEMU 模拟的 `virt` 机器有多处不同，因此从 QEMU 移植到真实板子需要针对性适配。

### 1.2 项目整体架构

RustOsWhu 采用 **HAL（Hardware Abstraction Layer）分层设计**，将平台/板级相关代码完全隔离在 `arch/` 目录下：

```
arch/src/
├── riscv64/      # RISC-V 平台实现
├── loongarch64/  # LoongArch 平台实现（本文重点）
│   ├── boot.rs       # 早期启动代码（DMW 初始化、PG 使能）
│   ├── console.rs    # UART 串口驱动（支持 qemu/2k1000 选择）
│   ├── consts.rs     # 架构常量（VIRT_ADDR_START 等）
│   ├── page_table.rs # 页表 PTE 定义与操作
│   ├── trap.rs       # 陷入向量、用户态切换
│   ├── unaligned.rs  # 非对齐访问软件模拟
│   └── unaligned.S   # 非对齐读写汇编实现
└── x86_64/
```

编译时通过 Cargo feature 的 `cfg` 机制区分目标板：

```toml
# os/dotcargo/config.toml
[target.loongarch64-unknown-none]
rustflags = [
    "-Clink-arg=-Tos/src/linker-loongarch64.ld",
    "-Cforce-frame-pointers=yes",
    "-Ctarget-feature=-ual",
    '--cfg=board="2k1000"',   # 切换为 2k1000 时使用此行
    # '--cfg=board="qemu"',   # 默认 QEMU 使用此行
]
```

`cfg_if!` 宏在代码中广泛用于根据 `board` cfg 选择不同实现路径，使得同一套源码可以编译出面向 QEMU 和 2K1000 两种目标的内核。

---

## 二、适配思路与总体架构

从 QEMU LoongArch 到真实 2K1000 开发板，适配工作主要覆盖以下几个层次：

```
┌─────────────────────────────────────────────┐
│               用户态程序 / 测试用例              │
├─────────────────────────────────────────────┤
│            系统调用层 / 内核主体               │  基本不变
├─────────────────────────────────────────────┤
│  串口驱动  │  页表PTE  │  异常处理  │  定时器  │  ← 主要适配点
├─────────────────────────────────────────────┤
│   SATA/AHCI 块设备驱动（替换 VirtIO-blk）     │  ← 驱动替换
├─────────────────────────────────────────────┤
│      DMW 映射窗口 / 非对齐访问处理             │  ← 底层硬件差异
└─────────────────────────────────────────────┘
```

每一层的适配难点和解决方案将在下文逐一分析。

---

## 三、核心适配点详解

### 3.1 DMW 直接映射窗口与 MMIO 访问问题

**背景知识：LoongArch 的 DMW 机制**

LoongArch 架构提供了 4 个"直接映射窗口"（Direct Mapping Window，DMW），通过 CSR 寄存器 `DMWIN0`（`0x180`）~`DMWIN3`（`0x183`）配置。DMW 使得某些虚拟地址区间可以无需经过 TLB 查询直接翻译到物理地址，通常用于内核代码/数据的快速访问和 MMIO 映射。

典型配置：
- **DMW0**（`0x180`）：`UC`（Uncached），前缀 `0x8000`，用于 MMIO 设备寄存器访问
- **DMW1**（`0x181`）：`CA`（Cached），前缀 `0x9000`，用于内核数据和代码

在 `_start` 汇编入口中，RustOsWhu 进行如下 DMW 初始化（`arch/src/loongarch64/boot.rs`）：

```asm
ori   $t0, $zero, 0x1      # CSR_DMW1_PLV0 (PLV0 可用)
lu52i.d $t0, $t0, -2048    # UC, PLV0, 前缀 0x8000_xxxx_xxxx_xxxx
csrwr $t0, 0x180           # 写入 DMWIN0 (Uncached)

ori   $t0, $zero, 0x11     # CSR_DMW1_MAT | CSR_DMW1_PLV0 (Cached + PLV0)
lu52i.d $t0, $t0, -1792    # CA, PLV0, 前缀 0x9000_xxxx_xxxx_xxxx
csrwr $t0, 0x181           # 写入 DMWIN1 (Cached)
```

**QEMU 与 2K1000 的差异**

在 QEMU 中，访问 MMIO（如 UART 地址 `0x1fe001e0`）时，无论走 `0x8000` 还是 `0x9000` 前缀，模拟器都能正确处理。但在真实的 2K1000 硬件上，MMIO 设备只能通过 **UC（Uncached）窗口**（`0x8000` 前缀）访问。如果使用 CA 窗口（`0x9000` 前缀）访问 MMIO，硬件会将请求送入 Cache，导致读写结果不确定或设备无响应。

这也是为何内核在 2K1000 上启动后 **串口完全没有输出**——UART 初始化失败了。

**串口地址的差异**

除了 DMW 前缀的选择，2K1000 上 UART 的物理基地址与 QEMU 模拟也不同：

| 环境 | UART 物理地址 |
|------|--------------|
| QEMU LoongArch virt | `0x1fe001e0` |
| 2K1000 Vision5 | `0x1fe20000` |

`arch/src/loongarch64/console.rs` 通过 `cfg_if!` 宏实现条件编译：

```rust
cfg_if::cfg_if! {
    if #[cfg(board = "qemu")] {
        const UART_ADDR: usize = 0x1fe001e0 | VIRT_ADDR_START; // 走 0x9000 CA 窗口
    } else if #[cfg(board = "2k1000")] {
        const UART_ADDR: usize = 0x800000001fe20000; // 走 0x8000 UC 窗口，不同物理地址
    }
}
```

注意 2K1000 版本直接硬编码了完整的虚拟地址（含 `0x8000` 前缀），而非使用 `| VIRT_ADDR_START` 偏移（`VIRT_ADDR_START = 0x9000_0000_0000_0000`），这正是两者在 DMW 窗口选择上的本质区别。

**换行符问题**

还有一个细节：2K1000 的串口终端需要 `\r\n` 作为换行，而不是单独的 `\n`。`os/src/console.rs` 中专门处理了这一点：

```rust
// 2k1000需要 \r\n 当作换行，而不只是 \n
if c == '\n' as u8 {
    console_putchar('\r' as u8);
    console_putchar(c);
} else {
    console_putchar(c);
}
```

---

### 3.2 非对齐内存访问处理

**问题背景**

LoongArch 规范中，CPU 默认**不支持非对齐内存访问**（Unaligned Access）。当程序（包括用户态程序和某些库）尝试以非对齐地址读写 2 字节、4 字节或 8 字节数据时，会触发 `AddressNotAligned`（地址未对齐）异常。

在 QEMU 中，虽然也会触发该异常，但问题相对隐蔽，且很多测试场景可以绕过。而在 2K1000 真实硬件上，该异常触发频率极高，且直接导致程序崩溃，内核启动后会持续报 `AddressNotAligned` 错误。

**两层解决方案**

RustOsWhu 采用了 **编译器选项 + 软件模拟异常处理** 的两层方案：

**第一层：关闭编译器的 UAL 优化**

在 `os/dotcargo/config.toml` 中为 LoongArch64 目标添加：

```toml
"-Ctarget-feature=-ual"
```

`-ual` 是 LoongArch 的 target feature，代表"Unaligned Access"。`-Ctarget-feature=-ual` 的含义是**禁用**该 feature，即告知 LLVM 后端不要生成非对齐的内存访问指令。这可以避免内核自身代码（Rust 编译产物）产生非对齐访问，但无法阻止用户态程序（如 musl libc、busybox）或手写汇编产生此类访问。

**第二层：陷入处理中软件模拟非对齐指令**

`arch/src/loongarch64/trap.rs` 中处理 `AddressNotAligned` 异常：

```rust
Trap::Exception(Exception::AddressNotAligned) => {
    unsafe { emulate_load_store_insn(tf) }
    TrapType::Unknown
}
```

`emulate_load_store_insn` 实现于 `arch/src/loongarch64/unaligned.rs`，其工作原理如下：

1. 从 `era`（Exception Return Address，即触发异常的指令地址）读取出触发异常的机器码
2. 解码操作码（opcode），判断是哪种 load/store 指令（`ld.d`、`ld.w`、`st.d` 等，包括浮点指令 `fld.d`、`fst.s` 等）
3. 从 `badv`（Bad Virtual Address）寄存器获取非对齐的目标地址
4. 调用汇编实现的 `unaligned_read` / `unaligned_write`（`unaligned.S`），以字节为单位逐字节完成读写
5. 将结果写回对应的通用寄存器，并将 `era += 4`，跳过触发异常的指令继续执行

支持的指令类型覆盖了所有常见 load/store 格式，包括：
- 普通整数 load/store：`LDH`、`LDW`、`LDD`、`STH`、`STW`、`STD` 及其无符号变体
- 指针形式：`LDPTR.W`、`LDPTR.D`、`STPTR.W`、`STPTR.D`
- 带索引的 load/store：`LDX.H`、`LDX.W`、`LDX.D` 等
- 浮点 load/store：`FLD.S`、`FLD.D`、`FST.S`、`FST.D` 及带索引变体

这一机制使得即便用户态程序产生了非对齐访问，内核也能透明地完成模拟，程序无感知地继续运行，git commit `e57cf14` 专门为此功能创建。

---

### 3.3 页表 PTE 标志位问题

**背景**

LoongArch 页表的 PTE（Page Table Entry）格式与 RISC-V 类似但有所不同，尤其是对 MMIO 和用户页表的要求，在 2K1000 上比 QEMU 更为严格。

**问题现象**

在 2K1000 上，用户程序开始运行后立刻触发页错误（LoongArch 中报为 `InstructionNotExist`，即"指令不存在"异常）。排查后发现页表映射本身是正确的，物理页帧中也有正确的代码数据，但 CPU 仍然无法取指。

**根本原因**

LoongArch 页表对 PTE 标志位的要求：2K1000 对页表检查比 QEMU 更严格，用户页表必须同时设置以下位：

| 标志位 | 含义 | 值 |
|--------|------|----|
| `V` | Valid（页有效） | bit 0 |
| `P` | Physical（物理页存在） | bit 7 |
| `MAT_NOCACHE` | Memory Access Type = Uncached/Normal | bits [5:4] = `01` |
| `PLV_USER` | Privilege Level = PLV3（用户态） | bits [3:2] = `11` |

在 QEMU 中，仅设置 `V` 位即可让程序运行，但 2K1000 硬件 MMU 会严格检查 `P` 和 `MAT` 位。若 `MAT_NOCACHE` 未设置，CPU 会将访问提交给 Cache，导致取指失败，报出"指令不存在"的迷惑性错误。

**代码修复**

`arch/src/loongarch64/page_table.rs` 中的 `MappingFlags` → `PTEFlags` 转换：

```rust
impl From<MappingFlags> for PTEFlags {
    fn from(value: MappingFlags) -> Self {
        // 2k1000 必须同时设置 V、P、MAT_NOCACHE 三个位
        let mut flags = PTEFlags::V | PTEFlags::P | PTEFlags::MAT_NOCACHE;
        if value.contains(MappingFlags::W) {
            flags |= PTEFlags::W | PTEFlags::D;
        }
        if value.contains(MappingFlags::U) {
            flags |= PTEFlags::PLV_USER; // 用户页必须设置特权级
        }
        if value.contains(MappingFlags::cow) {
            flags |= PTEFlags::cow;
        }
        flags
    }
}
```

注释 `//2k1000` 明确标注了这是针对真实硬件的修复，QEMU 上可以不设置这些位。

---

### 3.4 陷入向量的 4096 字节对齐要求

LoongArch 要求异常向量入口地址按 4096 字节（`0x1000`）对齐，这在 2K1000 上被严格执行。`trap.rs` 中多个关键函数使用 `.balign 4096` 汇编指令确保对齐：

```rust
// trap_vector_base 主陷入向量
pub unsafe extern "C" fn trap_vector_base() {
    core::arch::asm!(
        ".balign 4096   // 2k1000 需要 4096 对齐",
        ...
    );
}

// 用户地址读写探测异常入口
pub unsafe extern "C" fn user_rw_exception_entry() { // 2k1000需要4096对齐
    asm!(".balign 4096", ...);
}

// try_write_user / try_read_user 也需要 4096 对齐
pub unsafe extern "C" fn try_write_user() { // 2k1000需要4096对齐
    asm!(".balign 4096", ...);
}
```

在 QEMU 模拟器中，对齐要求相对宽松，代码可以工作，但在 2K1000 真实 MMU 中，若入口地址未对齐会触发额外错误。

---

### 3.5 块设备驱动：从 VirtIO 到 AHCI/SATA

**架构差异**

| 环境 | 块设备类型 | 接口 |
|------|----------|------|
| QEMU LoongArch | VirtIO Block（PCI） | `virtio-blk-pci` |
| 2K1000 Vision5 | SATA 机械/固态硬盘 | AHCI over PCI |

2K1000 开发板使用 SATA 硬盘而非 VirtIO 虚拟块设备。这要求实现一个完整的 AHCI 驱动，并通过 PCI 总线扫描定位 AHCI 控制器。

**驱动选择逻辑**

`os/src/drivers/block/mod.rs` 通过 `cfg` 条件编译选择驱动：

```rust
#[cfg(all(target_arch = "loongarch64", board = "qemu"))]
pub use pci_virtio_blk::VirtIOBlock;

#[cfg(all(target_arch = "loongarch64", board = "2k1000"))]
pub use sata_block::SataBlock;

// BLOCK_DEVICE 全局块设备句柄，按目标板选择初始化
#[cfg(all(target_arch = "loongarch64", board = "2k1000"))]
lazy_static! {
    pub static ref BLOCK_DEVICE: Arc<dyn BlockDevice> = Arc::new(SataBlock::new());
}
```

**AHCI 驱动实现要点**

`SataBlock`（`os/src/drivers/block/sata_block.rs`）封装了来自 `isomorphic_drivers` crate 的 AHCI 实现。关键初始化流程：

1. **PCI 总线扫描**：以内存映射（MMIO）方式访问 PCI 配置空间，基地址 `PCI_CONFIG_ADDRESS = 0x8000_00FE_0000_0000`（注意用的是 `0x8000` 开头的 UC 窗口）

2. **AHCI 设备识别**：扫描所有 PCI 设备，寻找 `class=0x01`（Mass Storage）、`subclass=0x06`（SATA）、`prog_if=0x01`（AHCI）的设备

3. **BAR 映射**：读取设备的 BAR0 获取 AHCI 控制器的物理地址，加上 `0x8000_0000_0000_0000` 前缀转换为内核可访问的虚拟地址

4. **DMA 内存分配**：AHCI 需要 DMA 内存（命令列表、接收 FIS、命令表）。`Provider` trait 实现通过物理页帧分配器分配连续物理内存，并用 PA | `VIRT_ADDR_START` 映射为内核虚拟地址

```rust
pub fn pci_init() -> Option<AHCI<Provider>> {
    for dev in unsafe { scan_bus(&UnusedPort, CSpaceAccessMethod::MemoryMapped, PCI_CONFIG_ADDRESS) } {
        if dev.id.class == 0x01 && dev.id.subclass == 0x06 && dev.id.prog_if == 0x01 {
            if let Some(BAR::Memory(pa, len, _, _)) = dev.bars[0] {
                unsafe { enable(dev.loc) };
                // PA 加上 0x8000... 前缀走 UC 窗口访问 AHCI 寄存器
                if let Some(x) = AHCI::new((pa | 0x8000_0000_0000_0000) as usize, len as usize) {
                    return Some(x);
                }
            }
        }
    }
    None
}
```

**文件系统烧录**

由于 2K1000 使用 SATA 硬盘，无法像 SD 卡一样直接拔出连到电脑烧录镜像。团队采用了一个颇为复杂的方案：

1. 将制作好的 ext4 文件系统镜像（`.img`）分割成若干小块
2. 通过 TFTP（`tftpd64` 工具）逐块传输到开发板内存
3. 在 U-Boot 命令行中，使用 `scsi write` 命令将各数据块写入 SATA 硬盘的指定偏移位置
4. 完成后重启，内核从 SATA 硬盘挂载文件系统

---

### 3.6 上板流程与调试工具链

**硬件连接**

2K1000 Vision5 需要：
- **USB-TTL 串口线**：连接到主机，用于 U-Boot 交互和内核日志输出
- **网线**：用于通过以太网从主机 TFTP 传输内核/镜像文件
- **专用电源线**：（非 USB Type-C，使用板子自带电源适配器）

**U-Boot 交互**

通过 PuTTY 等串口工具（115200 波特率）连接后，在 U-Boot 中执行：

```
setenv ipaddr 192.168.137.223      # 设置开发板 IP
setenv serverip 192.168.137.1      # 设置主机 IP（TFTP 服务器）
saveenv                             # 保存设置
ping 192.168.137.1                  # 测试网络连通性（Host is Alive 表示成功）
tftpboot 0x90000000 kernel-la      # 通过 TFTP 加载内核到内存
go 0x90000000                       # 跳转执行内核
```

此外，有时需要先输入 `scsi reset` 重置 SATA 控制器状态后再运行内核，否则 AHCI 初始化可能失败。

**内核加载地址**

链接脚本 `linker-loongarch64.ld` 将内核基地址设为：

```
BASE_ADDRESS = 0x9000000090000000;
```

这对应物理地址 `0x90000000`（约 2.25GB 偏移），通过 DMW1（Cached）窗口访问，tftpboot 时需要将内核加载到 `0x90000000`，`go` 命令跳转到该地址后 CPU 通过 DMW1 找到对应代码。

---

## 四、适配总结与经验

| 问题 | 表现 | 根本原因 | 解决方案 |
|------|------|----------|----------|
| 无串口输出 | 内核启动后完全无输出 | UART 地址不同，且需走 0x8000 UC 窗口 | `cfg_if!` 条件编译选择正确地址和 DMW 前缀 |
| 持续 AddressNotAligned | 内核/用户程序频繁崩溃 | 2K1000 不支持非对齐访问 | 编译器 `-ual` 选项 + 软件模拟异常处理 |
| 用户程序无法取指 | 进入用户态即 InstructionNotExist | PTE 缺少 P 位和 MAT_NOCACHE 位 | 修改 PTE flags 转换逻辑 |
| 块设备不可用 | SATA 硬盘无法读写 | 2K1000 无 VirtIO，使用 SATA/AHCI | 实现 AHCI 驱动，PCI 扫描定位控制器 |
| 终端乱码/无换行 | 日志输出行粘连 | 2K1000 串口需 `\r\n` 换行 | 输出每个 `\n` 前先发送 `\r` |
| 陷入向量偏移错误 | 异常处理跳转到错误地址 | 陷入向量需 4096 字节对齐 | 汇编中添加 `.balign 4096` |

**关键经验**

1. **MMIO 访问走 UC 窗口**：真实 LoongArch 硬件上所有设备寄存器访问必须使用 Uncached（0x8000）DMW 窗口，QEMU 的宽松行为会掩盖这一问题。

2. **非对齐问题比想象中更常见**：musl libc、busybox 等编译时默认假设可以非对齐访问，必须在内核层面兜底处理，否则几乎所有用户程序都会崩溃。

3. **页表调试需要逆向思维**：`InstructionNotExist` 异常通常不是"代码不见了"，而可能是 PTE 标志位导致 MMU 取指失败，调试时需要先验证物理内存是否有数据，再排查 PTE。

4. **文件系统烧录是上板的最大障碍**：没有 SD 卡的便捷烧录通道，TFTP 分块传输方案虽然可行但耗时，建议在内核稳定后再进行一次性完整烧录。

5. **多编译目标配置管理**：`dotcargo/config.toml` 中对不同目标板的 rustflags 切换，配合 `cfg_if!` 宏，是保持单一代码库同时支持多平台的关键基础设施，值得在自己的项目中借鉴。
