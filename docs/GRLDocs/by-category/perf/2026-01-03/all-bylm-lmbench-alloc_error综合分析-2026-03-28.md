# all-bylm.log：lmbench `lat_ctx` 期间 `alloc_error` 综合分析（2026-03-28）

## 1. 背景与本次要回答的问题

日志文件：`/home/grl/codeRepo/rcore-lab/all-bylm.log`  
场景：LA 架构跑 glibc/musl + busybox/iozone/lmbench 的综合测试，最终在 lmbench 阶段崩溃。  

本次聚焦回答以下问题：

1. 根因是不是“内存泄漏”？
2. 为什么 `alloc_error layout` 会是 `size=81920 align=16`？
3. 为什么 `81920` 会被分配器变成 `131072`？
4. `overhead=30649584` 的统计口径是什么，为什么这么高？
5. 进程退出时内核栈是否释放？
6. `Cache full` 大量刷日志意味着什么？
7. slab / VMA（或 page-backed）哪个方向更对症？
8. 对比 `oskernel2025-chronix-retest`，它做法有何差异？

---

## 2. 结论摘要

### 2.1 不是“已证实的纯泄漏”，而是“高并发创建压力 + 分配器内耗 + 回收时机问题”

- OOM 触发点发生在 `clone` 路径申请 `81920`（内核栈）时失败。
- 堆只有 64MB，崩溃时 `actual` 已到 66,991,504（99%）。
- `user` 仅 36,341,920，而 `overhead` 达 30,649,584，说明内部碎片/size class 取整损耗非常高。
- 日志显示大量 `Cache full ... forcing eviction`，块缓存路径长期处于压力/异常淘汰状态。

### 2.2 `layout=81920` 的直接来源是内核栈大小配置

- `KERNEL_STACK_SIZE = 4096 * 20 = 81920`。
- `kstack_alloc()` 通过 `Vec<u128>` 按该大小申请。

### 2.3 `81920 -> 131072` 是 buddy allocator 的幂次取整规则

- 分配大小按 `max(next_power_of_two(size), align, word_size)` 计算。
- 所以 81920 上取整到 131072（2^17，class 17）。

### 2.4 `overhead` 统计口径

- `user`：请求大小总和（layout.size）。
- `actual`：分配后真实占用总和（按 class 上取整）。
- `overhead = actual - user`。

该值高说明很多活跃对象都落在“请求尺寸与 class 尺寸差距较大”的区间。

### 2.5 进程退出时内核栈不是“立即彻底释放”

- 退出时先释放 `TaskUserRes`（用户栈/trap 相关）。
- `TaskControlBlock.kstack` 只有在对应 `Arc<TaskControlBlock>` 引用归零后才释放。
- 当前实现中主线程槽位在僵尸阶段通常仍保留（`tasks` 只 pop 到长度 1）。

---

## 3. 关键证据（日志与代码）

## 3.1 OOM 发生在 lmbench `lat_ctx`（context switch overhead）阶段

- `context switch overhead`：`all-bylm.log:60052`
- `size=32k ovr=68.37`：`all-bylm.log:60055`
- 已跑到 `64 51.83`：`all-bylm.log:60212`
- 紧接着 `alloc_error`：`all-bylm.log:60213`

`lmbench_testcode.sh` 对应命令：

```sh
lmbench_all lat_ctx -P 1 -s 32 2 4 8 16 24 32 64 96
```

见：`/home/grl/codeRepo/testsuits-for-oskernel/scripts/lmbench/lmbench_testcode.sh:35`

## 3.2 OOM 快照

来自日志：

- `layout: size=81920 align=16`（`all-bylm.log:60213`）
- `rounded_size=131072 class=17`（`all-bylm.log:60214`）
- `heap: user=36341920 actual=66991504 overhead=30649584 total=67108864 free=117360 used_pct=99%`（`all-bylm.log:60215`）
- 当前线程 `last_syscall=220`（`all-bylm.log:60217`）

`220` 对应 syscall 名称 `clone`：

- `SYSCALL_FORK const = 220`：`os/src/syscall/mod.rs:222`
- 名称表 `(220, "clone")`：`os/src/syscall/mod.rs:489`

## 3.3 最近分配轨迹显示反复 `81920 + 4096`，最终 81920 失败

`recent_alloc` 片段：

- 失败项：`req=81920 ... ok=false`（`all-bylm.log:60237`）
- 前序大量成功项：`req=81920` 与 `req=4096` 交替（`all-bylm.log:60238` ~ `60260`）
- 同一上下文：`pid=8 tid=0 syscall=220`

这说明崩溃点不是单个偶发大分配，而是 `clone` 压力下重复创建路径叠加。

## 3.4 `Cache full` 不是零星现象

统计结果（`rg ... | wc -l`）：

- `Cache full with all pages recently used/in use` 出现 **48729 次**

并且在 OOM 前后持续刷出（例如 `all-bylm.log:60030`~`60039`）。

---

## 4. 各问题详细回答

## 4.1 为什么 `layout` 这么大？

因为这次触发的是内核栈分配：

- `KERNEL_STACK_SIZE = 4096 * 20`：`os/src/config.rs:14`
- `kstack_alloc()`：`vec![...; KERNEL_STACK_SIZE / size_of::<u128>()]`：`os/src/task/id.rs:80`

所以 `layout.size=81920` 是配置直接决定，不是异常值。

## 4.2 为什么会 `81920 -> 131072`？

`buddy_system_allocator` 的基本策略是按 2 的幂大小分配：

- `size = max(layout.size().next_power_of_two(), align, word_size)`  
  你这边封装：`os/src/mm/heap_allocator.rs:205`  
  crate 原始实现：`~/.cargo/.../buddy_system_allocator-0.6.0/src/lib.rs:103`

因此 81920 被向上取整到 131072（class 17）。

## 4.3 `overhead` 的统计口径与高值原因

### 口径

OOM 打印来自：

- `user = heap.stats_alloc_user()`：`os/src/mm/heap_allocator.rs:109`
- `actual = heap.stats_alloc_actual()`：`os/src/mm/heap_allocator.rs:110`
- `overhead = actual - user`：`os/src/mm/heap_allocator.rs:113`

底层计数：

- 分配时：`self.user += layout.size(); self.allocated += size;`  
  `~/.cargo/.../buddy_system_allocator-0.6.0/src/lib.rs:132-133`
- 释放时对应减回：`lib.rs:181-182`

### 为什么高

这不是单个对象问题，而是“很多对象同时存在 + 每个对象都被 class 上取整”导致的总和：

- kstack 一项：每任务损耗 `128KB - 80KB = 48KB`
- 80 个活跃任务仅此就损耗约 `3.75MB`
- 再叠加大量中小对象（容器节点、Arc/Vec 扩容等）落到上一级 class
- 最终 `overhead` 累到 30MB 量级

## 4.4 进程退出时内核栈释放了吗？

“部分释放、延迟释放”：

- 退出时立刻做的是 `task_inner.res = None`（用户资源）：`os/src/task/mod.rs:212`
- `kstack` 在 `TaskControlBlock` 内，依赖 Arc 引用计数归零才释放：`os/src/task/task.rs:51-54`
- 当前退出逻辑中 `tasks` 仅 `pop` 到长度 1（主线程槽位保留）：`os/src/task/mod.rs:263-265`

因此“退出后马上全部释放”这个假设不成立。

## 4.5 `Cache full` 我们怎么看？

这是需要优先修的点。当前实现中存在一个明显可疑处：

- 先 `cloned()` 出候选 `Arc`，再判 `strong_count > 1`：  
  `os/src/drivers/block/cached_block_device.rs:270, 274`

因为 clone 本身就增加引用计数，`>1` 可能长期成立，导致正常淘汰路径难生效，频繁走 `forcing eviction`：

- `warn!("Cache full ... forcing eviction")`：`cached_block_device.rs:295`

此外默认缓存容量不小：

- `BLOCK_CACHE_SIZE` 默认 `49_152` blocks（约 24MB）：`cached_block_device.rs:36,49`

在 64MB 堆预算下，这个占比很高。

---

## 5. slab 还是 VMA/page-backed：哪个更对症？

对这次问题，优先级应是：

1. **先改内核栈分配策略**（把大对象从通用 heap 剥离）：  
   例如按页管理 / 专用池（避免 80KB 落入 128KB class 的浪费）。
2. **再引入/强化 slab**（优化小对象高频分配碎片）。

原因：本次最重的“可见热点”是大对象 kstack 的 class 取整损耗。

---

## 6. 与 `oskernel2025-chronix-retest` 的差异

对比路径：`/home/grl/codeRepo/oskernel2025-chronix-retest`

### 6.1 该仓库中内核栈策略

- `KERNEL_STACK_SIZE = 16 * 4096`（64KB）：  
  `hal/src/component/constant/riscv64.rs:27`  
  `hal/src/component/constant/loongarch64.rs:27`

- 存在 `.bss.stack` 的静态 `BOOT_STACK`（按 CPU 数预留）：  
  `hal/src/component/entry/mod.rs:26`

- KVM 映射有 `KernVmAreaType::KernelStack` 专门区域：  
  `os/src/mm/vm/kvm/riscv64.rs:109, 293`

### 6.2 该仓库堆配置

- `KERNEL_HEAP_SIZE = 256*1024*1024`：  
  `os/src/mm/allocator/heap_allocator.rs:2`

### 6.3 该仓库 slab 状态

- 有 slab 实现：`os/src/mm/allocator/slab_allocator.rs`
- 但全局分配器当前注册的是 `HeapAllocator`：`heap_allocator.rs:15`

因此它与当前仓库最大的收益点仍是：  
**每任务动态 kstack 压力模型不同（至少不是你当前这条“80KB Vec 上堆”路径）。**

---

## 7. 改进建议（按优先级）

## P0（先做）

1. 修 `cached_block_device` 淘汰逻辑中 `strong_count` 判断时机（避免 clone 干扰判断）。
2. 给 `Cache full` 日志做节流（比如每 N 次/每秒打印一次）。
3. 暂时下调 `BLOCK_CACHE_SIZE`（建议先 4MB~8MB）验证 OOM 变化。

## P1

1. 将 `kstack` 从全局 heap 分离（页粒度/专用池）。
2. 增加 `kstack alloc/free live/peak` 计数，验证回收时机问题。
3. 增加 `alloc + free` 双向 trace（当前只有 alloc 轨迹）。

## P2

1. 对小对象导入 slab 或专门 cache（Task/PCB/FD 小节点）。
2. 优化进程僵尸阶段资源保留策略（缩短主线程栈持有时间）。

---

## 8. 一句话结论

这次 `all-bylm.log` 的 `alloc_error`，核心不是单点“神秘泄漏”，而是：

**`clone` 高压场景下，64MB 堆内同时承受“大对象 kstack 的幂次取整损耗 + 其他对象活跃占用 + block cache 异常淘汰压力”，最终在 `lat_ctx` 末段被顶满。**

