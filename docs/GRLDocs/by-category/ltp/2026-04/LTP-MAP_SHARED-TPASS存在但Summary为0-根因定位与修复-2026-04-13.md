# LTP `TPASS` 已打印但 `Summary passed=0` 调试与修复记录

日期：2026/04/13  
分支：`tests/ltp_with_mm_improve_all-2`

## 1. 问题现象

在 LTP 执行中出现两类异常：

1. 用例正文里已经出现 `TPASS`，但 `Summary` 统计为 `passed 0`，并伴随 `TBROK: Test haven't reported results!`。  
2. 部分场景还出现过明显异常的大数统计（后续已先修复）。

典型失败日志（修复前）：

- 文件：`/tmp/ltp_after_chmod06.log`
- 现象片段：
  - `execl01_child.c:20: TPASS: execl01_child executed`
  - `tst_test.c:1449: TBROK: Test haven't reported results!`
  - `Summary: passed 0, broken 1`

这说明测试子进程能正常执行并打印，但汇总进程没有读到结果计数。

## 2. 根因分析

## 2.1 第一阶段根因（已修）

`MAP_SHARED` 文件映射在 `sys_mmap` 的 lazy 路径中，没有保证 fork 后仍共享同一组已物化页，导致统计区可能被当作私有页处理。  
这会造成“子进程写计数，父进程看不到”的症状。

对应修复：`os/src/syscall/process.rs` 中对文件 `MAP_SHARED` 改走 `insert_shared_file_mmap_area()`，避免仅靠 lazy fault 的分离物化。

## 2.2 第二阶段根因（本次关键）

第一阶段后，仍有 `execl01` 类场景出现 “`TPASS` 存在但 `passed=0`”。

进一步定位发现：即便都走 `MAP_SHARED`，不同进程/不同映射若各自从文件重新分配页框，也可能拿到不同物理页，导致共享语义仍被破坏。  
本质是缺少“按文件页维度的全局共享页复用”。

## 2.3 与 chronix-retest 的对齐点

参考仓库：`/home/grl/codeRepo/oskernel2025-chronix-retest`  
关键实现：`os/src/mm/vm/uvm.rs` 的 `map_shared_file()` 直接将共享文件页映射到同一底层页（`inode.read_page_at(offset)` + 共享 frame）。

我们的修复方向与其一致：`MAP_SHARED` 文件页必须跨进程/跨映射复用同一页，而不是每次独立分配。

## 3. 修复方案

## 3.1 全局共享文件页缓存（核心）

在 `os/src/mm/memory_set.rs` 引入：

- `SHARED_FILE_PAGE_CACHE: BTreeMap<(file_id, page_offset), Arc<FrameTracker>>`
- 通过 `get_or_alloc_shared_file_page()` 复用/分配共享页
- 在 `insert_shared_file_mmap_area()` 里映射缓存页，确保多个映射命中同一 frame

这样子进程和父进程会看到同一份共享统计页，LTP 汇总不再丢失。

## 3.2 截断路径失效处理

为避免 `truncate` 后复用到旧页内容，增加缓存失效：

- `os/src/syscall/fs.rs`
  - `sys_ftruncate`
  - `sys_truncate`
- `os/src/fs/vfs/file.rs`
  - `open(..., O_TRUNC)` 分支
- `os/src/mm/mod.rs`
  - 导出 `invalidate_shared_file_pages_by_path`

## 3.3 `sys_mmap` 路径统一

`os/src/syscall/process.rs` 中：

- 文件 `MAP_SHARED`：走 `insert_shared_file_mmap_area()`
- 其他文件映射：保持 `insert_lazy_file_area()`

确保共享映射和私有映射路径语义清晰分离。

## 4. 验证结果

## 4.1 `execl01`（原始问题点）

日志：`/tmp/ltp_after_execl01_fix.log`

- `RUN LTP CASE execl01`（line 8376）
- `execl01_child.c:20: TPASS`（line 8382）
- `Summary: passed 1, broken 0`（line 8384-8387）
- `FAIL LTP CASE execl01 : 0`（line 8390）

结论：不再出现 “`TPASS` 已打印但 `Summary passed=0`”。

## 4.2 `access04`（历史统计异常点）

日志：`/tmp/ltp_after_access04_fix2.log`

- `RUN LTP CASE access04`（line 8376）
- `Summary: passed 12, broken 0`（line 8396-8399）
- `FAIL LTP CASE access04 : 0`（line 8402）

结论：统计恢复正常，未见异常大数计数。

## 5. 修改清单

- `os/src/mm/memory_set.rs`
- `os/src/mm/mod.rs`
- `os/src/syscall/fs.rs`
- `os/src/fs/vfs/file.rs`
- `os/src/syscall/process.rs`

## 6. 后续建议

1. 增加内核侧回归用例：`MAP_SHARED + fork + 子写父读 + exec`。  
2. 将“共享文件页复用”作为 `mmap` 语义检查项，避免未来 demand paging 重构回归。  
3. 后续可补 `msync/munmap` 脏页回写一致性测试，完善 `MAP_SHARED` 持久化语义。
