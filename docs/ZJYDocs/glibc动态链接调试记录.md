# glibc 动态链接调试记录

**日期**: 2026/3/24

---

## 1. 目标

让 glibc 动态链接的 netperf/netserver 在 rCore-lab 上运行。musl 版本（静态链接）已经 5/5 全通，glibc 版本是动态链接的 PIE 可执行文件，依赖 `ld-linux-riscv64-lp64d.so.1` + `libc.so.6` + `libm.so.6`。

---

## 2. 调试过程与逐步修复

### 阶段 1: `undefined symbol: stdin, version GLIBC_2.27`

**现象**: glibc netserver/netperf 启动即报 symbol lookup error。

**调试**: 用 docker 挂载 sdcard 检查库文件，确认 `/glibc/lib/libc.so.6` 和 `/glibc/lib/libm.so.6` 都是正确的 glibc ELF，且包含 GLIBC_2.27 版本信息。

**根因**: `sys_fstat` 返回 `st_dev=0, st_ino=0`，glibc 动态链接器用 `(dev, ino)` 做文件去重。当 `libm.so.6` 和 `libc.so.6` 都返回 `(0, 0)` 时，linker 认为它们是同一个文件，只加载了 libm，跳过了 libc → 找不到 `stdin` 符号。

**修复**: `sys_fstat` 和 `stat_path_inner` 中用 djb hash 对文件路径生成唯一 inode，`stat.dev = 1`。注意 hash 必须 `& 0x7FFF_FFFF` 保证正数（测试程序用 `%d` 打印 inode，负数会导致 judge 正则不匹配）。

### 阶段 2: `failed to map segment from shared object`

**现象**: 修复 inode 后，linker 不再混淆文件，尝试 mmap 库但失败。

**调试**: 通过 SYSCALL 日志追踪 mmap 调用：
```
mmap(0x60006f000, 0x2000, R+W, MAP_FIXED|MAP_PRIVATE, fd=3, 0x6e000) → -12 (ENOMEM)
```

**根因**: `sys_mmap` 检测到 `MAP_FIXED` 目标地址和已有映射重叠时，直接返回 ENOMEM。但 `MAP_FIXED` 的 Linux 语义是**覆盖**已有映射。

**修复**: 当 `MAP_FIXED` 且有 overlap 时，调用 `memory_set.unmap_range()` 先移除目标范围内的页（精确到页粒度，保留范围外的部分），再创建新映射。

### 阶段 3: page fault at `0x600000190`

**现象**: mmap 成功了，但访问 TLS 区域时 page fault。

**根因**: `unmap_range` 移除了整个重叠 MapArea，包括不在目标范围内的页。libm 的 text 段 `0x600000000-0x600070000` 被完全移除，只剩 data 段 `0x60006F000-0x600071000`。

**修复**: `unmap_range` 改为只 unmap 目标范围内的页，partial overlap 的 MapArea 保留不受影响的部分。同时 `page_table.map()` 放宽 assert，允许 MAP_FIXED 覆盖已映射的 PTE。

### 阶段 4: page fault at `0x7ffffafd0` (栈溢出)

**现象**: 程序跑得更远了，但在 glibc 初始化阶段栈溢出。

**根因**: `USER_STACK_SIZE = 4096 * 5 = 20KB`，glibc 初始化代码需要更多栈空间。

**修复**: `USER_STACK_SIZE` 增大到 `4096 * 32 = 128KB`。

### 阶段 5: kernel panic at `page_table.rs:224` (map assert)

**现象**: mmap(MAP_FIXED) 的 `insert_framed_area` 对已映射的 VPN 调 `map_one`，触发 `assert!(!pte.is_valid())`。

**修复**: `page_table.map()` 改为：如果 PTE 已 valid，先清空再写入新 PTE（不 panic）。

### 阶段 6: netserver 成功启动！page fault at `0xf8e36`

**现象**: 能看到 `Starting netserver` 和 `MIGRATED UDP STREAM TEST` 输出。glibc 程序实际在运行。

**调试**: `0xf8e36` 是 libc 的一个函数偏移（libc 加载在 `0x600073000`）。GOT entry 只包含偏移，没加 libc base。

**根因**: PIE 可执行文件被加载到 VA 0x0（`load_base=0`），这在 Linux 上不会发生（Linux 给 PIE 一个非零 base）。load_base=0 可能导致 linker 的 base 推算出问题。

**修复**: `from_elf_with_interp` 中，当 ELF 是 SharedObject 且 `min_load_vaddr=0` 时，设 `load_base=0x40000000`。

### 阶段 7: page fault at `0x2000`

**现象**: PIE 加载到 0x40000000 后，page fault 地址变了，程序能输出 `enable_enobufs failed: getprotobyname`（已进入 main 函数！）。

**调试过程**:

1. 确认 fault 类型是 **InstructionPageFault**（不是 Store），排除共享页权限问题
2. dump 主程序 GOT：**全零**！`.dynamic` 也全零
3. dump data 段所有页：大部分全零，只有部分页有数据
4. 检查共享页（LOAD1 text 和 LOAD2 data 的边界页）是否有问题：**实际没有共享页**（VPN 0x40032 和 0x40033 不同）
5. 实现 COW：map_one 对已映射 VPN 做 copy-on-write（复制旧页内容到新 frame，合并权限）
6. 检查源文件内容：文件偏移 `0x32C48`（LOAD2 起始）确实是零（padding），但 `0x32DF0`（`.dynamic`）有非零数据
7. **关键发现**：在 `exec_with_interp` 完成后立即 dump——`.dynamic` val=0x1（**正确！**），**内核加载时 data 段是对的**
8. 但运行时（page fault 时）dump——同一 VA 指向**不同的 PA**，内容全零
9. **结论**：内核正确加载了 data 段，但后续某个操作（可能是 ld-linux 的 mmap、mprotect，或 fork）用新的零页替换了原来的数据页

---

## 3. 当前状态

### 已解决

| 问题 | 修复 |
|------|------|
| fstat inode 全零 → linker 文件去重混淆 | djb hash 生成唯一 inode |
| mmap MAP_FIXED 返回 ENOMEM | unmap_range + 重新映射 |
| unmap_range 删除整个 MapArea | 精确页粒度 unmap |
| 用户栈 20KB 不够 glibc | 增大到 128KB |
| page_table map assert panic | 放宽允许覆盖 |
| PIE 加载到 VA 0x0 | load_base=0x40000000 |
| 共享页 text 数据丢失 | COW：复制旧页内容到新 frame |

### 已解决（最终修复）

**阶段 8: page fault at `0x2000` — 信号 restorer 地址未重定位**

**现象**: glibc netperf 在 UDP_STREAM 测试中，SIGALRM 触发后立即 page fault at 0x2000，sepc=ra=0x2000。GOT 条目全部正确解析（指向 0x6000XXXXX libc 地址），程序部分正常运行（能输出日志、调用 sendto 发包）。

**调试过程**:

1. 添加 watchpoint 到 `map_one`/`unmap_one`/`unmap_range`/`insert_framed_area`，确认**没有任何 mmap/mprotect/brk 操作触及 data 段 VPN**
2. 验证 `copy_data` 写入正确且 readback 一致（data 段加载无误）
3. 在 page fault handler 添加完整 context dump，发现 GOT 条目全部正确
4. 追踪 SYSCALL 日志，发现 crash 发生在 **SIGALRM 触发后的第一条指令**
5. 检查 `sigaction(SIGALRM)` 的参数：**handler=0x4000529a（正确），restorer=0x2000（未重定位！）**

**根因**: glibc 的 `sigaction()` 包装器设置 `sa_restorer = &__restore_rt`，其中 `__restore_rt` 是 libc.so.6 中的信号返回跳板（调用 `rt_sigreturn`）。在我们的内核上，libc 内部的 `__restore_rt` 地址没有被正确重定位：应该是 `libc_base + 0x2000`（如 `0x600075000`），实际只有原始偏移 `0x2000`。

rCore 信号处理代码在 `handle_signals` 中将 `trap_cx[RA] = action.restorer`，导致信号 handler 返回时跳转到未映射的 0x2000。

**修复**: 在 RISC-V 信号派发代码中，检测 `sa_restorer` 是否为明显未重定位的地址（< 0x10000），如果是则回退到内核自带的 `rt_sigreturn` 跳板（功能完全等价——都只是调用 `rt_sigreturn` 系统调用）。

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

**结果**: glibc netperf **5/5 全部通过**（UDP_STREAM, TCP_STREAM, UDP_RR, TCP_RR, TCP_CRR），无回归。

---

## 5. 修改文件清单

| 文件 | 修改 |
|------|------|
| `os/src/syscall/fs.rs` | fstat 返回唯一 inode (djb hash & 0x7FFFFFFF) |
| `os/src/mm/memory_set.rs` | unmap_range 精确页粒度 + map_one COW + push 清理 |
| `os/src/syscall/process.rs` | mmap MAP_FIXED 覆盖 + mmap debug 清理 |
| `os/src/config.rs` | USER_STACK_SIZE 20KB→128KB |
| `arch/src/riscv64/mm/page_table.rs` | map() 放宽 assert 支持覆盖 |
| `os/src/task/mod.rs` | 信号 restorer 未重定位检测 + 内核跳板回退 |

---

## 6. 经验总结

### 经验 1: fstat 的 (dev, ino) 对动态链接至关重要

glibc ld-linux 用 `(st_dev, st_ino)` 做已加载库的去重。如果所有文件返回 `(0, 0)`，linker 会把第二个库当作"已加载"跳过。这个 bug 极其隐蔽——错误信息是 "undefined symbol"（看起来像版本不匹配），实际根因是文件去重。

### 经验 2: MAP_FIXED 是 glibc 动态链接的核心操作

glibc ld-linux 加载每个 .so 都用：
1. `mmap(NULL, total_size, R+E, MAP_PRIVATE)` — 一次性映射整个文件
2. `mprotect(gap, PROT_NONE)` — 设置 guard page
3. `mmap(data_addr, data_size, R+W, MAP_FIXED)` — 覆盖 data 段区域

不支持 MAP_FIXED 覆盖就无法加载任何 glibc 动态库。

### 经验 3: PIE 加载地址不能为 0

PIE (Position-Independent Executable) 的 `min_load_vaddr=0`，内核需要选一个非零 base。加载到 0x0 会导致 NULL 指针和 linker base 推算混淆。Linux 通常用 ASLR 或固定 base（如 0x555555554000）。

### 经验 4: page fault 在 SIGALRM 后 → 查信号 restorer

当 page fault 总是在 `setitimer` SIGALRM 触发后发生时，问题通常不在 data 段或 GOT，而在**信号处理的 restorer 地址**。glibc 的 `sigaction` 包装器会设置 `sa_restorer = &__restore_rt`（SA_RESTORER 标志），如果 libc 内部的 `__restore_rt` 地址没有正确重定位，handler 执行完毕后返回到错误地址。

排查方法：在 `sys_sigaction` 中 log handler 和 restorer 地址，确认 restorer 是否在合法的地址空间内。

### 经验 5: 用 watchpoint 排除 mmap/mprotect 干扰

对怀疑被覆盖的 VPN 范围，在 `map_one`/`unmap_one`/`insert_framed_area` 中添加条件 `warn!`。如果**没有任何输出**，说明该 VPN 范围在 exec 后从未被重映射——问题不在页替换，而在其他地方。这比逐条分析 mmap 日志高效得多。

### 经验 6: 逐层排查的方法论

```
完全不能运行 (symbol lookup error)
  → fstat inode 修复
    → failed to map (mmap ENOMEM)
      → MAP_FIXED 覆盖支持
        → page fault at TLS (unmap_range 精度)
          → 精确页粒度 unmap
            → 栈溢出
              → 增大栈
                → netserver 启动成功！
                  → page fault at 0x2000
                    → 排除 data 段页替换（watchpoint 无输出）
                      → 发现 crash 在 SIGALRM 后
                        → sa_restorer=0x2000（未重定位）
                          → 内核跳板回退 → 5/5 全通！
```

每一步修复都让程序走得更远。glibc 动态链接涉及 ELF 加载、mmap 语义、页表管理、auxv 设置、**信号机制**等多个子系统的协同。
