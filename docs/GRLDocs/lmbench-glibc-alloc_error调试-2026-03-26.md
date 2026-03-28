# lmbench glibc `alloc_error` 调试 alloc_error

日期：2026/3/26

---

## 一、先给结论（罪魁祸首）

本次 `alloc_error` 的**直接罪魁祸首**不是任务泄漏、不是 `fork` 的 `fd_table` 复制，也不是 COW 元数据在 `fork` 内暴涨，而是：

- 在 `exec` 路径中，对 `/glibc/lmbench_all` 这类较大的 ELF 文件执行了 `read_all()`；
- `read_all()` 使用 `Vec<u8>` 从空缓冲不断 `extend_from_slice`，触发几何扩容；
- 对约 `8.45 MiB` 的文件，容量扩容过程中出现一次 `16 MiB` 的申请；
- 内核堆在该时刻无法满足这次 `16 MiB` 申请，于是进入 `alloc_error_handler` 并 panic。

也就是说，**崩溃发生在 exec 装载大 ELF 的“临时整文件缓冲”阶段**，而非“系统运行时间长导致碎片化”这一类慢变量。

---

## 二、关键现象与证据链

### 1）原始现象（用户提供日志）

在 `all-la-lmbench.log` 中，`musl` 套件完成后，进入 `glibc lmbench` 立即报错：

- `alloc_error diag: pid2pcb_len=3 ready_queue_len=2 timer_len=0 live_tasks=3`
- 随后 `Panicked at src/mm/heap_allocator.rs:64`

这里第一眼可知：

- `live_tasks=3`，`pid2pcb_len=3`，并没有“线程/进程数量失控”的证据；
- 属于一次性峰值型失败，而不是长期累积型泄漏。

### 2）先用 SYSCALL 级日志缩小触发窗口

我用 glibc-only 复现（避免 musl 干扰），并观察崩溃前最后事件：

- 先看到：`[clone] flags=0x1200011 clone_flags=0x1200000 ...`
- 紧接着进入 `alloc_error`

这一步很容易让人误判为 “clone/fork 本身导致内存打爆”。因此我继续做阶段打点。

### 3）对 `fork()` 逐阶段打点（排除法）

我在 `fork` 的关键阶段逐条打点：

- before/after `memory_set` clone
- before/after `fd_table` clone
- child pcb 分配
- child task 分配
- 插入 `pid2process`
- 加入 ready queue

结果显示：

- 上述所有 `fork-stage` 打点全部成功打印；
- 说明 `fork()` 这条路径本身是跑完了的；
- `alloc_error` 出现在 `fork` 返回后不久的后续路径。

同时我额外统计了 `fd_table` 槽位：

- `total_fd_slots=18`，`max_fd_slots=6`

这直接排除了“`RLIMIT_NOFILE` 拉大后复制 `fd_table` 触发 16MiB 大申请”的猜想。

### 4）在 `alloc_error` 中打印 layout（决定性证据）

在 `alloc_error_handler` 打出布局后，得到：

- `alloc_error layout: size=16777216 align=1`

即失败申请是 `16 MiB`，且不是高对齐大对象（`align=1`）。这更像是 `Vec<u8>` 扩容申请大连续块。

### 5）在 `read_exec_image()` 中增加“大文件日志”

随后我在 `sys_exec` 的读取路径增加日志：

- 当读取文件尺寸超过 `8 MiB` 时打印路径与尺寸。

结果在崩溃前看到：

- `[exec-image] large file path=/glibc/./lmbench_all size=8453456 bytes`
- 紧接着：`alloc_error layout: size=16777216 align=1`

这组先后顺序几乎是“锁死证据”：

1. 正在加载 `8.45 MiB` 的 `lmbench_all`；
2. 随后出现 `16 MiB` 申请失败；
3. 与 `Vec` 扩容阶梯（大致倍增）高度一致。

因此可以认定：

> `exec` 的整文件读取缓冲是本次 `alloc_error` 的触发点。

---

## 三、为什么这条链条必然会炸（机制解释）

当前 rcore-lab 路径里，`sys_exec` 会调用 `read_exec_image()`，再走到底层 `read_all()`：

- `read_all()` 从 `Vec::new()` 开始，按 4KiB/512B 等小块不断 `extend`；
- 容量增长策略一般是几何增长，不是按“文件精确尺寸”一次到位；
- 当文件略大于 8MiB 时，很容易出现一次跳到 16MiB 的容量申请；
- 这次申请需要堆提供连续大块，失败即触发 `alloc_error_handler`。

注意这里并不要求“内核堆已满”。只要：

- 当前可用连续块不足 16MiB，或者
- 同时有其它分配占用导致大块不可得，

就会失败。

这解释了为什么日志中任务数很少，仍然会崩：

- 不是对象数量太多，而是**单次申请过大**。

---

## 四、Chronix（oskernel2025-chronix-retest）是怎么避免这个问题的

这里给出对比结论：Chronix 的核心思路是**避免“整文件读入内核堆”**，改为“文件映射 + 按页缺页加载”。

### 1）`sys_execve` 入口不是 `read_all`，而是 `FileReader`

在 Chronix 的 `sys_execve` 中：

- 打开目标文件后，创建 `FileReader::new(app.clone())`；
- 再把 `FileReader` 作为 `xmas_elf::ElfFile::new(&reader)` 的 Reader。

这意味着 ELF 解析读的是 Reader 视图，而不是整块 `Vec<u8>`。

### 2）`FileReader` 的实现是“内核地址空间 mmap + 按需触页”

`FileReader` 关键逻辑：

- `KVMSPACE.lock().mmap(file)` 获得一个映射虚地址；
- `len` 来自 inode 的 `st_size`；
- 在 `Reader::read(offset, len)` 内，先检查页是否已映射；
- 未映射则调用 `handle_page_fault(...READ)` 逐页拉入；
- 返回的是该地址区间 slice 引用。

这套机制不会在 exec 时额外申请一个“8~16MiB 的临时 Vec 缓冲”。

### 3）`map_elf` 把 PT_LOAD 段变成 file-backed VMA

Chronix 的 `UserVmSpace::map_elf()` 并不把整个文件复制到堆：

- 它按 program header 遍历 PT_LOAD 段；
- 给每段建 `UserVmArea`，记录 `file/offset/len`；
- 后续访问该页才在 page fault handler 中真正装页。

因此峰值内存模型从：

- “exec 瞬间整文件内存化”

变为：

- “按需页粒度装入（通常 4KiB）”。

### 4）Page Fault Processor 进一步把粒度压到页

Chronix 在 `UserDataHandler/UserMmapHandler` 中：

- 通过 `inode.read_page_at(offset)` 读取单页；
- private/shared 分支分别映射页框；
- 对未覆盖部分填零或映射零页。

这使得“单次分配上限”通常是页级，不会轻易触发 16MiB 连续申请。

---

## 五、两套实现的差异（本问题相关）

### rcore-lab（当前）

- `exec` 路径在早期就把 ELF 整体读成 `Vec<u8>`；
- 大文件必然走到大容量扩容；
- 一旦扩容目标块过大（本例 16MiB），就有概率触发堆分配失败。

### Chronix（规避）

- `exec` 解析阶段走 `Reader` 接口，底层是 file-backed 映射；
- 数据以页为单位在 fault 时拉取；
- 不制造巨型临时缓冲，规避了“扩容跃迁型 OOM”。

一句话总结：

> Chronix 不是“堆更大”，而是“加载模型更稳”，把内存峰值从“文件级”降到“页级”。

---

## 六、对 rcore-lab 的修复建议（按优先级）

### 建议 A（最小改动、短期可落地）

把 `read_all()` 改成“按已知文件大小预留容量”，减少扩容跳跃。

- 例如先取 inode size，再 `Vec::with_capacity(size)`；
- 这样不会在 8.45MiB 文件上跳到 16MiB 的几何扩容峰值。

优点：改动小，易合并。
缺点：仍是整文件读入，峰值仍是文件级。

### 建议 B（中期正确方向）

在 `exec` 路径引入 `Reader` 抽象，按段/按页读取，避免整文件缓冲。

- 可参考 Chronix 的 `FileReader + map_elf + lazy fault` 组合；
- 先从主 ELF 做起，再平移到 interp/ldso。

优点：从机制上消除这类问题。
缺点：改动范围较大，需要回归更多 case（动态链接、脚本 shebang、mmap 语义）。

### 建议 C（诊断保留）

保留当前新增诊断（`layout.size/align`、`fork-stage`、`exec-image`）一段时间，便于回归对照；稳定后再降噪。

---

## 七、最终结论

本次 `lmbench-glibc` 的 `alloc_error`，根因已经明确：

- 触发点在 `exec` 阶段读取大 ELF（`/glibc/lmbench_all`）；
- 失败申请为 `16MiB`，与 `Vec<u8>` 扩容峰值吻合；
- `fork` 只是时间上紧邻，不是根因（阶段打点已排除）。

Chronix 能规避同类风险的关键不在“运气/堆大小”，而在**架构选择**：

- 用 file-backed + lazy page-fault 的加载模型，避免整文件临时缓冲。

这也是后续 rcore-lab 想稳定跑大程序（特别是 glibc 系列）时最值得迁移的设计点。
