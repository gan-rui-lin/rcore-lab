# iozone RV/LA 性能问题记录与定性（2026-03-27）

## 1. 问题定性（先给结论）

- 该问题当前应定性为 **“RV 路径显著偏慢 + 进展很慢”**，而不是“稳定卡死/死锁”。
- 现象是：RV 能进入 iozone throughput 阶段并产出部分结果，但整体推进速度明显落后 LA，导致在固定时间窗口里看起来像“卡住”。

## 2. 关键证据

### 2.1 RV 并非完全不动

- `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:117)` 出现 `iozone test complete.`
- `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:118)` 出现 `iozone throughput write/read measurements`
- 同一日志继续出现吞吐行：
  - `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:147)` `Children ... initial writers = 11.40 kB/sec`
  - `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:154)` `Children ... rewriters = 177.57 kB/sec`
  - `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:161)` `Children ... readers = 5901.05 kB/sec`

结论：RV 不是“完全停死”，而是“推进慢，阶段切换慢”。
补充：旧日志 `[all-rv-iozone.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone.log:118)` 只到 throughput 标题、未在该窗口内打印评分行，和 `ff1a` 相比体现的是“同一问题在不同时间窗口下的可见程度不同”。

### 2.2 RV 与 LA 的量级差距

对比 `ff1a` 两份日志：

- LA 初始写吞吐：`79.09 kB/sec`  
  见 `[all-la-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-la-iozone-ff1a.log:92)`
- RV 初始写吞吐：`11.40 kB/sec`  
  见 `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:147)`
- 比值约 `6.9x`（LA/RV）

- LA rewriter：`1331.01 kB/sec`  
  见 `[all-la-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-la-iozone-ff1a.log:99)`
- RV rewriter：`177.57 kB/sec`  
  见 `[all-rv-iozone-ff1a.log](/home/grl/codeRepo/rcore-lab/all-rv-iozone-ff1a.log:154)`
- 比值约 `7.5x`（LA/RV）

说明 RV 慢不是“偶发一点点慢”，而是稳定的量级差距。

### 2.3 `pselect6` 大量出现的含义

- 在 `[iozone-rv-glibc-tmp-info.log](/home/grl/codeRepo/rcore-lab/iozone-rv-glibc-tmp-info.log)` 中统计到 `pselect6` 约 `51143` 次。
- 同日志可见进入 throughput 阶段：  
  `[iozone-rv-glibc-tmp-info.log](/home/grl/codeRepo/rcore-lab/iozone-rv-glibc-tmp-info.log:524)`

解释：大量 `pselect6(...)=0` 更像是多进程 throughput 测试中的同步/等待行为被反复触发（尤其在 INFO 级 syscall tracing 下），它解释了“看起来在刷同一类 syscall”，但**不单独构成死锁证据**。

## 3. 本轮调试尝试与思路

1. 先把 iozone 缩成临时最小 workload（`tmp-iozone`）以缩短复现路径，避免全量 test 引入过多噪声。  
2. 对比 LA/RV 的多份日志（`all-*.log` 与 `*-ff1a.log`），确认 RV 能推进但明显慢。  
3. 检查了 `pselect6` 大量打印现象，确认主要是 syscall 级日志放大了“等待态”视觉效果。  
4. 回看块设备缓存与驱动路径，确认当前缓存方案已经做过一轮增强（更大缓存+页级缓存+二次机会淘汰），但 RV 仍显著落后。

缓存相关代码现状可见：  
- `[cached_block_device.rs](/home/grl/codeRepo/rcore-lab/os/src/drivers/block/cached_block_device.rs:15)` `CACHE_PAGE_SIZE = 16 * 1024`  
- `[cached_block_device.rs](/home/grl/codeRepo/rcore-lab/os/src/drivers/block/cached_block_device.rs:36)` 默认 `49_152` blocks  
- `[cached_block_device.rs](/home/grl/codeRepo/rcore-lab/os/src/drivers/block/cached_block_device.rs:241)` second-chance eviction

## 4. 为什么 RV 比 LA 慢这么多（当前猜想，按置信度排序）

### A. 驱动/总线路径差异（高置信）

- RV 走 `virtio-mmio`：`[run.sh](/home/grl/codeRepo/rcore-lab/run.sh:222)`  
- LA 走 `virtio-pci`：`[run-la.sh](/home/grl/codeRepo/rcore-lab/run-la.sh:138)`  
- 两端 HAL 实现也不同：  
  - `[virtio_rv.rs](/home/grl/codeRepo/rcore-lab/os/src/drivers/bus/virtio_rv.rs)`  
  - `[virtio_la.rs](/home/grl/codeRepo/rcore-lab/os/src/drivers/bus/virtio_la.rs)`

这类差异在 1KB 小 IO + 多进程切换场景下会被显著放大。

### B. workload 本身放大了“慢路径”成本（高置信）

测试命令含 `-t 4 -r 1k -s 1m`，属于小块高频 IO；如果底层一次 IO 的固定开销偏大，吞吐会迅速塌陷。  
并且两端都常用 `-smp 1`（非多核掩盖）：  
- `[run.sh](/home/grl/codeRepo/rcore-lab/run.sh:219)`  
- `[run-la.sh](/home/grl/codeRepo/rcore-lab/run-la.sh:9)`

### C. 缓存策略虽改善，但未改变 RV 根瓶颈（中置信）

缓存增强后，RV 不再“几乎没输出”，但和 LA 仍有明显差距，说明 cache 是“缓解项”，不是根因项。

### D. `pselect6` 本身不是根因（中高置信）

`pselect6` 大量出现更多是“慢进展时的外在表现”，真正根因更可能在 I/O 路径固定开销和调度/同步成本。

## 5. 暂不建议继续投入的方向（当前证据下）

- 仅凭 “`pselect6` 多” 继续深挖 syscall 层，不是最高收益路径。  
- 把问题简单归因为 ext4 库正确性问题，证据不足；目前更像“架构路径性能差异 + workload 放大”。

## 6. 后续建议（面向可交付）

1. 先以“可接受耗时”作为门槛，而不是追求 RV 接近 LA。  
2. 固化一个 RV 的 smoke 配置（只跑关键 iozone 子项）用于回归，避免每次全量等待过久。  
3. 后续若要继续压时延，优先做 RV block path 的细粒度 profile（每次读写的平均耗时、队列提交/完成延迟、flush 频度），再决定是否继续改驱动或缓存策略。  
