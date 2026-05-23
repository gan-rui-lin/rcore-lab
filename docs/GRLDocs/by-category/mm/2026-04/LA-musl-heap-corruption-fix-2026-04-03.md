# LoongArch musl LTP 测试崩溃修复记录

**日期**: 2026/04/03

## 问题描述

LoongArch 架构下运行 musl LTP 测试时，测试直接崩溃，返回 `status=0x8b`（128 + 11 = SIGSEGV）。

```
=== Running /tmp/ltp_testcode_filtered.sh ===
#### OS COMP TEST GROUP START ltp-musl ####
=== /tmp/ltp_testcode_filtered.sh completed (status=0x8b) ===
```

而 basic 测试和 glibc 测试都能正常通过。**此问题仅出现在 LoongArch 架构**，RISC-V 没有这个问题。

## 问题分析

### 1. 初始错误日志

```
[ERROR] [kernel] trap_handler: page fault addr=0x120200000 pid=2 tid=0 name=busybox sepc=0x12018b688
[ERROR] [kernel] trap_handler: fault pte ppn=0x98479 flags=V | R | X | U bytes=[00, 00, 00, 00, 00, 00, 00, 00]
```

关键观察：
- 故障地址：`0x120200000`
- PTE 标志：`V | R | X | U`（有效、可读、可执行、用户态），但**没有写权限 W**
- 这是一个写入到只读页面的错误

### 2. 架构差异对比

| 特征 | RISC-V | LoongArch |
|------|--------|-----------|
| heap_bottom | 0x166000 | 0x1201fe000 |
| mmap(PROT_NONE) 在堆区域 | 否 | 是 |
| 崩溃 | 否 | 是 |

**关键发现**：LoongArch musl busybox 使用了不同的 malloc 实现策略，会在堆区域调用 `mmap(PROT_NONE, MAP_FIXED)` 创建 guard page。

### 3. 根本原因

通过追踪系统调用序列发现：

```
[TRACE] [sys_sbrk] pid=2 ... new=0x120200000 ok
[TRACE] [sys_mmap] pid=2 name=busybox req=0x1201fe000 len=0x1000 prot=0x0 flags=0x32 -> start=0x1201fe000 overlap=1 fixed=true
```

**问题流程**：

1. `sys_sbrk` 扩展堆到 `0x120200000`，权限 `R | W | U`
2. LoongArch musl malloc 调用 `sys_mmap` 在堆起始地址 `0x1201fe000` 使用 `MAP_FIXED`
3. `prot=0x0`（PROT_NONE）意味着新映射没有任何读写权限
4. `unmap_range` 移除/缩小了原堆区域
5. `insert_mmap_area` 创建新区域，权限只有 `U`
6. 下次 `sys_sbrk` 调用 `append_to` 查找 `heap_bottom` 地址的区域
7. 找到的是 mmap 创建的只有 `U` 权限的区域，而不是原来的堆区域
8. 新映射的页面没有写权限，导致写入时触发 SIGSEGV

### 4. 为什么只有 LoongArch 受影响

LoongArch musl 的 malloc 实现更积极地使用 mmap 和 guard page，这与 RISC-V musl 的实现不同。这可能是因为：
- 不同版本的 musl
- LoongArch 特定的编译选项
- 不同的内存布局导致的行为差异

## 解决方案

### 修复策略

添加 LoongArch 特定的堆扩展函数，通过权限验证确保找到真正的堆区域。

### 代码修改

**os/src/mm/memory_set.rs** - 添加 `append_to_heap` 函数：

```rust
/// append the heap area to new_end, creating a new heap area if the original was corrupted
pub fn append_to_heap(&mut self, heap_bottom: VirtAddr, new_end: VirtAddr) -> bool {
    let start_vpn = heap_bottom.floor();
    let expected_perm = MapPermission::R | MapPermission::W | MapPermission::U;
    
    // Find a heap-like area (R|W|U permissions) starting at heap_bottom
    if let Some(area) = self
        .areas
        .iter_mut()
        .find(|area| {
            area.vpn_range.get_start() == start_vpn && 
            area.map_perm == expected_perm
        })
    {
        area.append_to(&mut self.page_table, new_end.ceil());
        return true;
    }
    
    // No valid heap area found - create a new one
    let map_area = MapArea::new(heap_bottom, new_end, MapType::Framed, expected_perm);
    self.push(map_area, None);
    true
}
```

**os/src/syscall/process.rs** - 修改 `sys_sbrk` 使用架构特定逻辑：

```rust
} else {
    // LoongArch musl uses mmap(PROT_NONE, MAP_FIXED) in heap region which can
    // corrupt the heap area. Use append_to_heap which validates permissions.
    #[cfg(target_arch = "loongarch64")]
    {
        inner.memory_set.append_to_heap(VirtAddr(heap_bottom), VirtAddr(new_brk))
    }
    #[cfg(not(target_arch = "loongarch64"))]
    {
        inner.memory_set.append_to(VirtAddr(heap_bottom), VirtAddr(new_brk))
    }
};
```

## 测试验证

修复后两个架构都通过测试：

**LoongArch**:
```
ltp-musl:   status=0x0 ✅
basic-musl: status=0x0 ✅
ltp-glibc:  status=0x0 ✅
basic-glibc: status=0x0 ✅
```

**RISC-V**:
```
basic-musl: status=0x0 ✅
basic-glibc: status=0x0 ✅
```

## 经验总结

1. **架构特定问题需要对比分析**：通过对比 RISC-V 和 LoongArch 的系统调用序列，快速定位问题是 LoongArch 特有的。

2. **musl 和 glibc 的 malloc 实现差异**：不同的 C 库实现有不同的内存管理策略，需要内核适配。

3. **不要依赖地址来标识区域**：使用权限验证或类型标识更健壮。

4. **架构特定修复**：使用 `#[cfg(target_arch)]` 限定修复范围，避免影响其他架构。

## 相关文件

- `os/src/mm/memory_set.rs`: 添加 `append_to_heap` 函数
- `os/src/syscall/process.rs`: 修改 `sys_sbrk` 使用架构特定逻辑
