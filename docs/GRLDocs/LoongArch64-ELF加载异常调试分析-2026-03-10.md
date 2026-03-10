# LoongArch64 内核在 ELF 加载阶段异常的调试分析

日期：2026/3/10

## 一、问题现象与结论先行

本次异常发生在用户程序 ELF 装载流程中，具体表现为：在 `MemorySet::from_elf -> map_load_segments -> MapArea::copy_data` 的数据拷贝阶段触发异常，陷入 `trap_vector_base`，GDB 回溯显示异常点位于 `copy_from_slice` 内部。综合寄存器现场与代码路径分析，**罪魁祸首高度怀疑为“目标物理页帧地址非法（超出可用 RAM 范围或 PTE 地址抽取错误）”**，导致通过 DMW 直接映射访问该物理页时触发内存访问异常。

一句话结论：**ELF 段拷贝写入的目标页帧无效，根因在于页帧分配范围/物理地址抽取/页表项格式不一致引发的“错误物理地址”。**

## 二、关键现象复盘（从调试输出推断问题）

### 1. GDB 现场

GDB 中用户执行到：

- `MemorySet::map_load_segments` 分配映射后，进入 `memory_set.push(map_area, data)`
- 进入 `MapArea::copy_data`，最终触发异常。

异常后寄存器显示（用户给出的关键信息）：

- `PC=0x9000000090187000`（陷入 `trap_vector_base`）
- `ERA=0x9000000090199154`（返回地址落在 `copy_from_slice` 调用路径）
- `ESTAT=0x0000000000480000`

对 `ESTAT` 的解码：

- `ECODE = (ESTAT >> 16) & 0x3f = 0x08`

该值属于**访存异常类**（Load/Store/Fetch 或 TLB 异常的集合范围），与“访问非法物理页”高度一致。这说明不是普通的逻辑 panic，而是 CPU 在写入时直接触发硬件异常。

### 2. 关键寄存器迹象

寄存器中出现了类似：

- `r4 = 0x9000a06528c06000`
- `r7 = 0x9000a06528c07000`

这类地址形态具有两个明显特征：

1. 以 `0x9000...` 开头，符合 LoongArch DMW 的直接映射窗口（高位 VA = `VIRT_ADDR_START`）。
2. 中间段为 `0xa06528c0...`，对应的物理地址可能为 `0xa06528c0...` 或经过掩码/裁剪后的某段地址。

如果实际 RAM 仅 2G（如 QEMU 默认或 `run-la.sh` 指定 `-m 2G`），**那么超过 `0x8000_0000` 之后的物理地址即可能非法**。该寄存器值表明写入目标页帧的“物理地址”很可能已经偏离合法 RAM 区间，导致写入直接失败。

### 3. 代码路径定位

异常位置的核心逻辑：

- `MemorySet::map_load_segments` 为每个 ELF LOAD 段建立 `MapArea::new`，并 `memory_set.push`。
- `push` 中调用 `map_area.map()` 完成映射，然后 `copy_data` 写入段内容。
- `copy_data` 通过：

```
page_table.translate(current_vpn).unwrap().ppn().get_bytes_array()
```

得到目标页帧的内核可写切片，并执行 `copy_from_slice`。

因此一旦 `ppn` 错误或者 `get_bytes_array` 计算出非法 VA，就会在 `copy_from_slice` 触发异常。这与 GDB 看到的现场完全吻合。

## 三、根因分析（结合 LoongArch64 内存模型）

### 1. DMW 直接映射规则

LoongArch64 的 DMW（Direct Map Window）允许内核将一段高地址区直接映射到物理地址空间，例如：

- `VIRT_ADDR_START = 0x9000_0000_0000_0000`
- 访问 `VIRT_ADDR_START | pa` 等价于访问物理地址 `pa`

因此 `PhysAddr::get_mut()` 的实现逻辑通常是：

```
(pa | VIRT_ADDR_START) as *mut T
```

这要求 **pa 必须是有效物理地址**，否则立刻触发异常。

### 2. 物理页帧分配范围是否正确

当前 `frame_allocator` 的初始化逻辑为：

- 起始：`ekernel`（内核镜像结束）
- 终止：`MEMORY_END`

如果 `MEMORY_END` 配置不正确（例如仍保留为 RISC-V 逻辑或错误的 RAM 上限），则 allocator 会将“错误区间内的 ppn”当作合法页帧返回。此时 `get_mut` 会计算出一个落在 **不存在的物理 RAM 区间** 的 DMW 地址，导致写入时触发异常。

这个链条和当前异常高度一致。

### 3. PTE 格式与 PPN 抽取

LoongArch 的 PTE 格式与 RISC-V 不完全一致。当前代码在 `PageTableEntry::ppn()` 中使用：

```
(self.bits >> 12) & ((1 << 36) - 1)
```

这等价于 **只保留 36 位物理页号**。若系统物理地址宽度更大（LoongArch 物理地址可以到 56 位），在某些情况下此掩码会**截断高位**。

如果 `ppn` 被截断，实际转换到物理地址时就会跳转到错误位置，进而导致写入异常。虽然当前 QEMU 常用内存规模较小，按理不应跨越 36 位，但一旦内核误用了高位映射（如 `VIRT_ADDR_START` 与 `ppn` 未脱钩），此类截断会进一步放大问题。

### 4. 入口映射与 RAM 基址偏移

内核链接地址为 `0x9000000090000000`，说明内核位于高半区。若物理 RAM 实际基址不是 `0x0`，而是 `0x9000_0000` 之类，则需要确保 **物理地址 = 虚拟地址 - VIRT_ADDR_START** 的关系成立。否则，`PhysAddr::from(va)` 的掩码计算会得到错误的物理地址。

也就是说：

- 如果 QEMU 实际 RAM 基址不是 0
- 或 kernel 入口与 DMW 设定不一致

都会导致 `va -> pa` 的变换出现偏差。

## 四、为何不是其它问题

为了排除其它可能性，需说明以下几点：

1. **不是 ELF 解析错误**：
   - `map_load_segments` 可正常获取 `ph`，逻辑未触发 panic。
   - 数据长度和偏移没有超出 `elf.input` 的边界（否则更早就会 panic）。

2. **不是缺少 trampoline**：
   - LoongArch 依靠 DMW 直接映射，正常情况下无需 trampoline。异常发生在用户段写入，不是 trap return 或切换栈上下文，因此与 trampoline 关系不大。

3. **不是页表逻辑空指针**：
   - `translate(current_vpn).unwrap()` 未报错，说明 PTE 至少存在。
   - 真实的问题在于 PTE 指向的物理页本身不可访问。

## 五、参考实现的对比要点（OSKernel2025-rustoswhu）

参考仓库的 LoongArch 实现要点：

1. PTE 的地址抽取使用完整地址掩码：
   - `address() -> self.0 & 0xffff_ffff_f000`
   - 不强制 36 位裁剪。

2. PTE flags 的设置强调 `V | P | MAT_NOCACHE`，以及 PLV/写标志。

3. PageTable 修改与 TLB flush 比较谨慎，避免旧映射残留。

这说明**本仓库的 PTE 地址裁剪**可能是潜在风险点（即使不是直接根因，也需要确认）。

## 六、建议的下一步验证路径

### 1. 直接验证“目标物理地址是否越界”

在 `MapArea::copy_data` 中打印如下信息（仅打印首个页）：

- `current_vpn`
- `pte.bits`
- `ppn.0` 与 `ppn.0 << 12`
- 物理地址是否 < `MEMORY_END`

若 `ppn << 12 >= MEMORY_END`，即可确认“分配出了非法页帧”。

### 2. 对比 QEMU RAM 配置与 `MEMORY_END`

当前运行脚本 `run-la.sh` 默认 `-m 2G`，对应 RAM 范围为：

- 0x0000_0000 .. 0x7fff_ffff

如果 `MEMORY_END` 仍保持在 `0xb000_0000` 或更高，则 allocator 会错误分配 2G~3G 区间的页帧，直接导致非法访问。建议确认实际 RAM 基址与范围是否匹配。

### 3. 校验 PPN 抽取规则

将 `ppn()` 的掩码扩大到 `PA_WIDTH` 或使用 `address() & 0xffff_ffff_f000` 的方式，防止高位被截断。即便当前 RAM 较小，这一调整也能规避未来的隐患。

### 4. 校验 DMW 与 VA/PA 关系

确认：

- `VIRT_ADDR_START` 与 CSR 中设置的 DMW 区间一致；
- 链接地址是否与 DMW 区间匹配；
- 物理地址通过 `PhysAddr::from(va)` 计算时没有多余位或掩码错误。

## 七、小结

当前异常集中在 `copy_data` 阶段，且为硬件访存异常。结合寄存器值和 LoongArch DMW 机制，最合理的解释是：**页面分配出了越界或错误的物理页帧地址，导致内核使用 DMW 直接写入该页时发生异常。**

真正的“罪魁祸首”是：

- **Frame allocator 使用了错误的 RAM 终止地址（`MEMORY_END` 不匹配实际内存）**，或
- **PTE 物理地址提取/掩码错误**，导致写入访问了不存在的物理地址。

下一步调试应聚焦于“分配出的物理页帧地址是否越界”，一旦确认，即可沿着 `MEMORY_END` / PTE 抽取路径修正。