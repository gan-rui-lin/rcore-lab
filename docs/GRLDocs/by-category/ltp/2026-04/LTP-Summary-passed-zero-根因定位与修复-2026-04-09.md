# LTP Summary `passed=0` 异常调试报告

日期：2026/4/9

## 1. 问题背景

在 rCore 的 LTP 批量执行中，出现了一个非常反直觉的现象：单个用例正文里已经打印了多条 `TPASS`，但紧随其后的 `Summary` 段却持续输出 `passed   0`。由于评测和回归分析常常要读取 `Summary`，这个现象会直接导致“测试正文看起来通过、统计却显示全 0”的误判。

这类问题的危险点在于：它不是“某个 syscall 失败”这种显式功能缺失，而是“统计链路断裂”。如果不先把统计链路修通，后续做内核能力缺失分析（例如哪些 syscall 仍 ENOSYS）就会失真。

## 2. 现象与第一手证据

修复前日志（`/tmp/rv_run_timeout40.log`）中可以看到非常清楚的矛盾：

- `RUN LTP CASE abort01`（行 8187）
- `abort01.c:62 ... TPASS`（行 8192）
- `abort01.c:65 ... TPASS`（行 8193）
- 但 `Summary:` 下 `passed   0`（行 8195~8196）

同样的模式在 `accept01`、`accept03` 等大量用例中重复出现：

- `accept01` 的 `Summary` 仍为 `passed 0`（行 8218~8219）
- `accept03` 的 `Summary` 仍为 `passed 0`（行 8270~8271）

这说明不是单个用例偶发，而是 LTP 结果聚合机制系统性失效。

## 3. 调试思路与关键判断

### 3.1 为什么优先怀疑 `MAP_SHARED` / `fork` 语义

LTP 新框架（`tst_test.c`）的结果统计是“子进程打点、父进程汇总”模型：

1. 测试框架创建共享结果区（`mmap(MAP_SHARED)`）。
2. 子进程调用 `tst_res(TPASS/...)` 时，更新共享结果区计数。
3. 父进程在退出路径打印 `Summary`。

如果“正文 TPASS 正常打印”但“Summary passed 始终 0”，最典型解释是：

- 子进程写到了**自己的私有副本**，父进程看不到写入；
- 本质上就是共享映射在 `fork` 后被错误私有化（或退化为 COW 私有页）。

### 3.2 代码路径定位

沿内核路径追踪后，关键链路是：

- `sys_mmap()` 在 `os/src/syscall/process.rs` 中走 lazy VMA 注册。
- `fork` 时 `from_existed_user()` 在 `os/src/mm/memory_set.rs` 依据 `MapAreaKind` 判断是否共享。
- 只有 `MapAreaKind::Shared` 才会走“共享语义保留”分支；否则按私有可写页走 COW。

问题点在于：lazy mmap 注册时没有把 `MAP_SHARED` 语义正确携带到 `MapAreaKind`/mmap 元信息，导致后续 fork 阶段把本应共享的统计页当成私有页处理。

## 4. 引入 Commit 溯源

## 结论（引入本次回归的关键 commit）

**`88aab4786fd2465acad23d12d21a74616502e48f`**

提交信息：`mm: add VMA demand paging to reduce mmap heap pressure`

### 4.1 溯源证据

通过命令：

```bash
git log -S "insert_lazy_anon_area" -- os/src/syscall/process.rs os/src/mm/memory_set.rs
```

唯一命中该行为引入提交即 `88aab47...`。

在该 commit 的 diff 中，`sys_mmap()` 从旧路径切换为：

- `insert_lazy_anon_area(...)`
- `insert_lazy_file_area(...)`

这次切换本身是为了解决大 mmap 压力（动机正确），但它把 mmap 语义传播链改掉了，尤其是与后续共享语义判定的耦合点没有同步补齐，最终触发了 `Summary` 统计失真。

## 5. 修复方案

本次修复的目标是：让 lazy mmap 路径与非 lazy 路径在“共享语义元数据”上保持一致。

核心改动如下：

1. `sys_mmap()` 构造并传递 `MmapMeta`（shared/file_backed/file_writable）到 lazy 插入函数。  
   代码位置：`os/src/syscall/process.rs:2711-2741`

2. `insert_lazy_anon_area()` / `insert_lazy_file_area()` 接收 `meta`，并在 `meta.shared` 时显式设置：  
   - `MapAreaKind::Shared`  
   - `with_shared_frames()`  
   代码位置：`os/src/mm/memory_set.rs:189-220`

3. `/proc/<pid>/maps` 的 `s/p` 显示由“是否有 mmap_meta”改为“mmap_meta.shared 是否为真”，避免把所有 mmap 都标成共享。  
   代码位置：`os/src/mm/memory_set.rs:1216-1219`

这样一来，fork 阶段的共享判定（`from_existed_user()` 中 `area.kind == MapAreaKind::Shared`）才能命中正确分支，不再把共享统计页当作普通 COW 私有页。

## 6. 调试-测试循环验证

按“修复 -> 运行 -> 读日志 -> 再修复”的闭环执行，并在卡住时按约定重建镜像：

```bash
rm sdcard-rv.img && xz -dk ./sdcard-rv.img.xz
```

另外，为避免 `run.sh` 每次自动编译干扰调试节奏，验证阶段使用直接 QEMU 启动：

```bash
timeout 20 qemu-system-riscv64 -machine virt -kernel kernel-rv.bin -m 1G -nographic -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -no-reboot -device virtio-net-device,netdev=net -netdev user,id=net -rtc base=utc
```

修复后日志（`/tmp/qemu_direct_timeout20_after_imgreset.log`）关键结果：

- `abort01`：2 条 `TPASS`，`Summary passed   2`（行 73, 78, 79, 81, 82）
- `accept01`：5 条 `TPASS`，`Summary passed   5`（行 93, 104, 105）
- `accept03`：8 条 `TPASS`，`Summary passed   8`（行 127, 156, 157）

这与修复前 `passed   0` 的模式形成直接对照，证明统计链路恢复。

## 7. 影响评估

### 7.1 正向影响

- LTP `Summary` 与正文 `TPASS/TFAIL/TBROK` 对齐，回归统计可用。
- `MAP_SHARED` 跨 fork 语义在 lazy mmap 场景下恢复一致，不再出现“看起来共享、实际私有化”的隐性错误。

### 7.2 风险与边界

- 本修复主要覆盖 `sys_mmap` lazy 路径语义传递；
- 与 `shm*`（SysV SHM）路径并不冲突，但建议后续补一轮“混合场景”回归（匿名 shared / 文件 shared / SysV SHM）。

## 8. 后续建议

1. 在 CI 增加一个最小哨兵用例：`MAP_SHARED + fork + 子写父读`，并在失败时打印父子页框号或摘要值。  
2. 对 `sys_mmap` 建立“语义不丢失”约束：无论 eager/lazy 路径，`shared/file_backed/file_writable` 必须贯通到 `MapArea`。  
3. 保持双口径检查：`TPASS` 明细 + `Summary` 汇总，避免再次出现单口径误判。

---

## 附：本次确认的“引入 commit”

- **引入回归的关键提交**：`88aab4786fd2465acad23d12d21a74616502e48f`  
- 提交标题：`mm: add VMA demand paging to reduce mmap heap pressure`

