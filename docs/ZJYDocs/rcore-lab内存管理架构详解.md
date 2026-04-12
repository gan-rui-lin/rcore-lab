# rcore-lab 内存管理架构详解

**日期**: 2026/04/12

---

## 目录

1. [总体架构概述](#1-总体架构概述)
2. [双架构支持: RISC-V 64 与 LoongArch64](#2-双架构支持)
3. [物理地址空间布局](#3-物理地址空间布局)
4. [内核虚拟地址空间布局](#4-内核虚拟地址空间布局)
5. [用户虚拟地址空间布局](#5-用户虚拟地址空间布局)
6. [四大用户地址区域详解](#6-四大用户地址区域详解)
7. [页表与权限保护机制](#7-页表与权限保护机制)
8. [COW (Copy-on-Write) 策略](#8-cow-copy-on-write-策略)
9. [Demand Paging (按需分页) 策略](#9-demand-paging-按需分页-策略)
10. [内核内存分配器](#10-内核内存分配器)
11. [与 Chronix 的对比与改进 TODO](#11-与-chronix-的对比与改进-todo)

---

## 1. 总体架构概述

rcore-lab 的内存管理子系统位于 `os/src/mm/`，核心由以下组件构成：

| 文件 | 职责 |
|---|---|
| `address.rs` | `PhysAddr`/`VirtAddr`/`PhysPageNum`/`VirtPageNum` 地址类型与算术 |
| `page_table.rs` | Sv39 三级页表遍历、映射、翻译 |
| `frame_allocator.rs` | `StackFrameAllocator` 物理页帧分配器 (栈+回收池) |
| `heap_allocator.rs` | `TracedLockedHeap` 内核堆分配器 (buddy system, 128MB) |
| `memory_set.rs` | `MemorySet`/`MapArea` 虚拟内存区域管理、COW、demand paging |

整体设计遵循 rCore-Tutorial 的经典两层模型：
- **底层**: 物理页帧分配 (`FrameTracker` 的 RAII 管理) + 页表硬件抽象
- **上层**: `MemorySet` 管理一个完整的地址空间，包含若干 `MapArea`(VMA)

---

## 2. 双架构支持

rcore-lab 同时支持 **RISC-V 64** 和 **LoongArch64** 两个架构，架构相关代码位于 `arch/src/{riscv64,loongarch64}/`。

### RISC-V 64 (主力架构)

- **页表模式**: Sv39 (3级页表, 39位虚拟地址)
- **虚拟地址位宽**: 39 bits → 512 GiB 虚拟地址空间
- **物理地址位宽**: 56 bits
- **高半映射基址**: `VIRT_ADDR_START = 0xFFFF_FFC0_0000_0000`
- **内核加载地址**: `BASE_ADDRESS = 0xFFFF_FFC0_8020_0000`
- **satp 模式字**: `8 << 60 | root_ppn`

### LoongArch64

- **内核加载地址**: `BASE_ADDRESS = 0x9000_0000_9000_0000`
- **RAM 起始**: `0x9000_0000`
- **RAM 大小**: 1 GiB
- **MEMORY_END**: `0xD000_0000`

两种架构共享相同的用户态虚拟地址布局常量（见 `os/src/config.rs`），确保跨架构行为一致。

---

## 3. 物理地址空间布局

### 3.1 RISC-V 64 QEMU virt 平台物理地址

```
物理地址空间 (56-bit PA, QEMU virt board)
┌──────────────────────────────────────────────┐
│  0x0000_0000 ─ 0x000F_FFFF  (未使用)         │
├──────────────────────────────────────────────┤
│  0x0010_0000 ─ 0x0011_FFFF  QEMU virt test   │  8 KB MMIO
│                              device           │
├──────────────────────────────────────────────┤
│  0x0200_0000 ─ 0x020F_FFFF  CLINT            │  64 KB MMIO
│                              (Core Local      │  mtimecmp / mtime
│                               Interruptor)    │
├──────────────────────────────────────────────┤
│  0x0C00_0000 ─ 0x0C20_FFFF  PLIC             │  ~2.1 MB MMIO
│                              (Platform-Level  │
│                               Interrupt Ctrl) │
├──────────────────────────────────────────────┤
│  0x1000_0000                 UART (16550)     │  VIRT_UART
│  0x1000_1000                 VirtIO Block     │  VIRTIO_BLK
│  0x1000_0000 ─ 0x1000_8FFF  UART+VirtIO区    │  36 KB MMIO
├──────────────────────────────────────────────┤
│           ... 中间未使用 ...                   │
├──────────────────────────────────────────────┤
│  0x8000_0000                 RAM 起始          │  OpenSBI 在此
│  0x8020_0000                 内核镜像起始       │  skernel/stext
│    .text          stext ─ etext               │  R|X
│    .rodata        srodata ─ erodata           │  R
│    .data          sdata ─ edata               │  R|W
│    .bss           sbss_with_stack ─ ebss      │  R|W (含 boot stack)
│  ekernel                     内核镜像结束       │
├──────────────────────────────────────────────┤
│  ekernel ─ 0xBFFF_FFFF      可用物理页帧       │  帧分配器管理范围
│                              (StackFrameAlloc)│
├──────────────────────────────────────────────┤
│  0xC000_0000                 MEMORY_END       │  物理内存上界
└──────────────────────────────────────────────┘
```

**关键物理地址常量**（定义在 `arch/src/riscv64/board.rs`）：

| 常量 | 值 | 说明 |
|---|---|---|
| `CLOCK_FREQ` | `10_000_000` (10 MHz) | aclint-mtimer 时钟频率 |
| `MEMORY_END` | `0xC000_0000` | 物理内存上界 (从 0x8000_0000 起 1GB) |
| `VIRT_PLIC` | `0x0C00_0000` | PLIC 基址 |
| `VIRT_UART` | `0x1000_0000` | UART 基址 |
| `VIRTIO_BLK` | `0x1000_1000` | VirtIO 块设备基址 |

**MMIO 映射表**（恒等映射到内核地址空间）：
```rust
MMIO = &[
    (0x0010_0000, 0x00_2000),  // QEMU virt test device (8 KB)
    (0x0200_0000, 0x1_0000),   // CLINT (64 KB)
    (0x0C00_0000, 0x21_0000),  // PLIC (~2.1 MB)
    (0x1000_0000, 0x9000),     // UART + VirtIO (36 KB)
];
```

### 3.2 帧分配器范围

物理页帧分配器 (`StackFrameAllocator`) 管理 `[ceil(ekernel PA), floor(MEMORY_END)]` 范围内的所有 4KB 页帧。

- **初始化**（`frame_allocator.rs:107-114`）：
  ```rust
  FRAME_ALLOCATOR.init(
      PhysAddr::from(ekernel as usize).ceil(),  // 第一个可用物理页
      PhysAddr::from(MEMORY_END).floor(),         // 最后一个可用物理页
  );
  ```
- `ekernel` 是链接脚本中内核镜像的结束符号
- 实际可用页帧数 ≈ `(0xC000_0000 - ekernel) / 4096`

### 3.3 内核堆 (BSS 段内)

内核堆并非独立的物理区域，而是**驻留在内核 BSS 段内**的一块静态数组：

```rust
// heap_allocator.rs:274
static mut HEAP_SPACE: [u8; KERNEL_HEAP_SIZE] = [0; KERNEL_HEAP_SIZE];
// KERNEL_HEAP_SIZE = 128 * 1024 * 1024 = 128 MB
```

使用 `buddy_system_allocator::LockedHeap` 管理，封装在 `TracedLockedHeap` 中（带分配追踪），作为 `#[global_allocator]`。

---

## 4. 内核虚拟地址空间布局

### 4.1 RISC-V 64 高半映射

rcore-lab 使用 **高半内核** 模型：内核代码运行在虚拟地址的高半部分。

```
内核虚拟地址空间 (Sv39 高半)
┌────────────────────────────────────────────────────────┐
│  0xFFFF_FFC0_0000_0000   VIRT_ADDR_START               │
│  │  高半线性映射 (1GiB 大页)                            │
│  │  root[0x100] → PA 0x0000_0000 (1 GiB)              │
│  │  root[0x101] → PA 0x4000_0000 (1 GiB)              │
│  │  root[0x102] → PA 0x8000_0000 (1 GiB)  ← 内核所在  │
│  │                                                      │
│  0xFFFF_FFC0_8020_0000   BASE_ADDRESS (skernel)        │
│  │  .text     (R|X)                                    │
│  │  .rodata   (R)                                      │
│  │  .data     (R|W)                                    │
│  │  .bss      (R|W) ← 含 HEAP_SPACE[128MB]            │
│  │  ekernel → MEMORY_END 映射为 (R|W)                  │
│  │  MMIO 恒等映射 (R|W)                                │
├────────────────────────────────────────────────────────┤
│  0xFFFF_FFC1_0000_0000   SIG_RETURN_ADDR               │
│  │  信号返回 trampoline (1 页, R|X|U)                   │
│  │  包含 `li a7, 139; ecall` (sys_rt_sigreturn)        │
└────────────────────────────────────────────────────────┘
```

**启动页表**（`entry.rs:40-49`）使用 1GiB 大页的恒等映射：
- `root[2]` → PA `0x8000_0000`（低半临时恒等映射，启动后弃用）
- `root[0x100..0x102]` → PA `0x0000_0000..0xC000_0000`（高半线性映射）

运行时内核页表 (`new_kernel()`) 对各 section 按细粒度权限重新映射，并通过 `install_riscv_high_linear_root()` 保持与 boot 页表的高半兼容。

### 4.2 地址转换公式

```
虚拟地址 = VIRT_ADDR_START | 物理地址
物理地址 = 虚拟地址 & !VIRT_ADDR_START
```

`rv_strip_high_alias()` 函数（`memory_set.rs:114-120`）实现此转换。

---

## 5. 用户虚拟地址空间布局

所有架构共享以下用户态虚拟地址常量（`os/src/config.rs`）：

```
用户虚拟地址空间 (0 ─ 0x8_0000_0000)
┌────────────────────────────────────────────────────────┐
│  0x0000_0000_0001_0000   典型 ELF 加载基址 (非 PIE)      │
│  │  .text  (R|X|U)                                     │
│  │  .rodata (R|U)                                      │
│  │  .data/.bss (R|W|U)                                 │
│  │                                                      │
│  0x0000_0000_4000_0000   PIE/SharedObject 加载基址       │
│  │  (load_base for ET_DYN without interpreter)          │
├────────────────────────────────────────────────────────┤
│  max_end_vpn (对齐后)     heap_bottom                   │
│  │  ▼ Heap 向上增长 (sbrk/brk)                          │
│  │  MapAreaType::Heap  权限: R|W|U                      │
│  │  program_brk (当前堆顶)                               │
│  │                                                      │
│  │    ... (空闲间隙) ...                                 │
│  │                                                      │
├────────────────────────────────────────────────────────┤
│  0x0000_0006_0000_0000   USER_MMAP_TOP (mmap_base 起点) │
│  │  ▼ mmap 区域 (向上分配, mmap_base 递增)               │
│  │  MapAreaType::MmapAnon / MmapFile / Shm              │
│  │  权限: 由 mmap prot 参数决定                          │
│  │                                                      │
│  │    ... (空闲间隙) ...                                 │
│  │                                                      │
├────────────────────────────────────────────────────────┤
│  0x0000_0007_FFE0_0000   user_stack_bottom               │
│  │  ▲ User Stack (2 MB, 向下增长)                        │
│  │  MapAreaType::Stack  权限: R|W|U                      │
│  0x0000_0008_0000_0000   USER_STACK_TOP                  │
├────────────────────────────────────────────────────────┤
│  (RISC-V only)                                          │
│  0xFFFF_FFC1_0000_0000   SIG_RETURN_ADDR                │
│  │  信号返回 trampoline (1页)  权限: R|X|U               │
└────────────────────────────────────────────────────────┘
```

**关键常量汇总**:

| 常量 | 值 | 大小 | 定义位置 |
|---|---|---|---|
| `USER_STACK_TOP` | `0x8_0000_0000` (32 GiB) | — | `config.rs:9` |
| `USER_STACK_SIZE` | `4096 * 512` | 2 MB | `config.rs:7` |
| `USER_MMAP_TOP` | `0x6_0000_0000` (24 GiB) | — | `config.rs:11` |
| `KERNEL_STACK_SIZE` | `4096 * 16` | 64 KB | `config.rs:15` |
| `KERNEL_HEAP_SIZE` | `128 * 1024 * 1024` | 128 MB | `config.rs:17` |
| `PAGE_SIZE` | `0x1000` | 4 KB | `config.rs:20` |
| PIE `load_base` | `0x4000_0000` | — | `memory_set.rs:807,868` |
| `SIG_RETURN_ADDR` | `0xFFFF_FFC1_0000_0000` | 1 page | `arch/riscv64/mod.rs:100` |

---

## 6. 四大用户地址区域详解

### 6.1 Heap (堆区)

- **起点**: `heap_bottom` = ELF 最高段结束地址向上页对齐
- **当前顶部**: `program_brk`（初始 = `heap_bottom`，通过 `sys_sbrk` 增长）
- **增长方向**: 向高地址增长
- **权限**: `R|W|U`
- **分配方式**: **即时分配**（非 lazy），`sys_sbrk` 调用 `append_heap_to` → `map_one` 立即分配物理页并映射
- **MapAreaType**: `Heap`

```
heap_bottom ────────── ELF 最后一段结束 (page-aligned)
    │  已分配区域 (有物理页支撑)
program_brk ─────── 当前堆顶
    │  (未分配, 访问将 SIGSEGV)
```

**注意**: Heap 区的 `sbrk` 是 eager allocation，每调用一次就为新页立即分配物理帧。这与 mmap 的 lazy 策略不同。

### 6.2 Mmap (内存映射区)

- **起点**: `mmap_base` 初始化为 `USER_MMAP_TOP = 0x6_0000_0000`
- **分配方向**: 向上分配，`mmap_base` 递增（与 Linux 默认向下增长不同）
- **权限**: 由 `mmap()` 的 `prot` 参数 + `U` flag 决定
- **分配方式**: **延迟分配**（lazy / demand paging）
  - `MAP_PRIVATE | MAP_ANON`: `insert_lazy_anon_area()` — 无物理页，fault 时零页填充
  - `MAP_PRIVATE | file`: `insert_lazy_file_area()` — fault 时从文件读取内容
  - `MAP_SHARED | MAP_ANON`: 即时分配（因为共享映射需要所有进程看到同一物理帧）
- **MapAreaType**: `MmapAnon` / `MmapFile` / `Shm`

```
USER_MMAP_TOP = 0x6_0000_0000 ─── mmap 起点
    │  mmap region 1 (lazy anon)
    │  mmap region 2 (lazy file-backed)
    │  mmap region 3 (shared)
    │  ...
mmap_base (递增) ────── 下一次 mmap 的起点
```

**MmapMeta 元数据**:
```rust
pub struct MmapMeta {
    pub shared: bool,         // MAP_SHARED
    pub file_backed: bool,    // 文件支撑 (非匿名)
    pub file_writable: bool,  // fd 是否可写
}
```

### 6.3 Stack (用户栈)

- **栈顶**: `USER_STACK_TOP = 0x8_0000_0000`
- **栈底**: `USER_STACK_TOP - USER_STACK_SIZE = 0x7_FFE0_0000`
- **大小**: 2 MB (固定)
- **增长方向**: 向低地址增长
- **权限**: `R|W|U`
- **分配方式**: **即时分配** — `from_elf` 创建时一次性映射全部 2MB
- **MapAreaType**: `Stack`

**保护**: 当前实现没有显式的 guard page（栈底下方没有预留无映射的保护页）。栈溢出将直接触发 page fault → SIGSEGV。

### 6.4 ELF Segments (程序段)

- **加载基址**:
  - 非 PIE 静态链接: 由 ELF `PT_LOAD` 段的 `p_vaddr` 决定（通常从 `0x10000` 起）
  - PIE / `ET_DYN`: `load_base = 0x4000_0000`
  - 动态链接解释器 (`ld-linux`): 有单独的 `interp_base` 和 `interp_entry`
- **权限**: 直接从 ELF `PF_R|PF_W|PF_X` 翻译，始终加上 `U`
- **分配方式**: **即时分配** — `map_load_segments` 遍历所有 `PT_LOAD` 段，立即分配页帧并拷贝数据
- **MapAreaType**: `ElfSegment`

**特殊处理**: 当两个 `PT_LOAD` 段在同一页重叠时（如 .text 和 .data 共享页边界），`map_one` 会分配新帧、拷贝旧内容、合并权限标志位。

---

## 7. 页表与权限保护机制

### 7.1 Sv39 页表结构

```
VPN[2] (9 bit) → 第一级页表 (root)
VPN[1] (9 bit) → 第二级页表
VPN[0] (9 bit) → 第三级页表 (leaf)
Offset (12 bit) → 页内偏移
```

- 每级 512 个 PTE，每个 PTE 8 字节
- 一个页表节点占恰好 1 个 4KB 物理页

### 7.2 PTEFlags 位域

```
bit 0: V (Valid)           — 页表项有效
bit 1: R (Read)            — 可读
bit 2: W (Write)           — 可写
bit 3: X (Execute)         — 可执行
bit 4: U (User)            — 用户态可访问
bit 5: G (Global)          — 全局映射 (TLB 不随 ASID 刷新)
bit 6: A (Accessed)        — 已访问
bit 7: D (Dirty)           — 已修改
```

`MapPermission` 的 bit 编号与 `PTEFlags` 完全对齐（R=1, W=2, X=3, U=4），可直接 `from_bits` 转换。

### 7.3 各区域权限矩阵

| 区域 | R | W | X | U | 说明 |
|---|---|---|---|---|---|
| 内核 .text | Y | - | Y | - | 代码段只读可执行 |
| 内核 .rodata | Y | - | - | - | 只读数据 |
| 内核 .data/.bss | Y | Y | - | - | 可读写数据 |
| 内核 ekernel→MEMORY_END | Y | Y | - | - | 帧分配器物理页池 |
| MMIO 区域 | Y | Y | - | - | 设备寄存器 |
| 用户 ELF .text | Y | - | Y | Y | 用户代码 |
| 用户 ELF .data | Y | Y | - | Y | 用户数据 |
| 用户 Heap | Y | Y | - | Y | 堆 |
| 用户 Stack | Y | Y | - | Y | 栈 |
| 用户 Mmap | 按 prot | 按 prot | 按 prot | Y | mmap 映射 |
| 信号返回 trampoline | Y | - | Y | Y | sigreturn stub |

### 7.4 mprotect 实现

`change_protection()`（`memory_set.rs:1489-1590`）支持对任意虚拟地址范围修改权限：

1. **覆盖检查**: 验证请求范围完全被已有 VMA 覆盖，否则返回 `ProtectError::Unmapped`
2. **权限检查**: 对 shared file-backed 映射，不允许设置 W 如果 fd 是只读的（返回 `AccessDenied`）
3. **VMA 拆分**: 如果请求范围只覆盖 VMA 的一部分，会 `split_off` 拆分成 2-3 个子 VMA
4. **PTE 更新**: 调用 `apply_perm` 修改每个 VPN 的 PTE flags

### 7.5 内核栈保护

内核栈使用 `Vec<u128>` 在堆上分配 64KB，保护策略：

- **Guard magic**: 栈底 4 个 `u128` word 写入 `KSTACK_GUARD_MAGIC ^ i`
- **Fill pattern**: 所有栈空间初始化为 `KSTACK_FILL_MAGIC = 0xA5A5...`
- **运行时检测**: `check_guard()` 函数验证 guard word 完整性
- **对象池**: `KSTACK_POOL`（容量 16）回收释放的栈 Vec，减少 buddy allocator 碎片

**局限**: 没有使用硬件级 guard page（无映射页），而是用软件 magic number 检测。如果栈溢出恰好跳过 guard region 则无法检测。

---

## 8. COW (Copy-on-Write) 策略

### 8.1 设计概述

COW 在 `fork()` 时避免立即复制父进程的所有物理页。父子进程共享同一物理帧，仅在首次写入时才分配新帧并复制数据。

核心利用 `Arc<FrameTracker>` 的引用计数来判断物理帧是否被共享。

### 8.2 Fork 时的 COW 设置 (`from_existed_user`)

**代码位置**: `memory_set.rs:918-1059`

流程：

```
for each parent MapArea:
  1. 创建 child MapArea (空 data_frames)
  2. 对每个已映射的 VPN:
     if is_shared (SHM/MAP_SHARED):
        → 父子共享同一 Arc<FrameTracker>，保持 W 位
     else if is_writable && is_private:
        → 移除父 PTE 的 W 位 (read-only)
        → 子 PTE 也映射为 read-only
        → 共享同一 Arc<FrameTracker> (引用计数 +1)
     else (read-only):
        → 直接共享，PTE 不变
  3. 将 child area 加入 child.areas
flush_tlb()  // 刷新父进程 TLB (因为修改了 W 位)
```

**大面积优化**: 当 VMA 超过 `FULL_VPN_SCAN_LIMIT = 4096` 页时，只遍历 `data_frames` BTreeMap 中已有的物理页，跳过尚未 fault-in 的 lazy 页。这对 fork14 等测试中的巨大稀疏 mmap 映射至关重要。

### 8.3 COW 缺页处理 (`handle_cow_fault`)

**代码位置**: `memory_set.rs:1093-1173`

**触发条件**: Store Page Fault，且 PTE valid + not writable

```
handle_cow_fault(addr):
  1. 查找包含 fault_vpn 的 MapArea (线性扫描 areas Vec)
  2. 检查 VMA 权限包含 W → 否则 return false (真正的 SIGSEGV)
  3. 检查 PTE: valid && !writable → 否则 return false
  4. if MapAreaKind::Shared:
     → 直接恢复 W 位 (SHM 不做 COW)
  5. 获取 Arc<FrameTracker>
  6. if Arc::strong_count == 1:
     → 唯一持有者，直接恢复 W 位，无需拷贝 ★ 重要优化
  7. else (strong_count > 1):
     → 分配新物理页
     → 拷贝 4096 字节
     → 替换 data_frames 中的 Arc (旧引用计数 -1)
     → 重新映射 PTE (新 ppn + W)
  8. flush_tlb_page(vpn)
```

### 8.4 COW 的问题与不足

1. **线性扫描 VMA**: `areas.iter_mut().find()` 是 O(n)，在 VMA 数量多时性能差
2. **无零页优化**: 不像 Linux/Chronix 共享静态零页，每个零页各自独立分配
3. **PTE 状态判断粗糙**: 仅凭 `valid && !writable` 判断 COW，没有专用的 COW 标志位（如 Linux 的 `_PAGE_COW`），可能误判
4. **TLB 刷新粒度**: `flush_tlb_page` 只刷当前 hart 的单个 TLB 条目，多核环境下不安全（但 rcore-lab 目前是单核）
5. **无 page cache 集成**: 文件 mmap 的 COW 无法复用 page cache 帧，每次都是独立拷贝

---

## 9. Demand Paging (按需分页) 策略

### 9.1 设计概述

Demand paging 允许注册一个 VMA（`lazy = true`）但不分配任何物理页。首次访问触发 page fault 时才分配帧并建立映射。

### 9.2 Lazy VMA 注册

| 函数 | 用途 | lazy 标志 |
|---|---|---|
| `insert_lazy_anon_area()` | 匿名私有 mmap | `lazy = true` |
| `insert_lazy_file_area()` | 文件支撑 mmap | `lazy = true`, `file_back = Some(file, offset)` |
| `push()` (普通 MapArea) | ELF/Stack/Heap | `lazy = false` (即时分配) |

### 9.3 缺页处理 (`handle_demand_fault`)

**代码位置**: `memory_set.rs:222-270`

```
handle_demand_fault(addr):
  1. 查找 lazy=true 且包含 fault_vpn 的 MapArea (线性扫描)
  2. 如果 PTE 已 valid → return false (这是 protection fault, 不是 missing page)
  3. 如果是 file-backed:
     → 计算 file_offset = base_off + page_idx * PAGE_SIZE
     → 预先获取 file Arc clone (避免混合借用)
  4. 调用 area.map_one(&mut page_table, fault_vpn):
     → frame_alloc() 分配零页
     → 安装 PTE
  5. 如果 file-backed:
     → 将文件内容 read_at_kernel(file_off, page_buf) 写入新页
  6. return true
```

### 9.4 Page Fault 总调度流程

**代码位置**: `trap/user_trap_riscv64.rs:21-53`

```
Store/Load/InstructionPageFault(stval)
  ┌──────────────────┐
  │ handle_cow_fault  │ → true: 已处理 (COW break), return
  └────────┬─────────┘
           │ false
  ┌────────▼─────────┐
  │handle_demand_fault│ → true: 已处理 (lazy fill), return
  └────────┬─────────┘
           │ false
  ┌────────▼─────────┐
  │ SIGSEGV 信号投递  │ → 非法访问
  └──────────────────┘
```

**关键**: COW 检查先于 demand fault，因为 COW fault 的 PTE 是 valid+readonly，而 demand fault 的 PTE 是 invalid/absent。

### 9.5 Demand Paging 的问题与不足

1. **VMA 查找 O(n)**: 与 COW 相同问题，线性扫描 `areas` Vec
2. **无 read/write 区分**: demand fault 不区分读访问和写访问，始终分配可写帧。对于 `PROT_READ` 的 mmap，可以映射共享零页来节省物理内存
3. **无预读 (readahead)**: 文件 mmap fault 每次只读一页，没有预读相邻页面来减少 fault 次数
4. **无 page cache**: 文件内容直接读入私有帧，不复用 VFS 层的 page cache。相同文件被多个进程 mmap 时，每个进程各自持有独立副本
5. **lazy 标志是 VMA 级的**: 一旦某页被 fault-in，VMA 仍然是 `lazy=true`（靠 PTE valid 检查来避免重复分配），语义上不够清晰

---

## 10. 内核内存分配器

### 10.1 两级分配架构

```
┌─────────────────────────────────────────┐
│   alloc::Vec / BTreeMap / Arc / Box     │  Rust 动态分配 API
├─────────────────────────────────────────┤
│   TracedLockedHeap (#[global_allocator]) │  buddy_system_allocator
│   HEAP_SPACE: [u8; 128 MB]  (BSS 段)    │  管理所有 < 128MB 的内核堆分配
├─────────────────────────────────────────┤
│   StackFrameAllocator                    │  管理物理页帧 (4KB 粒度)
│   [ekernel PA .. MEMORY_END PA]          │  用于用户态页帧 + 页表节点
└─────────────────────────────────────────┘
```

### 10.2 物理帧分配器 (`StackFrameAllocator`)

- **策略**: 线性推进 + 回收栈
  - `current` 指针单调递增分配新帧
  - `recycled: Vec<usize>` 回收释放的帧号
  - 优先从 `recycled` 分配，用完再从 `current` 推进
- **RAII**: `FrameTracker` Drop 时自动归还帧号到 `recycled`
- **批量分配**: `frame_alloc_more(n)` 一次分配连续 n 页
- **零初始化**: `FrameTracker::new` 对分配到的页做全零清除

### 10.3 内核堆分配器 (`TracedLockedHeap`)

- **底层**: `buddy_system_allocator::LockedHeap` (二叉伙伴系统)
- **追踪**: 记录最近 64 次 >= 4KB 的分配操作到环形缓冲 `AllocTraceRing`
- **OOM 诊断**: `handle_alloc_error` 输出极其详细的诊断信息（堆统计、帧分配器状态、进程/线程数、fd 表摘要等）

---

## 11. 与 Chronix 的对比与改进 TODO

通过分析 `oskernel2025-chronix-retest` 的内存管理实现，以下是 Chronix 做得更好的地方，以及 rcore-lab 可以参考改进的方向：

### TODO 1: VMA 容器从 `Vec` 升级为 `RangeMap`

**现状**: rcore-lab 使用 `Vec<MapArea>` 存储所有 VMA，查找/插入/删除均为 O(n)。

**Chronix 方案**: 使用自定义 `RangeMap<VirtPageNum, UserVmArea>`（基于 BTree 的区间映射），提供 O(log n) 的点查询、范围搜索和空闲区间查找。

**影响**: 当进程有大量 VMA（如 LTP 测试中的上千个 mmap 区域）时，线性扫描成为严重瓶颈。COW fault 和 demand fault 每次都需要遍历。

**优先级**: **高**。这是最影响性能的架构问题。

---

### TODO 2: 引入静态零页 (Zero Page) 优化

**现状**: rcore-lab 的 demand fault 每次都分配一个新的物理帧并零初始化。

**Chronix 方案**: 维护一个全局 `ZERO_PAGE_ARC: StrongArc<FrameTracker>`，所有匿名读 fault 映射到同一只读零页。仅当写入时才 COW 分配私有帧。

**收益**: 大量匿名 mmap + 只读访问的场景可节省大量物理帧（如 `calloc` 大块内存但只读部分区域）。

**优先级**: 中。

---

### TODO 3: COW 引用计数从 `Arc` 升级为 `StrongArc`

**现状**: rcore-lab 使用标准 `Arc<FrameTracker>` 管理共享帧，`Arc::strong_count` 检查引用数。

**Chronix 方案**: 使用自定义 `StrongArc`（无 weak 引用、无额外元数据），COW break 时通过 `emplace()` 原子替换，是 lock-free 的。

**收益**: 减少原子操作开销，lock-free COW break 在多核环境下更高效。

**优先级**: 低（rcore-lab 目前单核，`Arc` 足够）。

---

### TODO 4: 文件 mmap 与 Page Cache 集成

**现状**: rcore-lab 的文件 mmap fault 直接从 inode 读数据到私有帧，没有 page cache 层。多个进程 mmap 同一文件各自持有独立副本。

**Chronix 方案**: VFS inode 维护 `PageCache`（基于 `BTreeMap<offset, Arc<Page>>`）。只读 mmap fault 直接映射 page cache 帧（零拷贝），写入时 COW 私有化。shared mmap 直接修改 page cache 帧并标记 dirty。

**收益**: 大幅减少 I/O 和内存使用，实现真正的统一缓存。

**优先级**: **高**（尤其是运行大量共享库的场景）。

---

### TODO 5: 引入类型化的缺页处理分派

**现状**: rcore-lab 的 `handle_cow_fault` 和 `handle_demand_fault` 是两个独立函数，串行调用。demand fault 内部不区分 Data/Stack/Heap/Mmap 的不同语义。

**Chronix 方案**: 每个 `UserVmAreaType`（Data/Stack/Heap/Mmap）实现独立的 `handle_lazy_page_fault` handler，通过 match 分派。不同类型有不同的 fault 策略（如 Stack 可以自动扩展、Mmap 可以 readahead）。

**收益**: 更清晰的代码结构，便于为不同区域定制策略。

**优先级**: 中。

---

### TODO 6: User Pointer 安全层

**现状**: rcore-lab 在 syscall 中手动翻译用户指针，没有编译期安全保障。

**Chronix 方案**: 提供 `UserPtr<T, ReadMark>` / `UserPtr<T, WriteMark>` 类型系统，`ensure_read()`/`ensure_write()` 在编译期区分读写语义，运行时使用硬件探测（`try_read_user`/`try_write_user`）作为 fast path。

**收益**: 消除用户指针验证的遗漏风险，硬件探测加速热路径。

**优先级**: 中。

---

### TODO 7: mremap 支持

**现状**: rcore-lab 尚未实现 `sys_mremap`。

**Chronix 方案**: 完整支持 `MREMAP_MAYMOVE`、`MREMAP_FIXED`、`MREMAP_DONTUNMAP`，通过 `move_frames_to` 迁移帧映射而非拷贝数据。

**优先级**: 低（等 LTP mremap 相关测试需要时再实现）。

---

### TODO 8: 内核栈 Guard Page 硬件保护

**现状**: rcore-lab 使用软件 magic number 检测栈溢出。

**改进方案**: 在内核栈底部预留 1 个 unmapped 页作为 guard page，溢出时立即触发 page fault，而非依赖 magic number 检测（可能被跳过）。

**优先级**: 低（内核栈溢出在正常运行中极少发生）。

---

### 总结优先级排序

| 优先级 | 改进项 | 预估工作量 |
|---|---|---|
| **P0** | VMA 容器升级为 RangeMap/BTreeMap 索引 | 大 |
| **P0** | 文件 mmap 与 Page Cache 集成 | 大 |
| **P1** | 静态零页优化 | 小 |
| **P1** | 类型化缺页处理分派 | 中 |
| **P1** | User Pointer 安全层 | 中 |
| **P2** | StrongArc COW | 小 |
| **P2** | mremap 支持 | 中 |
| **P2** | 内核栈 Guard Page | 小 |
