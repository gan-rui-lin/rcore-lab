# lmbench(rv) `lat_fs` 输出 `-1` 与 `memory_set.rs:1018` panic 调试报告

日期：2026/3/25

---

## 一、先给结论（罪魁祸首）

本次问题有两个表象，但核心链路是一条：

1. **`lat_fs` 输出 `-1` 的直接原因**：`lat_fs` 过程中由 `benchmp` 派生的子进程反复收到 `SIGSEGV(11)`，导致 `gettime()==0`，lmbench 按既定逻辑打印 `-1`。这不是“计时器读不到值”，而是“测量进程异常退出”。
2. **最终 panic 的直接原因**：内核在 `os/src/mm/memory_set.rs:1018` 执行 `frame_alloc().unwrap()` 时拿不到页帧（物理页分配失败），触发 panic。

也就是说：

- `lat_fs` 的 `-1` 是“前置异常信号”；
- 最后的 panic 是“资源进一步恶化后的硬崩溃点”；
- 两者高度相关，且都发生在 **musl lmbench 阶段内部**，并非“已经切到 glibc 才炸”。

---

## 二、关键证据与定位依据

### 2.1 `lat_fs` 的 `-1` 并非正常性能值

在 `all-rv-lmbench copy.log` 可以看到：

- `file system latency` 下 0k/1k/4k/10k 全部是 `-1`。

这与 lmbench 源码 `testsuits-for-oskernel/lmbench_src/src/lat_fs.c` 完全对应：

- `measure()` 里如果 `gettime()==0`，就打印 `\t-1\t-1` 或 `\t-1`。

因此 `-1` 的语义是“本轮测量没有得到有效时间值”，不是某种可接受的慢值。

### 2.2 为什么 `gettime()==0`：子进程被 `SIGSEGV` 打死

在最小复现日志 `all-rv-lmbench-info-debug.log`（`LOG=INFO SINGLE_TEST=lmbench`）里，`lat_fs` 区间出现大量模式化序列：

1. 子进程进入 `fstatat` / `mkdirat`；
2. 紧接着出现：`[ WARN] [signal] pid=... default handler for signal 11 -> terminate`；
3. 父进程随后写出 `-1` 行。

这说明 `lat_fs` 的工作子进程是“跑着跑着段错误退出”，不是“返回错误码后优雅收敛”。

### 2.3 最后 panic 的现场

panic 日志：

- `[kernel] Panicked at src/mm/memory_set.rs:1018`

对应源码位置为：

- `let frame = frame_alloc().unwrap();`

这个 `unwrap()` 只有在 `frame_alloc()` 返回 `None` 时才会触发，结论非常直接：**物理页帧耗尽（或可分配页被耗尽）**。

结合 panic 前日志窗口可见当时在高频 `clone`/进程切换相关路径上，说明内存压力并非瞬时单点，而是随着 lmbench 后续阶段累计上来的。

---

## 三、为什么说“不是马上切到 lmbench-glibc 才炸”

很多人第一反应是“musl 跑完了，切 glibc 时炸”。这次不是。

判断依据：

1. 已给日志里 `musl/lmbench_testcode.sh` 输出还在进行（`context switch overhead` 后续）即 panic；
2. 日志中没有出现“开始跑 `/glibc/lmbench_testcode.sh`”的清晰切换标识；
3. panic 点发生在 lmbench 子进程活跃 clone 阶段，不像是切换下一套测试前的初始化阶段。

因此，这次 panic 归因应放在 **musl lmbench 本轮内部**。

---

## 四、对根因的分层判断（按确定性排序）

### A. 确定性高（已实锤）

1. `lat_fs` 的 `-1` 来自 `gettime()==0`，非“慢值”；
2. `gettime()==0` 的直接触发是子进程 `SIGSEGV`；
3. 最终 panic 是 `frame_alloc` 失败导致 `unwrap` 崩溃。

### B. 高概率（需要下一轮证据闭环）

`lat_fs` 子进程在 `mkdirat` 附近崩溃，可能是“用户态内存分配失败后空指针链式解引用”，或“某个路径处理触发的用户态非法访问”。

理由：

- `lat_fs.c` 在目录名构造和递归路径构建中大量依赖 `malloc/strdup/tempnam`，对返回值校验很少；
- 一旦内存紧张，空指针很容易在后续 `sprintf("%s/...", basename)` 里爆成 `SIGSEGV`；
- 这与日志中“syscall 看似正常，随后立刻 signal 11”的形态一致。

### C. 系统级可能性（并行怀疑项）

内核内存生命周期存在泄漏或回收滞后（例如文件对象、管道、页表页、临时 VMA、子进程资源），在 lmbench 的 fork/clone/pipe 高压模式下被放大，最终触发 `frame_alloc` 耗尽。

这能解释为什么：

- 前面指标还能跑出数值；
- 到后半段（尤其 context switch/fork 压力后）突然进入不可逆崩溃。

---

## 五、为何这两处问题要一起看

很多调试会把 `lat_fs=-1` 和 `memory_set panic` 分开看，但这会丢失时间序关系：

1. 先出现 `lat_fs` 子进程段错误（系统已不健康）；
2. 再继续执行更多压力项；
3. 最终在 `frame_alloc` 处耗尽并 panic。

如果只盯 panic，容易误判成“单次偶发内存不足”；
如果只盯 `lat_fs=-1`，又会忽略其后续对全局稳定性的破坏。

因此正确策略是把它当成同一条故障链来处理。

---

## 六、下一步调试计划（可直接执行）

### 6.1 先把 `lat_fs` 子进程 `SIGSEGV` 打穿

目标：拿到“段错误前最后一次关键内核路径”的确定证据。

建议：

1. 对 `mkdirat/openat/fstatat/unlinkat/rmdir` 增加按 pid 的简要 trace（只对 lmbench 进程名或目标 pid 打印）；
2. 在用户地址访问边界处（copyin/copyout 与路径解析）打印失败地址与返回 errno；
3. 观察 `SIGSEGV` 前是否存在固定 syscall 序列与固定地址模式。

### 6.2 同步盯内存水位

目标：确认是否存在明显泄漏/回收缺口。

建议：

1. 在 `frame_alloc/frame_dealloc` 处每 N 次打印剩余页数（或分配总量）；
2. 在 `fork/exec/exit/wait` 关键节点打印当前页帧计数快照；
3. 对比 `lat_fs` 前后、`lat_ctx` 前后的水位变化。

### 6.3 临时止血（用于继续跑后续项）

在根因未闭环前，可临时在 `memory_set.rs` 将 `unwrap()` 改成带错误信息的 `expect("frame_alloc failed at ...")` 并附带计数，至少让 panic 信息可用于下一轮归因，而不是只有裸行号。

---

## 七、当前结论状态

- `/code/lmbench_src/bin/build/lmbench_all` 路径问题已独立修复，不是这次 `rv` `lat_fs=-1` 的主因；
- 当前主线问题是：
  1) `lat_fs` 子进程 `SIGSEGV`；
  2) 之后资源恶化，`frame_alloc` 失败 panic。

后续应优先把 `SIGSEGV` 的第一触发点钉死，因为这一步通常也是避免后续 OOM 链式故障的关键。
