# libcbench 测试适配调试记录

日期: 2026/3/26

---

## 1. 背景

libcbench（libc-bench）是一个 C 标准库性能基准测试套件，包含 **27 个子测试**，覆盖以下领域：

| 类别 | 测试数量 | 测试项 |
|------|---------|--------|
| malloc | 8 | sparse, bubble, tiny1/2, big1/2, thread_stress, thread_local |
| string | 8 | strstr×5, memset, strchr, strlen |
| pthread | 4 | createjoin_serial1/2, create_serial1, uselesslock |
| utf8 | 2 | bigbuf, onebyone |
| stdio | 2 | putcgetc, putcgetc_unlocked |
| regex | 3 | compile, search×2 |

每个子测试在 fork 出的子进程中运行，子进程内通过 `clock_gettime(CLOCK_REALTIME)` 计时，输出格式为：
```
b_xxx (params)
  time: X.XXXXXXX, virt: 0, res: 0, dirty: 0
```

评测脚本（`judge_libcbench-musl.py` / `judge_libcbench-glibc.py`）解析 time 字段，与 baseline 对比计算得分。每项满分 2.0，公式 `score = 2 - baseline/result`（result < baseline 时 score=1.0）。

### sdcard 上的测试入口

```
/musl/libcbench_testcode.sh    →  cd /musl && ./libc-bench
/glibc/libcbench_testcode.sh   →  cd /glibc && ./libc-bench
```

`libc-bench` 二进制为**静态链接**（musl 版本），glibc 版本为动态链接。

---

## 2. 初始状态（feature/cow-libcbench 基线 libcbench_cow1.log）

在 COW fork 实现之后的首次运行中，存在以下问题：

| 问题 | 影响测试 | 现象 |
|------|---------|------|
| madvise 未实现（syscall 233） | 全部 | 大量 `[ERROR] unimplemented syscall 233` 日志刷屏 |
| pthread 页错误 | createjoin_serial1/2, create_serial1 | `page fault addr=0x10 sepc=0x1c74c` → SIGSEGV |
| regex 栈溢出 | regex_search×2 | `page fault addr=0x7fffc05d0` 超出用户栈下界 |
| stdio 极慢 | putcgetc, putcgetc_unlocked | 29s/38s vs baseline 0.77s（约 40 倍慢） |

musl 结果：24/27 有输出（3 个 pthread 崩溃，2 个 regex 崩溃 = 5 个无输出），但 stdio 极慢拖分。

---

## 3. 已完成的修复

### 3.1 madvise stub（syscall 233）

**改动**：`os/src/syscall/mod.rs`

- 添加 `const SYSCALL_MADVISE: usize = 233;`
- 在 dispatch match 中添加 `SYSCALL_MADVISE => 0`（直接返回成功）

**原理**：`madvise` 是内存使用提示（如 `MADV_DONTNEED`），对于简单内核只需忽略即可。musl 的 malloc 在 free 时调用 `madvise(MADV_DONTNEED)` 回收页面，返回 -ENOSYS 不影响正确性但会刷大量 ERROR 日志。

**效果**：消除了几百行 ERROR 日志，测试输出更干净。

### 3.2 用户栈增大（128KB → 512KB）

**改动**：`os/src/config.rs`

```rust
// 修改前
pub const USER_STACK_SIZE: usize = 4096 * 32;  // 128KB
// 修改后
pub const USER_STACK_SIZE: usize = 4096 * 128; // 512KB
```

**原理**：`b_regex_search` 使用 POSIX regex 引擎，内部通过递归实现 NFA 匹配。对于复杂正则表达式（如 `(a|b|c)*d*b`），递归深度可达数千层，需要大量栈空间。原来 128KB 的用户栈不足，导致 `page fault addr=0x7fffc05d0`（栈底以下约 130KB）。

**注意事项**：
- 不能设为 1MB，否则 glibc 动态链接场景在 128MB RISC-V QEMU 下会 OOM（`frame_alloc` panic）
- 512KB 是在 regex 能通过与 glibc 不 OOM 之间的平衡点
- 同时将 `frame_alloc().unwrap()` 改为 graceful error，避免内核 panic

**效果**：`b_regex_search` 两个测试从 SIGSEGV 变为正常输出（time: 0.067s / 0.202s）。

### 3.3 munmap 修复（使用 unmap_range 替代 remove_area_with_start_vpn）

**改动**：`os/src/syscall/process.rs` 的 `sys_munmap`

```rust
// 修改前：只按 VMA 起始地址精确匹配删除
inner.memory_set.remove_area_with_start_vpn(VirtAddr(start).floor());

// 修改后：按范围删除，支持部分重叠
let aligned_len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
inner.memory_set.unmap_range(VirtAddr(start), VirtAddr(start + aligned_len));
```

**原理**：`remove_area_with_start_vpn` 只有在 munmap 的起始地址恰好等于某个 VMA 的起始地址时才会删除该 VMA。但 musl 的 `pthread_join` 调用 `munmap(map_base, map_size)` 时，map_base 可能位于 VMA 中间（因为 musl 在 mmap 区域开头放了 guard page，然后给 munmap 传的是 guard page 之后的地址）。使用 `unmap_range` 可以正确处理这种情况。

**效果**：修复了 mmap 地址空间泄漏（之前每个线程栈 munmap 后不会真正释放，地址单调递增）。

---

## 4. 未解决的问题

### 4.1 pthread createjoin/create 崩溃（3 个测试）

**现象**：`b_pthread_createjoin_serial1/2` 和 `b_pthread_create_serial1` 在第一次创建线程时就崩溃：

```
page fault addr=0x10 sepc=0x1c74c ra=0x1c6f8 sp=0x601df0e08 tp=0x601df0fe8
```

**关键发现**：

1. **tp 寄存器正确**：崩溃时 `tp=0x601df0fe8`（非零，是 CLONE_SETTLS 设置的值），排除了 TLS 未设置的可能。

2. **崩溃位置**：反汇编 `libc-bench` 二进制在 `sepc=0x1c74c`：
   ```asm
   1c6bc:  mv s0, tp              # 函数用 tp 作为 frame pointer
   1c6f8:  ld a4, -200(s0)        # 加载 pthread 结构体中的链表指针
   1c6fc:  bne a4, s2, 0x1c734    # 如果不等于哨兵值，进入 unlink 路径
   1c74c:  sd a5, 16(a4)          # CRASH: a4=0 → *(NULL+16) = a5
   ```

3. **根因分析**：这是 musl 的 `__pthread_exit` 函数中的线程链表 unlink 操作。字段 `*(tp-200)` 是线程的 `prev` 指针，值为 NULL。正常情况下，musl 的 `pthread_create` 在调用 `clone` **之前**就会将新线程链入全局线程链表（设置 prev/next），因此 `__pthread_exit` 中的 prev 应该非空。

4. **在 clone 时就已经是 0**：内核侧在 `sys_clone` 中读取 `*(tp-200)` 发现对所有线程（包括成功的）都是 0。但成功的线程在启动后通过 musl 的 `start()` 函数修改了该值，而崩溃的线程在 `start()` 执行 unlink 路径时该值仍为 0。

5. **可能的深层原因**：
   - musl 的 `pthread_create` 在 clone 之前链入线程链表，但写入发生在父进程的用户空间
   - 这些写入需要通过 COW 机制处理（因为整个进程是 fork 出来的）
   - **COW + 原子操作（lr/sc）交互问题**：musl 的 `__tl_lock` 使用 RISC-V lr.w/sc.w 原子指令。如果 lr/sc 操作的页面是 COW 只读页面，sc 会因为页面只读而失败，触发 COW fault。但 RISC-V 规范中，COW fault 会破坏 lr/sc 的 reservation，导致 sc 在重试时仍然失败
   - 另一个可能：munmap 修复后 VMA 被拆分/部分删除，导致后续 mmap 返回的区域与旧 VMA 重叠

**状态**：需要进一步调试。可考虑的方向：
- 检查 COW fault 路径是否正确处理 lr/sc 的 store page fault（scause 应为 StorePageFault）
- 在 `handle_cow_fault` 中添加日志，确认 pthread struct 所在页面的 COW 是否正常
- 检查 `unmap_range` 对 VMA 的拆分是否导致了 data_frames 丢失

### 4.2 stdio putcgetc 极慢（2 个测试）

**现象**：`b_stdio_putcgetc` 耗时 29s（baseline 0.77s，约 38 倍慢），`b_stdio_putcgetc_unlocked` 耗时 34s（baseline 0.77s）。

**原因分析**：
- 该测试调用 `tmpfile()` 创建临时文件，然后 putc/getc 100 万次
- musl 的 `tmpfile()` 先尝试 `memfd_create`（我们未实现，返回 -ENOSYS），然后 fallback 到 `open("/tmp/tmpXXXXXX")` 在 ext4 文件系统上创建文件
- 每次 write 约 1024 字节（musl BUFSIZ），100 万字符 ≈ 1000 次 write syscall
- 每次 write 经过 ext4 → lwext4 → VirtIO block → QEMU 后端，单次 I/O 延迟极高

**可能的优化方向**：
- 实现 `memfd_create` syscall（sys_memfd_create），返回一个内存文件 fd，让 musl 的 tmpfile 走内存路径而非磁盘
- 实现简单的 tmpfs/ramfs 挂载到 `/tmp`
- 增大 pipe buffer 无效（tmpfile 不用 pipe）

**状态**：暂未修复。影响 2/27 的测试得分。

---

## 5. 当前测试结果汇总（libcbench5.log，musl 部分）

| 测试 | 结果 | 耗时 | Baseline | 状态 |
|------|------|------|----------|------|
| b_malloc_sparse | 0.300s | 0.385s | PASS（faster） |
| b_malloc_bubble | 0.277s | 0.360s | PASS（faster） |
| b_malloc_tiny1 | 0.009s | 0.014s | PASS（faster） |
| b_malloc_tiny2 | 0.007s | 0.011s | PASS（faster） |
| b_malloc_big1 | 0.181s | 0.119s | PASS（slower） |
| b_malloc_big2 | 0.177s | 0.107s | PASS（slower） |
| b_malloc_thread_stress | 0.082s | 0.096s | PASS（faster） |
| b_malloc_thread_local | 0.081s | 0.097s | PASS（faster） |
| b_string_strstr ×5 | 0.014-0.019s | 0.011-0.021s | PASS |
| b_string_memset | 0.009s | 0.025s | PASS（faster） |
| b_string_strchr | 0.015s | 0.013s | PASS |
| b_string_strlen | 0.014s | 0.013s | PASS |
| b_pthread_createjoin_serial1 | SIGSEGV | - | **FAIL** |
| b_pthread_createjoin_serial2 | SIGSEGV | - | **FAIL** |
| b_pthread_create_serial1 | SIGSEGV | - | **FAIL** |
| b_pthread_uselesslock | 0.054s | 0.081s | PASS（faster） |
| b_utf8_bigbuf | 0.062s | 0.037s | PASS（slower） |
| b_utf8_onebyone | 0.104s | 0.110s | PASS（faster） |
| b_stdio_putcgetc | 29.9s | 0.768s | PASS（极慢） |
| b_stdio_putcgetc_unlocked | 34.8s | 0.765s | PASS（极慢） |
| b_regex_compile | 0.051s | 0.088s | PASS（faster） |
| b_regex_search (1) | 0.067s | 0.083s | PASS（faster） |
| b_regex_search (2) | 0.202s | 0.286s | PASS（faster） |

**musl 总结**：24/27 有 time 输出（3 个 pthread FAIL），其中 22 个性能达标或超标，2 个 stdio 极慢。

### glibc 部分

glibc libcbench 运行到 `b_malloc_big2` 后触发 OOM panic（`frame_alloc` 返回 None）。原因是 glibc 动态链接的内存开销 + 512KB 用户栈 + COW fork 占用了大量物理页帧。

在将 `frame_alloc().unwrap()` 改为 graceful error 后，不会 panic 但可能导致测试失败。glibc libcbench 的完整通过需要进一步优化内存使用（如：惰性栈分配、减少 COW 拷贝开销）。

---

## 6. 分支与代码改动总览

**分支**：`feature/libcbench`（从 `dev` 签出）

**改动文件**：

| 文件 | 改动 |
|------|------|
| `os/src/syscall/mod.rs` | 添加 SYSCALL_MADVISE 常量和 dispatch |
| `os/src/config.rs` | USER_STACK_SIZE: 128KB → 512KB |
| `os/src/syscall/process.rs` | sys_munmap 使用 unmap_range 替代精确匹配；clone debug 日志 |
| `os/src/mm/memory_set.rs` | frame_alloc OOM graceful error |
| `os/src/trap/user_trap_riscv64.rs` | page fault 日志增加 tp 寄存器值 |
| `user/src/bin/initcode.rs` | TEST_SUITES 添加 "libcbench" |

---

## 7. 后续计划

1. **pthread 崩溃**（优先级高）：
   - 在 `handle_cow_fault` 中加日志，确认 pthread_create 写入 pthread struct 时 COW 是否正确触发
   - 检查 lr/sc 原子操作遇到 COW 只读页面时的行为
   - 考虑在 fork 后预热（eagerly COW）线程链表所在的页面

2. **stdio 性能**（优先级中）：
   - 实现 `sys_memfd_create` 让 musl tmpfile 走内存文件
   - 或者实现简单的 `/tmp` tmpfs

3. **glibc OOM**（优先级中）：
   - 用户栈改为按需分配（lazy allocation）而非一次性映射
   - 优化 COW fork 的页帧回收

4. **评分验证**：
   - 使用 `judge_libcbench-musl.py` 和 `judge_libcbench-glibc.py` 评分
   - 当前预期 musl 得分约 22×1.0~2.0 分（22 个通过项 + 2 个极慢项得低分 + 3 个 FAIL 得 0 分）
