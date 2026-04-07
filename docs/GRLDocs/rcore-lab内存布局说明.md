# rCore-lab 内存布局说明

**日期**: 2026/04/04

本文档详细描述 rCore-lab 内核的物理地址和虚拟地址内存布局，涵盖 RISC-V 64 和 LoongArch64 两种架构。

---

## 一、物理地址布局

### 1.1 RISC-V 64 (QEMU virt)

| 地址范围 | 大小 | 用途 |
|---------|------|------|
| `0x0010_0000 - 0x0012_0000` | 8KB | QEMU virt test device |
| `0x0200_0000 - 0x0210_0000` | 64KB | CLINT (Core Local Interruptor) |
| `0x0C00_0000 - 0x0C21_0000` | ~2MB | PLIC (Platform-Level Interrupt Controller) |
| `0x1000_0000 - 0x1000_9000` | 36KB | UART (16550) + VirtIO 设备 |
| `0x1000_1000` | - | VirtIO Block 设备基地址 |
| `0x8000_0000 - 0x8020_0000` | ~2MB | 内核代码段/数据段（物理位置） |
| `0x8020_0000 - 0xC000_0000` | ~1GB-2MB | 可用物理内存（帧分配器管理） |

**关键符号**:
- `skernel` / `ekernel`: 内核镜像的起止物理地址
- `MEMORY_END = 0xC000_0000`: 物理内存上限（1GB RAM）

### 1.2 LoongArch64 (QEMU virt)

| 地址范围 | 大小 | 用途 |
|---------|------|------|
| `0x1fe0_01e0` | - | UART MMIO 基地址 |
| `0x9000_0000` | - | RAM 起始地址 |
| `0x9000_0000 - 0xD000_0000` | 1GB | 物理内存范围 |

**关键符号**:
- `MEMORY_END = 0xD000_0000`: 物理内存上限

---

## 二、虚拟地址布局

### 2.1 内核虚拟地址空间

#### RISC-V 64

| 虚拟地址 | 物理地址 | 说明 |
|---------|---------|------|
| `0xFFFF_FFC0_8020_0000` | `0x8020_0000` | 内核 `.text` 段起始（BASE_ADDRESS） |
| `VIRT_ADDR_START = 0xFFFF_FFC0_0000_0000` | - | 高半部虚地址起点 |
| `PA \| VIRT_ADDR_START` | `PA` | 线性映射规则（恒等映射偏移） |

内核链接脚本 `linker.ld`:
```
BASE_ADDRESS = 0xFFFF_FFC0_8020_0000
```

内核段顺序（低地址→高地址）:
1. `.text.entry` - 启动入口
2. `.text.trampoline` - 陷入跳板（4K 对齐）
3. `.text` - 代码段
4. `.rodata` - 只读数据
5. `.data` - 可读写数据
6. `.bss.stack` - 内核栈
7. `.bss` - 零初始化数据
8. `ekernel` - 内核结束，后续为帧分配器管理区域

#### LoongArch64

| 虚拟地址 | 说明 |
|---------|------|
| `0x9000_0000_9000_0000` | 内核 BASE_ADDRESS |
| `VIRT_ADDR_START = 0x9000_0000_0000_0000` | 直接映射窗口起点 |

---

### 2.2 用户虚拟地址空间

以下常量定义于 `os/src/config.rs`：

```rust
pub const USER_STACK_SIZE: usize = 4096 * 128;      // 512 KB
pub const USER_STACK_TOP: usize = 0x8_0000_0000;    // 32 GB
pub const USER_MMAP_TOP: usize = 0x6_0000_0000;     // 24 GB
pub const KERNEL_STACK_SIZE: usize = 4096 * 20;     // 80 KB
pub const KERNEL_HEAP_SIZE: usize = 128 * 1024 * 1024; // 128 MB
```

#### 用户地址空间布局图

```
高地址
┌──────────────────────────────────────────────┐
│          Signal Return Trampoline            │  (架构相关，见下)
├──────────────────────────────────────────────┤
│                                              │
│                  (保留区)                     │
│                                              │
├──────────────────────────────────────────────┤ 0x8_0000_0000 (USER_STACK_TOP)
│              User Stack                      │  512 KB
│              (向下增长)                       │
├──────────────────────────────────────────────┤ 0x7_FFFC_0000 (USER_STACK_BOTTOM)
│                                              │
│                 (空闲区)                      │
│                                              │
├──────────────────────────────────────────────┤ 0x6_0000_0000 (USER_MMAP_TOP)
│              mmap 区域                        │
│              (向下/向上分配)                   │
│                                              │
├──────────────────────────────────────────────┤ 
│                 (空闲区)                      │
│                                              │
├──────────────────────────────────────────────┤ heap_bottom + brk增量
│              Heap 堆区                        │
│              (sbrk/brk 管理)                  │
├──────────────────────────────────────────────┤ heap_bottom (动态，紧随 ELF 段)
│              BSS 段                          │
├──────────────────────────────────────────────┤
│              Data 段                         │
├──────────────────────────────────────────────┤
│              RoData 段                       │
├──────────────────────────────────────────────┤
│              Text 段 (代码)                   │
├──────────────────────────────────────────────┤ ELF 加载基址 (load_base)
│                                              │
低地址
```

---

### 2.3 关键地址详解

#### 2.3.1 ELF 加载基址 (load_base)

| ELF 类型 | load_base | 说明 |
|---------|-----------|------|
| 静态链接可执行文件 (ET_EXEC) | `0x0` | 按 ELF 中指定的虚地址加载 |
| PIE 可执行文件 (ET_DYN, 无 interp) | `0x4000_0000` | 1GB 偏移 |
| 动态链接可执行文件 (ET_DYN, 有 interp, min_vaddr=0) | `0x4000_0000` | 1GB 偏移 |

代码位置: `os/src/mm/memory_set.rs:706-770`

#### 2.3.2 解释器加载基址 (interp_base)

```rust
let mut interp_base = max_end_va.into();  // 紧随主 ELF 段之后
interp_base = align_up(interp_base, PAGE_SIZE);
```

解释器（如 `ld-linux.so`）加载在主 ELF 所有 LOAD 段结束地址之后，页对齐。

#### 2.3.3 堆区 (Heap)

| 字段 | 说明 |
|------|------|
| `heap_bottom` | 堆起始地址，等于 ELF 所有 LOAD 段结束后的 `max_end_va` |
| `program_brk` | 当前堆顶指针，初始等于 `heap_bottom` |
| 增长方向 | 向高地址增长 |

**不同进程的 heap_bottom 不同**：由其 ELF 加载后的 `max_end_vpn` 决定。

示例（静态链接小程序）:
```
heap_bottom ≈ 0x0040_0000  (ELF 结束于 4MB 附近)
```

示例（动态链接程序 + 解释器）:
```
heap_bottom ≈ 0x4010_0000  (PIE 基址 1GB + ELF/interp 大小)
```

#### 2.3.4 用户栈 (User Stack)

| 参数 | 值 | 说明 |
|------|-----|------|
| `USER_STACK_TOP` | `0x8_0000_0000` (32GB) | 栈顶（高地址） |
| `USER_STACK_SIZE` | `512 KB` | 固定大小 |
| `USER_STACK_BOTTOM` | `0x7_FFFC_0000` | 栈底（低地址） |

栈区标记为 `MapAreaType::Stack`，受 `unmap_range` 保护。

#### 2.3.5 mmap 分配区

| 参数 | 值 | 说明 |
|------|-----|------|
| `USER_MMAP_TOP` | `0x6_0000_0000` (24GB) | mmap 分配起点 |
| `mmap_base` | 进程初始为 `USER_MMAP_TOP` | 当前 mmap 分配水位 |
| 分配策略 | 向高地址递增 | 每次 `mmap_base += len` |

非 `MAP_FIXED` 时，mmap 从 `mmap_base` 开始分配并更新水位。

#### 2.3.6 内核栈 (Kernel Stack)

| 参数 | 值 |
|------|-----|
| `KERNEL_STACK_SIZE` | 80 KB (4096 × 20) |
| 分配方式 | 堆分配 (`Vec<u128>`)，非虚拟地址映射 |
| Guard | 底部 16 个 u128 slot 填充魔数检测溢出 |

每个任务（线程）有独立的内核栈，通过 `kstack_alloc()` 分配。

#### 2.3.7 Signal Return Trampoline

| 架构 | 虚拟地址 | 说明 |
|------|---------|------|
| RISC-V 64 | `0xFFFF_FFC1_0000_0000` | 内核高半部空间 |
| LoongArch64 | `0x40_0000_0000` (256GB) | 用户空间高地址 |

信号返回跳板页映射一页（4KB），包含 `sigreturn` 系统调用指令序列。

---

## 三、内存区域类型 (MapAreaType)

引入于 commit `30d4749`，用于显式标识内存区域类型：

```rust
pub enum MapAreaType {
    Kernel,      // 内核恒等映射
    ElfSegment,  // ELF 程序段（代码/数据/BSS）
    Heap,        // 堆区（sbrk/brk 管理）
    Stack,       // 用户栈
    MmapAnon,    // 匿名 mmap
    MmapFile,    // 文件映射 mmap
    Shm,         // SysV 共享内存
    Other,       // 其他/未知
}
```

**保护机制**: `unmap_range()` 跳过 `Heap`、`Stack`、`ElfSegment` 类型区域，防止 `mmap(MAP_FIXED)` 或 `munmap` 意外破坏关键内存。

---

## 四、地址空间示意图 (综合)

### RISC-V 64 完整布局

```
用户虚拟地址空间                           内核虚拟地址空间
==================                        ==================

0xFFFF_FFC1_0000_0000 ─────────────────── Signal Return Trampoline (RV)
         │
0xFFFF_FFC0_C000_0000 ─────────────────── MEMORY_END 映射
         │
0xFFFF_FFC0_8020_0000 ─────────────────── 内核 .text 起始
         │
0xFFFF_FFC0_0000_0000 ─────────────────── VIRT_ADDR_START (高半部起点)
         │
         │  ... (中间地址空间未使用) ...
         │
0x8_0000_0000 ────────────────────────── USER_STACK_TOP
         │                                 │
         │    User Stack (512KB)           │
         │                                 │
0x7_FFFC_0000 ────────────────────────── USER_STACK_BOTTOM
         │
         │    (空闲)
         │
0x6_0000_0000 ────────────────────────── USER_MMAP_TOP (mmap 起始分配点)
         │
         │    mmap 区域 (向高地址增长)
         │
         │    (空闲)
         │
program_brk ──────────────────────────── 当前堆顶
         │
         │    Heap (sbrk 管理)
         │
heap_bottom ──────────────────────────── 堆底 (紧随 ELF 段)
         │
         │    ELF BSS/Data/RoData/Text
         │
load_base ────────────────────────────── ELF 加载基址 (0 或 0x4000_0000)
         │
0x0
```

---

## 五、关键源码位置

| 功能 | 文件 | 行号/函数 |
|------|------|----------|
| 地址常量定义 | `os/src/config.rs` | 全文件 |
| 物理内存上限 | `arch/src/riscv64/board.rs` | `MEMORY_END` |
| 内核链接脚本 | `os/src/linker.ld` | `BASE_ADDRESS` |
| ELF 加载 | `os/src/mm/memory_set.rs` | `from_elf()`, `from_elf_with_interp()` |
| 用户栈/堆映射 | `os/src/mm/memory_set.rs` | `map_user_stack_and_trap()` |
| 进程内存字段 | `os/src/task/process.rs` | `heap_bottom`, `program_brk`, `mmap_base` |
| 内核栈分配 | `os/src/task/id.rs` | `kstack_alloc()`, `KernelStack` |
| mmap 分配 | `os/src/syscall/process.rs` | `sys_mmap()` |
| sbrk 实现 | `os/src/syscall/process.rs` | `sys_sbrk()` |

---

## 六、常见调试信息解读

1. **`heap_bottom = 0x120200000`**: 进程堆起始于 ~4.5GB（PIE + 较大 ELF）
2. **`mmap_base = 0x600001000`**: mmap 已分配一页，水位上移
3. **`[unmap_range] skipping protected Heap area`**: 保护机制生效，munmap 跳过堆区

---

*最后更新: 2026/04/04*
