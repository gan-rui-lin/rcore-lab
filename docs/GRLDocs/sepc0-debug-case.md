# 调试记录：sepc = 0 导致 InstructionPageFault

本文记录一次用户态 `InstructionPageFault (scause=12)` 的调试过程，最终定位为 `.init_array` 未正确拷贝导致函数指针为 0。

## 现象

运行 busybox 测试时出现：

```
[kernel] trap_handler:  Exception(InstructionPageFault) in application, bad addr = 0x0, bad instruction = 0x0
```

GDB 中观察到：

```
(gdb) p/x os::task::processor::current_trap_cx().sepc
$3 = 0x0
```

说明 **用户态真的在取指地址 0x0**。

## 定位过程

### 1) 先加载符号

```gdb
(gdb) add-symbol-file /home/grl/codeRepo/rcore-lab/busybox/musl/busybox 0x10120
```

`0x10120` 来自 busybox ELF 的 `.text` 段**虚拟地址（VMA）**。可用以下方式确认：  

```bash
readelf -S busybox/musl/busybox | rg '\\.text'
# 会看到：.text 0000000000010120 ...
```

或者：

```bash
readelf -l busybox/musl/busybox
# 看 LOAD 段的 VirtAddr（文本段）与 Entry point
```

因此对 **非 PIE / 未重定位** 的程序，`add-symbol-file` 直接使用 `.text` VMA 即可。  
若是 PIE 或运行时重定位，则需要 **运行时基址 + ELF VMA** 重新计算后再加载符号。

确认 `sepc` 附近的返回地址：

```gdb
(gdb) set $cx = os::task::processor::current_trap_cx()
(gdb) p/x $cx.x[1]   # ra
$13 = 0x104a7c
(gdb) info symbol 0x104a7c
__libc_start_init + 72
```

这里用 `info symbol 0x104a7c` **不是“突然想到”**，而是因为 `sepc` 已经是 0，无法定位；  
所以改用 `ra`（返回地址）来判断“是谁调用/跳转到了 0”。

### 2) 反推调用点

```gdb
(gdb) set $call = $cx.x[1] - 4
(gdb) x/4i debug_user_va_to_pa($call)
   ...
   jalr a5
```

检查 `a5`：

```gdb
(gdb) p/x $cx.x[15]
$15 = 0x0
```

结论：`jalr a5` 发生在 `__libc_start_init`，而 `a5=0`，所以 PC 跳到了 0。

### 3) 追到 `.init_array`

`__libc_start_init` 的逻辑是遍历 `.init_array`。

从 ELF 可知 `.init_array`：

```
readelf -x .init_array busybox
# 0x00162ff0: 0x0000000000010238
```

这行的含义是：在 ELF 的**虚拟地址** `0x162ff0` 处，有一个 8 字节的函数指针，值为 `0x10238`。
`__libc_start_init` 会依次调用这些指针（构造函数）。如果这里是 0，就会 `jalr` 跳到 0。

这些数据最终需要被**内核在 `exec` 时拷贝**到用户地址空间里：
- 内核解析 ELF 的 `LOAD` 段
- 为每个段建立映射
- 把 ELF 文件里的数据复制到对应的虚拟地址

但运行时实际读取到的是 0，说明 `.init_array` 没被拷贝到它的目标地址 `0x162ff0`。

## 根因

busybox 的 RW 段起始地址是 **非页对齐** 的 `0x162ff0`：

```
LOAD 0x0000000000151ff0 -> VMA 0x0000000000162ff0
```

而内核的 `MapArea::copy_data()` 逻辑是从 **页头** 写入，忽略了起始虚拟地址的页内偏移，导致：
- 页头被写入了正确数据
- 真正的 `0x162ff0` 仍是 0

`.init_array` 的函数指针因此为 0，触发 `jalr a5` 跳到 0。

**调用链说明（拷贝发生在哪）：**

1. `ProcessControlBlock::exec`  
   在 `os/src/task/process.rs` 中调用 `MemorySet::from_elf(elf_data)`。
2. `MemorySet::from_elf`  
   在 `os/src/mm/memory_set.rs` 中遍历 ELF 的 `LOAD` 段，创建 `MapArea`，并调用 `self.push(map_area, Some(data))`。
3. `MemorySet::push`  
   在 `os/src/mm/memory_set.rs` 中先 `map_area.map(...)`，再调用 `map_area.copy_data(...)`。  
   **`MapArea::copy_data` 就是发生“页内偏移未处理”问题的地方。**

## 修复

在 `os/src/mm/memory_set.rs` 修改 `MapArea::copy_data()`，正确处理 **起始 VA 的页内偏移**。

关键行为：
- 第一个页写入时使用 `start_va.page_offset()`
- 后续页从 0 偏移写

## 验证

1) 重编译：
```
make debug
```

2) 在 GDB 中验证 `.init_array` 已正确拷贝：

```gdb
(gdb) p/x debug_user_va_to_pa(0x162ff0)
(gdb) x/gx <pa>
# 期望看到 0x0000000000010238
```

3) 运行测试：`sepc` 不再为 0，`InstructionPageFault` 消失。

## 经验总结

- 当 `sepc=0` 时，优先检查 **函数指针为空**。
- `jalr` 的源寄存器值是关键（这里是 `a5`）。
- 需要确认 ELF 段是否**按正确偏移**拷贝，尤其是 **非页对齐**段。
