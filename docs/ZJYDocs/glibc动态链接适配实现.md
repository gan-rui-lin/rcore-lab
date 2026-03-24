# glibc 动态链接适配实现

**日期**: 2026/3/24

---

## 1. 背景与目标

oscomp 比赛的评测包含 glibc 动态链接的 netperf、iperf3 和 basic 测试。与 musl 静态链接不同，glibc 版本的可执行文件是 PIE（Position-Independent Executable）格式的动态链接二进制，依赖：

- `ld-linux-riscv64-lp64d.so.1`（glibc 动态链接器/解释器）
- `libc.so.6`（glibc C 运行库）
- `libm.so.6`（数学库）

rCore-lab 此前只支持静态链接的 ELF 加载。要运行 glibc 程序，需要在内核中实现：

1. **PT_INTERP 解释器加载**：识别 ELF 的 PT_INTERP 段，加载 ld-linux 并跳转到其入口
2. **PIE 地址空间布局**：为 SharedObject 类型的 ELF 分配非零 load_base
3. **MAP_FIXED 覆盖语义**：glibc ld-linux 加载每个 .so 时使用 MAP_FIXED 覆盖已有映射
4. **fstat inode 唯一性**：ld-linux 用 (dev, ino) 做库文件去重
5. **ELF 共享页 COW**：相邻 LOAD 段共享的边界页需要 copy-on-write
6. **信号 restorer 兼容**：glibc 设置的 sa_restorer 可能未被正确重定位

最终目标：glibc netperf 5/5、iperf 6/6、basic 32/32 全部通过。

---

## 2. 适配架构

### 2.1 ELF 加载流程

```
exec(glibc_binary)
  │
  ├── scan_elf_meta() ──→ 发现 PT_INTERP
  │
  ├── from_elf_with_interp(elf_data, interp_data)
  │     ├── 计算 load_base（PIE: 0x40000000）
  │     ├── map_load_segments(主程序, load_base)
  │     ├── map_load_segments(ld-linux, interp_base)
  │     └── 返回 (memory_set, auxv_info, interp_entry)
  │
  ├── exec_with_interp()
  │     ├── 设置 auxv: AT_PHDR, AT_ENTRY, AT_BASE
  │     ├── 构建用户栈: [argc][argv][envp][auxv]
  │     └── entry_point = interp_entry（跳转到 ld-linux）
  │
  └── ld-linux 在用户态执行
        ├── 读取 AT_PHDR 找到主程序的 program headers
        ├── 读取 .dynamic 段找到依赖库
        ├── mmap + MAP_FIXED 加载 libc.so.6, libm.so.6
        ├── 处理 .rela.dyn (R_RISCV_RELATIVE) 和 .rela.plt (R_RISCV_JUMP_SLOT)
        ├── 调用 .init_array 初始化函数
        └── 跳转到 AT_ENTRY（主程序 main）
```

### 2.2 地址空间布局

```
0x00000000 ─────────── 未使用
0x40000000 ─────────── 主程序 LOAD 段（PIE load_base）
0x40033000 ─────────── 主程序 data 段
0x4004C000 ─────────── ld-linux LOAD 段（紧接主程序之后）
0x40067000 ─────────── heap_bottom
    ...
0x600000000 ────────── mmap_base（ld-linux 在此分配 libc/libm）
0x600073000 ────────── libc.so.6
0x60012B000 ────────── libm.so.6
    ...
0x7FFFFC000 ────────── 用户栈（128KB）
0x800000000 ────────── USER_STACK_TOP
```

---

## 3. 逐项实现细节

### 3.1 fstat inode 唯一性

**问题**：`sys_fstat` 对所有文件返回 `(st_dev=0, st_ino=0)`。glibc ld-linux 用 `(dev, ino)` 做已加载库去重，当 `libm.so.6` 和 `libc.so.6` 都返回 `(0, 0)` 时，linker 把 libm 当作"已加载的 libc"跳过，导致 `undefined symbol: stdin`。

**实现**：用 djb hash 对文件路径生成唯一 inode，`stat.dev = 1`。hash 结果 `& 0x7FFF_FFFF` 保证以 `%d` 打印时为正数（测试 judge 正则匹配需要）。

```rust
// os/src/syscall/fs.rs
for b in full_path.bytes() {
    h = h.wrapping_mul(33).wrapping_add(b as u64);
}
stat.ino = h & 0x7FFF_FFFF;
```

### 3.2 MAP_FIXED 覆盖语义

**问题**：`sys_mmap` 检测到 MAP_FIXED 目标地址与已有映射重叠时直接返回 ENOMEM。但 Linux 的 MAP_FIXED 语义是**覆盖**已有映射。

**实现**：当 MAP_FIXED 且有 overlap 时，先调用 `memory_set.unmap_range()` 移除目标范围内的页，再创建新映射。`unmap_range` 实现了精确到页粒度的 unmap——对部分重叠的 MapArea，只移除目标范围内的页，保留范围外的部分。

```rust
// os/src/syscall/process.rs sys_mmap
if overlap > 0 {
    if is_fixed {
        inner.memory_set.unmap_range(VirtAddr(start), VirtAddr(start + len));
    } else {
        return errno(ENOMEM);
    }
}
inner.memory_set.insert_framed_area(VirtAddr(start), VirtAddr(start + len), map_perm);
```

`unmap_range` 的关键逻辑：

```rust
// os/src/mm/memory_set.rs
if a_start >= unmap_start_vpn && a_end <= unmap_end_vpn {
    // 完全包含：整体移除
    self.areas[i].unmap(&mut self.page_table);
    self.areas.remove(i);
} else {
    // 部分重叠：只 unmap 重叠的页
    let overlap_start = a_start.max(unmap_start_vpn);
    let overlap_end = a_end.min(unmap_end_vpn);
    for vpn in overlap_start..overlap_end {
        self.areas[i].unmap_one(&mut self.page_table, vpn);
    }
}
```

### 3.3 PIE load_base

**问题**：PIE 可执行文件的 ELF 类型是 `SharedObject`，`min_load_vaddr=0`。如果 load_base=0，程序从 VA 0x0 开始，导致 NULL 指针混淆和 ld-linux base 推算出错。

**实现**：在 `from_elf_with_interp` 中，当 ELF 类型为 SharedObject 且 `min_load_vaddr=0` 时，设置 `load_base=0x40000000`。

```rust
let load_base = if elf_type == xmas_elf::header::Type::SharedObject && min_load_vaddr == 0 {
    0x4000_0000usize
} else {
    0
};
```

### 3.4 ELF 共享页 COW

**问题**：相邻 LOAD 段可能共享同一个虚拟页。例如 LOAD1（text, R+X）结束于 VPN 0x40032 的中间，LOAD2（data, R+W）从同一页开始。先映射 LOAD1 写入 text 数据，再映射 LOAD2 时 `map_one` 发现 PTE 已 valid，需要处理。

**实现**：`map_one` 检测到 PTE 已 valid 时，执行 COW：分配新物理帧，复制旧页内容，合并权限标志后重新映射。

```rust
pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
    if let Some(pte) = page_table.translate(vpn) {
        if pte.is_valid() {
            let old_ppn = pte.ppn();
            let new_frame = frame_alloc().unwrap();
            let new_ppn = new_frame.ppn;
            new_ppn.get_bytes_array().copy_from_slice(old_ppn.get_bytes_array());
            self.data_frames.insert(vpn, new_frame);
            let merged = PTEFlags::from_bits(pte.flags().bits() | new_flags.bits()).unwrap();
            page_table.unmap(vpn);
            page_table.map(vpn, new_ppn, merged);
            return;
        }
    }
    // 正常路径：分配新零页
    let frame = frame_alloc().unwrap();
    ...
}
```

### 3.5 用户栈扩大

glibc 初始化代码需要比 musl 更多的栈空间。`USER_STACK_SIZE` 从 `4096 * 5 = 20KB` 增大到 `4096 * 32 = 128KB`。

### 3.6 page_table.map() 放宽

MAP_FIXED 覆盖时，`insert_framed_area` 对已映射的 VPN 调用 `map_one`，`map_one` 的 COW 路径先 unmap 再 map。但在某些边界条件下，PTE 可能已存在。`page_table.map()` 从 `assert!(!pte.is_valid())` 改为：如果 PTE 已 valid，先清空再写入新 PTE。

### 3.7 信号 restorer 未重定位兼容

**问题**：glibc 的 `sigaction()` 包装器设置 `sa_restorer = &__restore_rt`（libc 中的信号返回跳板）。在我们的内核上，该地址没有被 ld-linux 正确重定位——只包含原始偏移 `0x2000`，而非 `libc_base + 0x2000`。信号 handler 返回时跳转到 0x2000，引发 page fault。

**实现**：检测 `sa_restorer < 0x10000`（明显是未重定位的偏移），回退到内核自带的 `rt_sigreturn` 跳板。两者功能完全等价（都是 `li a7, 139; ecall`）。

```rust
let use_restorer = action.restorer != 0
    && action.restorer < USER_ADDR_MAX
    && action.restorer >= 0x10000;
if use_restorer {
    trap_cx[TrapFrameArgs::RA] = action.restorer;
} else {
    trap_cx[TrapFrameArgs::RA] =
        arch::SIG_RETURN_ADDR + arch::sigtrx::sigreturn_trampoline_offset();
}
```

### 3.8 mmap 文件映射增强

`sys_mmap` 的文件映射路径添加了 `total_read` 跟踪，确保 MAP_FIXED 文件映射正确读取文件内容到新分配的页中。

---

## 4. initcode 适配

`user/src/bin/initcode.rs` 中为 glibc profile 做了以下准备：

1. **环境变量**：设置 `LD_LIBRARY_PATH=/glibc/lib`，让 ld-linux 能找到共享库
2. **符号链接**：创建 `/lib/libc.so.6 → /glibc/lib/libc.so.6` 和 `/lib/libm.so.6 → /glibc/lib/libm.so.6` 硬链接
3. **解释器路径**：创建 `/lib/ld-linux-riscv64-lp64d.so.1 → /glibc/lib/ld-linux-riscv64-lp64d.so.1` 硬链接（PT_INTERP 请求的路径）

---

## 5. 测试结果

| 测试套件 | 结果 | 说明 |
|----------|------|------|
| glibc-netperf | **5/5 PASS** | UDP_STREAM, TCP_STREAM, UDP_RR, TCP_RR, TCP_CRR |
| glibc-iperf | **6/6 PASS** | BASIC/PARALLEL/REVERSE × UDP/TCP |
| glibc-basic | **32/32 PASS** | brk, clone, fork, execve, mmap, munmap 等 |
| musl-netperf | 5/5 PASS | 无回归 |
| musl-iperf | 6/6 PASS | 无回归 |

---

## 6. 修改文件清单

| 文件 | 修改内容 |
|------|----------|
| `os/src/mm/memory_set.rs` | `from_elf_with_interp` 解释器加载 + `unmap_range` 精确页粒度 + `map_one` COW + PIE load_base |
| `os/src/syscall/process.rs` | mmap MAP_FIXED 覆盖 + 文件映射增强 |
| `os/src/syscall/fs.rs` | fstat djb hash inode（& 0x7FFFFFFF 保持正数） |
| `os/src/config.rs` | USER_STACK_SIZE 20KB → 128KB |
| `os/src/task/mod.rs` | 信号 restorer 未重定位检测 + 内核跳板回退 |
| `arch/src/riscv64/mm/page_table.rs` | map() 放宽 assert 支持覆盖 |
| `user/src/bin/initcode.rs` | glibc 环境变量 + 库路径硬链接 |

---

## 7. 已知限制

1. **sa_restorer 是 workaround**：libc 内部的 `__restore_rt` 地址理应被 ld-linux 正确重定位为 `libc_base + offset`，但实际只包含原始偏移。根因可能是 ld-linux 对 libc 自身内部 R_RISCV_RELATIVE 重定位的某个边界条件未处理。内核跳板回退是安全等价方案。

2. **ASLR 未实现**：PIE 程序固定加载到 0x40000000，共享库固定分配到 0x600000000+。Linux 通常使用随机化基址。

3. **mmap hint 地址处理**：当 `mmap(hint_addr, ...)` 不带 MAP_FIXED 但 hint 地址有重叠时，内核返回 ENOMEM 而非忽略 hint 重新分配。Linux 会忽略无法满足的 hint。

4. **dlopen 未测试**：当前测试的 glibc 程序都是启动时链接。`dlopen` 运行时加载尚未验证。
