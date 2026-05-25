# 第 10 阶段：块设备 Cache 锁重构与 I/O 脱锁报告

## 背景

第 7-9 阶段已经把 PCB/TCB、VFS 文件对象、ROOT_VFS 以及部分文件 I/O 热点做了短锁收敛。剩余的 I/O 热点继续向下落到块设备 cache：`CachedBlockDevice` 过去用一个 `CacheManager` 全局锁管理 cache page，同时命中、淘汰、统计、sync 路径会在 manager 锁内进一步锁 page，甚至可能触发 dirty page 写回。

这类锁嵌套对 `tmp-iozone` 的 writer/random writer、ext4 flush/sync、dirty eviction 特别敏感。即使当前仍是单核 + polling I/O，manager 锁覆盖底层设备 I/O 也会放大全局串行区，后续一旦引入非阻塞块设备或 sleep lock，风险会更高。

本阶段不直接引入通用睡眠锁。原因是 block cache 在 boot/mount 阶段也会使用，此时未必存在可睡眠的 current task；直接把底层锁改成 sleep lock 容易把早期初始化路径、panic/drop 兜底写回、非阻塞 virtio 配置揉成一个大风险点。

## 改动范围

代码改动集中在：

- `os/src/drivers/block/cached_block_device.rs`

新增报告：

- `docs/GRLDocs/block-cache-lock-split-stage10-report-2026-05-25.md`

未纳入本阶段：

- VFS/lwext4 语义改造。
- 通用 `SleepLock`。
- virtio-blk 非阻塞模式重构。
- `analysis/`、`hotspot.md`、`分工.md`、`user/src/bin/initcode.rs`、`scripts/ltp/analyze_rv_ltp_log.py` 等旁路文件。

## 核心改动

### 1. Cache page 内部化 page lock

`CachedPage` 不再由外层 `Arc<Mutex<CachedPage>>` 包裹，而是变为：

- `CachedPage.accessed: AtomicBool`
- `CachedPage.inner: Mutex<CachedPageInner>`

`CachedPageInner` 承载：

- `data`
- `loaded_mask`
- `dirty_mask`

命中路径只需要原子更新 accessed bit，不再为了 second-chance 标记而进入 page lock。

### 2. manager 锁只管理元数据

`CacheManager.pages` 改为保存 `Arc<CachedPage>`。`get_page()` 返回：

```rust
(Arc<CachedPage>, Vec<Arc<CachedPage>>)
```

第一项是命中或插入后的目标 page；第二项是从 manager 中摘除、需要在锁外显式 sync/drop 的 evicted pages。

现在 manager 锁只保护：

- `pages`
- `queue`
- capacity / second-chance 选择

不再在 manager 锁内：

- 锁 page。
- 检查 dirty mask。
- 调用 page sync。
- 触发底层 block device I/O。

### 3. dirty eviction 和 sync 移到 manager 锁外

`CachedBlockDevice::read_block/write_block()` 现在流程为：

1. 短持 manager 锁获取目标 page，并摘除需要淘汰的 page。
2. 释放 manager 锁。
3. 对 evicted pages 显式 `sync()`。
4. 对目标 page 执行 `read_block/write_block()`。

`CachedBlockDevice::sync()` 改为 manager 锁内只 clone page snapshot，锁外逐页 sync。

`CachedBlockDevice::stats()` 改为 manager 锁内只生成 snapshot 和容量统计，dirty block 统计在锁外逐页读取。

### 4. 避免淘汰外部持有 page

第二轮补强了 fallback eviction：如果所有 page 都被外部持有，manager 不强行移除这些 page，而是允许短暂超过容量。这样避免移除一个仍在调用者手中的 page。

## 锁边界

本阶段后的锁边界为：

- `CacheManager` lock：只管理 `pages/queue` 元数据。
- `CachedPage.inner` lock：只管理单个 page 的 `data/loaded_mask/dirty_mask`。
- 底层 block device I/O：允许持有 page lock，但不持有 manager lock。

锁顺序固定为：

```text
manager lock -> clone/remove Arc<CachedPage> -> release manager lock -> page lock -> device I/O
```

禁止路径：

```text
manager lock -> page lock -> dirty sync/device I/O
```

## 一致性窗口审计结论

本轮认真审计了 block index 到 page 的唯一性窗口，并试过一个更保守的“两阶段 eviction”方案：candidate 在 sync 期间继续留在 `pages` 表内，sync 后再 finalize remove。该方案可以更强地保证同一 `page_id` 不会在旧 dirty page 写回完成前被重新加载，但 `tmp-iozone` 在 600 秒内未完成，性能退化过大，因此本阶段不采用。

当前保留的实现有这些边界：

- 在当前单核 + polling I/O + 不跨调度持 page lock 的模型下，evicted page 通常不会和同 `page_id` 的新 page 并发执行。
- `dirty_mask/loaded_mask/data` 都在 `CachedPage.inner` page lock 内读写，单个 page 内 `sync/read/write` 互斥。
- 显式 sync 会清 dirty bit；后续 `Drop` 再调用 `sync()` 时看到 `dirty_mask == 0`，不会重复写回。
- 如果后续引入 SMP、真正异步块设备、sleep lock 或允许 block I/O 跨调度，本窗口必须重新设计。建议方向是 per-page `Evicting/Writeback` 状态、inflight page 表或 writeback completion barrier，而不是简单地在 miss 路径同步等待 victim 写回。

## 收益判断

本阶段收益预计为中高，主要体现在写重和 flush/eviction 路径：

- writer/random writer：dirty page 淘汰和写回不再堵住整个 cache manager，其他 cache lookup/insert 的等待时间下降。
- ext4 sync/flush：snapshot 后逐页 sync，manager 锁不再覆盖整批 dirty page 写回。
- cache hit read：收益较小，主要来自 accessed bit atomic 化，少一次 page lock。
- 后续非阻塞块设备：锁边界更清楚，为 sleep lock 或 async block I/O 留出接口。

这不是单次吞吐必然大幅上升的改动。当前系统仍是单核 polling、ext4/lwext4 内部仍有更大锁域，writer 初写仍受元数据创建、文件扩展和底层写放大影响。本轮更核心的收益是消除 manager 全局锁覆盖 I/O 的结构性瓶颈。

## 验证结果

### 编译

已通过：

```bash
cargo check --release --target riscv64gc-unknown-none-elf --features ext4
cargo check --release --target loongarch64-unknown-none --features ext4
make -C os rv
make -C os la
```

说明：`make rv` 与 `make la` 会分别清理并重建 user target；单独 cargo check 前需要对应架构的 initcode 已生成。

### 静态验收

执行：

```bash
rg -n "cache\\.lock\\(\\)\\.sync_all|cached\\.lock\\(\\)\\.mark_accessed|candidate\\.lock\\(\\)\\.has_dirty|\\.lock\\(\\)\\.sync\\(|\\.lock\\(\\)\\.read_block|\\.lock\\(\\)\\.write_block" os/src/drivers/block/cached_block_device.rs
git diff --check
```

结果：

- 无旧式 page-lock accessed/dirty/sync 命中。
- `git diff --check` 通过。
- `pages.remove` 只用于 manager 元数据摘除，不在 manager 锁内 sync dirty page。

### 基础回归

使用镜像：

```bash
xz -dc sdcard-rv.img.xz > /tmp/rcore_stage10_sdcard-rv.img
```

已通过：

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage10_sdcard-rv.img -t rv
SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage10_sdcard-rv.img -t rv
SINGLE_TEST=busybox LOG=INFO timeout 300 bash run.sh -f /tmp/rcore_stage10_sdcard-rv.img -t rv
```

`busybox` 日志中出现的 `timeout` 是 busybox applet 名称，不是外层 timeout 触发。

### I/O 热点

已通过：

```bash
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f /tmp/rcore_stage10_sdcard-rv.img -t rv
```

关键摘要：

```text
Children see throughput for  4 initial writers = 31.26 kB/sec
Parent sees throughput for  4 initial writers   = 28.82 kB/sec
Children see throughput for  4 rewriters        = 1577.01 kB/sec
Parent sees throughput for  4 rewriters         = 449.69 kB/sec
Children see throughput for  4 readers          = 17820.53 kB/sec
Parent sees throughput for  4 readers           = 14942.09 kB/sec
Children see throughput for 4 re-readers        = 8555.93 kB/sec
Parent sees throughput for 4 re-readers         = 7784.41 kB/sec

Children see throughput for  4 initial writers = 35.40 kB/sec
Parent sees throughput for  4 initial writers   = 31.98 kB/sec
Children see throughput for  4 rewriters        = 1501.88 kB/sec
Parent sees throughput for  4 rewriters         = 433.93 kB/sec
Children see throughput for 4 random readers    = 2361.71 kB/sec
Parent sees throughput for 4 random readers     = 2283.06 kB/sec
Children see throughput for 4 random writers    = 912.43 kB/sec
Parent sees throughput for 4 random writers     = 427.27 kB/sec
```

本轮按要求未跑 `SINGLE_TEST=cyclictest`。

## 暂不引入 SleepLock 的原因

现在还不是把 FS/VFS 全面切到 sleep lock 的好时机：

- block cache 会在 early boot/mount 路径使用，未必有 current task 可睡眠。
- `Drop for CachedPage` 仍保留兜底 sync，drop 上下文不适合直接睡眠。
- 当前 virtio block 默认 polling；如果打开 `VIRTIO_BLK_NON_BLOCKING`，page lock 是否可能跨 schedule 需要单独审计。
- lwext4/VFS 内部仍有语义锁和全局模型，直接替换底层锁容易扩大改动面。

因此本阶段先完成 I/O 脱 manager 锁，让 manager 不再覆盖底层 I/O；sleep lock 放到下一阶段有条件引入。

## 下一阶段建议

下一阶段可以进入“boot-safe sleep/adaptive lock 设计 + ext4/lwext4 锁域拆分预研”，但建议满足这些条件后再落地：

1. 定义 `SleepLock` 或 `AdaptiveLock` 的上下文规则：
   - early boot / no current task 时走 non-sleep path。
   - task context 中允许阻塞。
   - panic/drop/irq 相关路径禁止睡眠。

2. 给 block cache page lock 增加上下文审计：
   - polling 模式继续短持 page lock。
   - 非阻塞 virtio 模式下禁止 page lock 跨 schedule。
   - dirty writeback 需要明确是否同步等待完成。

3. 拆 ext4/lwext4 慢路径：
   - inode metadata lock。
   - file data lock。
   - xattr/statfs/list cache helper。
   - flush/shutdown snapshot 化继续向下推进。

4. 建立性能基线：
   - `tmp-iozone` writer/random writer。
   - busybox shell I/O。
   - mount/unmount。
   - ext4 sync/dirty eviction 压力场景。

收益预期：

- 若只把 spin/mutex 机械替换为 sleep lock，收益不稳定，风险偏高。
- 若先完成 boot-safe 上下文模型，再把 ext4 inode/file 慢路径从全局锁中拆出，收益会更明显，尤其是并发 write、metadata create/remove、sync/flush。
