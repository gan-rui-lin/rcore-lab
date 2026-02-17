# rcore-lab 地址空间与 mmap 说明

本文目标是解释进程地址空间如何构建、是否会出现交叠，以及 mmap 基址如何选择与更新。内容以当前代码为准，重点参考内核的内存管理实现与 exec 装载流程。

## 1. 进程地址空间的大框架

rcore-lab 的用户态地址空间由 MemorySet 统一描述。它包含一个页表与一组 MapArea。每个 MapArea 对应一段连续的虚拟地址范围，映射方式可以是 Framed 或 Identical。对用户进程而言，主要使用 Framed 映射。

地址空间整体包含以下几类区域：

1) 程序段映射（ELF PT_LOAD）
- 由 MemorySet::from_elf 解析 ELF 的 program headers。
- 对每个 PT_LOAD 计算 vaddr 和 memsz，对应的虚拟区间用 MapPermission(U + R/W/X) 映射。
- 段内容用 copy_data 拷贝到新分配的物理页上。
- 若 ELF 为静态可执行，load_base 为 0；若是 ET_DYN 且无 PT_INTERP，则 load_base = 0x4000_0000（PIE 类）。

2) 用户栈（USER_STACK_SIZE + guard page）
- 在 heap_bottom 之后创建 guard page，再映射用户栈。
- 栈顶部固定在 user_stack_top，栈从高地址向低地址增长。

3) 堆（brk 区域）
- heap_bottom 是所有 PT_LOAD 最大末端对齐到页后的结果。
- sbrk 调整 program_brk，内核通过 append_to 或 shrink_to 追加/收缩映射。

4) TrapContext 与 Trampoline
- TRAMPOLINE 在最高地址处映射，供用户态返回内核与 trap 使用。
- TRAP_CONTEXT_BASE 到 TRAMPOLINE 之间映射用于保存 TrapContext。

5) mmap 区域
- 由 sys_mmap 分配，基址由进程结构中的 mmap_base 控制。

这些区域整体的布局是从低地址起的程序段 -> 堆 -> guard + 用户栈 -> TRAP_CONTEXT_BASE -> TRAMPOLINE。mmap 的默认基址设置在一个较高、固定的区域，避免干扰程序段与堆。

## 2. MemorySet::from_elf 的关键流程

MemorySet::from_elf 做了三件事：

1) 解析 ELF header 与 PT_LOAD
- 遍历所有 program headers，定位最小 vaddr 与是否有 PT_INTERP。
- 决定 load_base，用于 ET_DYN 的 PIE 类可执行。

2) 映射并拷贝 PT_LOAD
- 计算 start_va = load_base + ph.vaddr
- 计算 end_va = load_base + ph.vaddr + ph.memsz
- 使用 MapPermission(U + R/W/X) 构造 MapArea 并 push。
- 文件数据拷贝自 ph.offset .. ph.offset + ph.filesz。

3) 建立用户栈与初始堆
- heap_bottom = max_end_vpn 对应的虚拟地址。
- user_stack_bottom = heap_bottom + PAGE_SIZE (guard)
- user_stack_top = user_stack_bottom + USER_STACK_SIZE
- heap_bottom 本身也会被插入一个 MapArea（用于 sbrk 扩展）。

from_elf 不会直接设置 mmap_base，这个动作在 ProcessControlBlock::exec 中完成。

## 3. exec 与栈布局

ProcessControlBlock::exec 会：

1) 用 from_elf 创建新的 MemorySet，并写入进程内核状态。
2) 初始化 TLS/TCB（无 PT_TLS 的情况下，放置一个最小 TCB，设置 tp）。
3) 构建用户栈：
   - 写入 envp 字符串与 argv 字符串。
   - 对齐 sp，写入 argc / argv / envp / auxv。
   - auxv 中包含 AT_ENTRY/AT_PHDR/AT_PHNUM/AT_PAGESZ/AT_RANDOM 等。

## 4. mmap 基址与交叠问题

当前实现中：

- DEFAULT_MMAP_BASE = 0x4000_0000
- 进程在 exec 时把 mmap_base 设置为 max(DEFAULT_MMAP_BASE, heap_bottom 对齐后)

这解决了一个关键问题：

- 如果执行的是 ET_DYN 或动态链接器，其 PT_LOAD 可能映射在 0x4000_0000 附近。
- 旧逻辑直接把 mmap_base 固定为 0x4000_0000，当动态链接器后续调用 sys_mmap (req=0) 时，会从 mmap_base 分配，可能与映射区域冲突。
- 这个冲突会触发页表 map 时的 “vpn is mapped before mapping” panic。

因此，新的逻辑让 mmap_base 至少在 heap_bottom 之后，这样匿名 mmap 会从已加载镜像的末端之后开始，避免交叠。

结论：
- 地址空间设计本身不允许交叠，交叠会触发 panic。
- 如果出现交叠，通常是 mmap_base 选择不当或 ET_DYN 基址与 mmap 冲突。

## 5. mmap 与文件映射

sys_mmap 中支持 MAP_ANON 和文件映射：

1) 计算权限：prot -> MapPermission(U + R/W/X)。
2) 确定映射起始地址：
   - MAP_FIXED 且 start != 0: 直接使用。
   - start != 0: 选择对齐后 start，并更新 mmap_base。
   - start == 0: 选择 mmap_base 对齐后的地址，并更新 mmap_base。
3) 插入 Framed Area，分配物理页。
4) 文件映射时，按页读入文件内容。

注意：
- 目前没有高级 VMA 管理，映射冲突会直接 panic。
- 对动态链接器，这种 mmap 方式只能“勉强跑起来”，仍可能缺少完整的动态链接支持。

## 6. 与动态链接器的关系

用户态的动态 ELF 会包含 PT_INTERP，表示需要解释器（ld-linux）。

现有 exec 做了以下处理：
- 若发现 PT_INTERP，则尝试把 exec_path 改为解释器路径，并把原始程序作为 argv[1] 传给解释器。
- 若 /lib/ld-linux-riscv64-lp64d.so.1 不存在，会回退到 /musl/lib/libc.so。
- 需要保证 /lib/ld-linux-riscv64-lp64d.so.1 的硬链接存在，否则解释器找不到。

这套逻辑在形式上类似于 Linux 的动态链接执行流程，但内核并未实现完整的动态链接器 ABI 细节，可能依赖 musl 的兼容性。

## 7. 总结

- 地址空间是由 MemorySet 统一管理，MapArea 不允许交叠。
- 程序加载顺序为：ELF PT_LOAD -> 堆 -> 栈 -> TrapContext/Trampoline。
- mmap 基址必须避开已加载镜像，否则会触发重复映射 panic。
- 动态 ELF 会触发解释器执行，需要 /lib/ld-linux-riscv64-lp64d.so.1 指向 musl 的可用解释器（目前通过硬链接解决）。

如果后续需要深入优化，可考虑：
- 引入 VMA 结构管理 mmap 区间与冲突检测。
- 为 ET_DYN 选择更安全的 load_base，避免与 mmap_base 交叠。
- 实现更完整的动态链接器支持与 auxv 初始化。