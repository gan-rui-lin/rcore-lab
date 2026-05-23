# ext4 运行时补路径导致镜像损坏调试报告

日期：2026-05-23

## 背景

在 rCore-lab RV ext4 镜像上跑单测后，宿主机重新挂载镜像失败：

```text
mount: /mnt/sdcard-rv-1: mount(2) system call failed: Structure needs cleaning.
dmesg(1) may have more information after failed mount system call.
```

用户手动恢复 `sdcard-rv.img` 后，初始镜像可以通过只读检查：

```bash
e2fsck -fn sdcard-rv.img
```

因此排查重点是 rCore-lab 启动或测试过程中对 ext4 镜像的写入行为，而不是镜像包本身已经损坏。

## 根因结论

直接触发点是 ext4 根文件系统启动时调用 `ensure_basic_paths()` 和 `ensure_busybox_links()`，在镜像根目录补齐 `/etc`、`/dev`、`/tmp`、`/root`、`/bin`、`/usr`、`/lib` 等路径。

在开启 `metadata_csum` 的 ext4 镜像上，经 lwext4 `mkdir` 新建的目录块会被宿主 Linux ext4 检查为 checksum 错误。再叠加之前缺少 shutdown 时显式 ext4 umount/journal stop，以及 block cache 写回时序不符合 ext4 预期，镜像会进入 `needs_recovery`、journal corrupt 或 directory checksum failure 状态。

本轮最终选择的兼容策略是：仍然保留内核侧 runtime mkdir，但测试镜像关闭 `metadata_csum`。这样运行时路径依然真实落在 ext4 中，同时避开当前 vendor lwext4 与 Linux ext4 metadata checksum 的兼容风险。

## 复现路径

使用干净镜像副本，先确认基线：

```bash
rm -f /tmp/rcore-sdcard-repro.img
cp --reflink=auto sdcard-rv.img /tmp/rcore-sdcard-repro.img
e2fsck -fn /tmp/rcore-sdcard-repro.img
```

运行单测：

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 120 \
  bash run.sh -f /tmp/rcore-sdcard-repro.img -t rv
```

早期现象分两层：

1. 不显式 shutdown ext4 时，superblock 可能留下 `needs_recovery`，`e2fsck -p` 会看到 journal replay 或 corrupt transaction。
2. 增加 ext4 shutdown 后，superblock 可以变成 clean，但开启 `metadata_csum` 的镜像仍会看到新建目录 checksum 错误。

关键只读检查输出示例：

```text
Directory inode 3220, block #1: directory passes checks but fails checksum.
Directory inode 3221, block #1: directory passes checks but fails checksum.
Directory inode 3222, block #1: directory passes checks but fails checksum.
```

用 `debugfs ncheck` 反查 inode：

```bash
debugfs -R "ncheck 3220" /tmp/rcore-sdcard-repro.img
debugfs -R "ncheck 3221" /tmp/rcore-sdcard-repro.img
debugfs -R "ncheck 3222" /tmp/rcore-sdcard-repro.img
```

对应路径为：

```text
3220 -> /dev/shm
3221 -> /tmp
3222 -> /root
```

这说明损坏点集中在启动时由 `ensure_basic_paths()` 新建的目录，而不是测试程序 `write` 自身的数据文件写坏。

## 排除过的假设

### 镜像原始内容损坏

恢复后的 `sdcard-rv.img` 在运行 rCore 前 `e2fsck -fn` 返回 `0`，因此不是压缩包或手动 mount 导致的初始损坏。

### 只是 virtio 非阻塞模式问题

非阻塞路径确实有 bug：`read_block_nb/write_block_nb` 提交失败时只打印 fallback 日志，却没有真正执行 blocking I/O；cached block device 也没有把 IRQ 转发到底层 virtio 设备。修掉后非阻塞测试不再卡住，但 polling 模式下仍能观察到 ext4 目录 checksum 问题，因此非阻塞不是主根因，只是同一排查中暴露出的独立 I/O bug。

### 只是没有 umount

缺少 ext4 shutdown 会留下 journal 或 superblock 脏状态，但加上 shutdown 后仍能看到 `/dev/shm`、`/tmp`、`/root` 的目录块 checksum failure。因此 shutdown 是必要修复，但不是完整修复。

### 只要改成 ramfs 就够了

把 `/etc`、`/dev`、`/tmp`、`/root` 改成 ramfs 可以绕开写 ext4 目录块，验证上也能避免 checksum failure。但该方案改变了运行时路径的持久化语义，且 `/bin`、`/usr/bin`、`/lib` 等 BusyBox/动态链接器兼容路径仍需要额外 fallback。当前决策是不采用 ramfs workaround，而是保留内核侧 mkdir，并要求测试镜像关闭 `metadata_csum`。

## 修复策略

### 1. 仍然在内核侧补齐运行时路径

`ensure_basic_paths()` 继续创建：

```rust
create_dir("/etc");
create_dir("/dev");
create_dir("/dev/misc");
create_dir("/dev/shm");
create_dir("/bin");
create_dir("/usr");
create_dir("/usr/bin");
create_dir("/tmp");
create_dir("/root");
```

`ensure_busybox_links()` 也继续在创建 hardlink 前补齐 `/bin`、`/usr`、`/usr/bin`、`/lib`，以及 loongarch64 的 `/lib64`。

这样做的语义最接近原先设计：运行时补路径属于内核启动兼容层，测试执行后这些路径会留在 ext4 镜像内。

### 2. 测试镜像关闭 metadata_csum

保留内核侧 mkdir 后，必须避免在当前 vendor lwext4 路径上写入 Linux 不能接受的 metadata checksum。推荐对测试镜像副本执行：

```bash
e2fsck -fy /tmp/sdcard-rv-nocsum.img
tune2fs -O ^metadata_csum /tmp/sdcard-rv-nocsum.img
e2fsck -fn /tmp/sdcard-rv-nocsum.img
dumpe2fs -h /tmp/sdcard-rv-nocsum.img | rg "Filesystem features|Filesystem state"
```

`vendor/lwext4_rust/c/lwext4/README.md` 声明 `metadata_csum: yes`，源码中也存在 `CONFIG_META_CSUM_ENABLE`、`ext4_dir_csum_verify()`、`ext4_dir_set_csum()`、`ext4_dir_init_entry_tail()` 等实现。但本次实测说明，在 rCore-lab 当前集成和目录创建路径下，开启 `metadata_csum` 的镜像仍可能被 Linux `e2fsck` 判为目录 checksum 错。

因此关闭 `metadata_csum` 是镜像格式兼容 workaround，不等价于断言 vendor 完全没有 checksum 支持。

### 3. shutdown 时显式关闭 ext4

root VFS 是全局对象，原先 `Ext4BlockWrapper` 不会在 `sys_shutdown` 前 drop，导致 lwext4 的 journal stop/umount 不执行。

修复后：

- `Ext4Fs` 持有 `Option<Ext4BlockWrapper<Ext4Disk>>`
- VFS shutdown 遍历 ext4 guard 并调用 `fs.shutdown()`
- `sys_shutdown()` 在 `arch::shutdown()` 前调用 `crate::fs::shutdown_filesystems()`

日志中可以看到：

```text
[ INFO] Drop struct Ext4BlockWrapper
[ INFO] lwext4 umount Okay
```

### 4. block cache 写路径改为 write-through

ext4/lwext4 期望下层 block device 的写入顺序和持久化更接近同步语义。原先 cached block device 只标 dirty，可能让目录项、inode bitmap、journal 等元数据落盘顺序不符合 ext4 预期。

修复后 `CachedPage::write_block()` 更新 cache 后立即写底层 block device，并清掉 dirty bit。该层仍保留读缓存，但写路径不再延迟。

### 5. 修复 virtio 非阻塞 fallback 和 IRQ 转发

非阻塞模式下：

- `read_block_nb/write_block_nb` 返回 Err 后，现在会真正调用 blocking retry。
- 已提交 token 但找不到 condvar 时直接 panic，避免“请求已提交但静默 fallback”造成重复 I/O。
- `CachedBlockDevice::handle_irq()` 转发到底层 virtio 设备，否则 PLIC 到 cache 层后中断处理是空操作。

### 6. 修复 mkdir 后的进程锁重入

非阻塞验证中暴露了一个额外 panic：

```text
[upcell] exclusive_access conflict type=os::task::process::ProcessControlBlockInner
caller=src/syscall/process.rs:1857 holder=src/syscall/fs.rs:1104
```

原因是 `sys_mkdirat()` 创建目录后持有当前进程 inner guard，又调用 `inode_for_path()`，后续路径会再次访问当前进程状态。修复为先取出 uid/gid，再 drop guard，再做 inode metadata 设置。

## rv2.log 中 inotify03 的失败原因

按 `rcore-ltp-failure-triage` 的稳定性优先策略，先看 `RUN LTP CASE` 附近和 hard-stability 信号：

```bash
rg -n "RUN LTP CASE inotify0[123]|TCONF|FAIL LTP CASE inotify|alloc_error|last_syscall=47|Panicked" rv2.log -S
```

关键输出：

```text
6000:RUN LTP CASE inotify01
6005:inotify01.c:146: TCONF: syscall(26) __NR_inotify_init1 not supported on your arch
6013:FAIL LTP CASE inotify01 : 32
6014:RUN LTP CASE inotify02
6019:inotify02.c:203: TCONF: syscall(26) __NR_inotify_init1 not supported on your arch
6027:FAIL LTP CASE inotify02 : 32
6028:RUN LTP CASE inotify03
6029:[kernel] alloc_error layout: size=314572800 align=1
6033:[kernel] alloc_error current: pid=3 tid=0 name=<proc_busy> status=Running last_syscall=47 ...
6046:[kernel] alloc_error recent_alloc[0]: ... req=314572800 ... ok=false
6070:[kernel] Panicked at src/mm/heap_allocator.rs:196
```

`os/src/syscall/mod.rs` 中 syscall 47 是 `fallocate`。LTP 源码路径说明这次 panic 出现在测试设备准备阶段，而不是 inotify 事件语义阶段：

- `inotify03.c` 使用 `.format_device = 1`，测试前需要创建并挂载测试设备。
- `lib/tst_device.c` 中 `DEV_SIZE_MB` 为 `300u`。
- `tst_acquire_loop_device()` 调用 `tst_prealloc_file(filename, 1024 * 1024, acq_dev_size)`。
- `lib/tst_fill_file.c` 中 `tst_prealloc_size_fd()` 先调用 `fallocate(fd, 0, 0, bs * bcount)`。

所以 `rv2.log` 记录的 P0 失败是：LTP 创建 300MiB loop image 时触发 rCore `sys_fallocate(fd, 0, 0, 300MiB)`；当时的实现会把“预分配/扩展文件”映射到实际大块写入或分配路径，最终在内核堆上申请 `314572800` 字节失败并 panic。

本轮实现后又用关闭 `metadata_csum` 的副本做了一次单独复现：

```bash
SINGLE_TEST=all LTP_START_FROM=inotify03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 LOG=INFO \
  timeout 60 bash run.sh -f /tmp/sdcard-rv-inotify03.img -t rv \
  > /tmp/repro_inotify03.log 2>&1
```

这次没有再出现 `alloc_error` 或 panic。关键日志变成：

```text
RUN LTP CASE inotify03
[syscall] pid=3 name=inotify03 num=47(fallocate) args=[0x3,0x0,0x0,0x12c00000,...] ret=0
tst_device.c:147: TINFO: No free devices found
tst_device.c:354: TBROK: Failed to acquire device
FAIL LTP CASE inotify03 : 2
```

这说明 `inotify03` 仍然没有进入 inotify 事件语义验证；当前阻塞点已经从 `fallocate(300MiB)` panic 后移到 LTP loop device acquisition。后续若要真正修 `inotify03`，需要继续处理测试设备准备链路，包括 loop device 查找/attach 能力，以及之后的 `inotify_init1`/watch/event 语义。

本次不把 inotify/fallocate/loop-device 修复混进 ext4 checksum 变更。

最小复现命令：

```bash
SINGLE_TEST=all LTP_START_FROM=inotify03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 LOG=INFO \
  timeout 60 bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/repro_inotify03.log 2>&1
rg -n "RUN LTP CASE inotify03|alloc_error|last_syscall=47|Panicked" /tmp/repro_inotify03.log
```

当前代码下也应额外看 loop-device 信号：

```bash
rg -n "RUN LTP CASE inotify03|fallocate\\).*ret=0|No free devices found|Failed to acquire device|FAIL LTP CASE inotify03" \
  /tmp/repro_inotify03.log
```

## 验证矩阵

### 镜像特性检查

```bash
rm -f /tmp/sdcard-rv-nocsum.img
cp --reflink=auto sdcard-rv.img /tmp/sdcard-rv-nocsum.img
e2fsck -fy /tmp/sdcard-rv-nocsum.img
tune2fs -O ^metadata_csum /tmp/sdcard-rv-nocsum.img
e2fsck -fn /tmp/sdcard-rv-nocsum.img
dumpe2fs -h /tmp/sdcard-rv-nocsum.img | rg "Filesystem features|Filesystem state"
```

预期：

- `Filesystem features` 不再包含 `metadata_csum`
- `Filesystem state` 为 `clean`
- `e2fsck -fn` 返回 `0`

### 编译检查

```bash
cd os
cargo check --release --target riscv64gc-unknown-none-elf --features ext4
```

### polling 模式：write

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_write.log 2>&1
e2fsck -fn /tmp/sdcard-rv-nocsum.img
```

预期：

- `/musl/basic/write completed`
- 日志包含 `lwext4 umount Okay`
- `e2fsck -fn` 返回 `0`

### polling 模式：mount

```bash
SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_mount.log 2>&1
e2fsck -fn /tmp/sdcard-rv-nocsum.img
```

预期：

- `mount return: 0`
- `umount return: 0`
- 日志包含 `lwext4 umount Okay`
- `e2fsck -fn` 返回 `0`

### 非阻塞 virtio 模式：mount

```bash
VIRTIO_BLK_NON_BLOCKING=1 SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_nb_mount.log 2>&1
rg -n "Panicked|ERROR|All tests completed|mount return|lwext4 umount" \
  /tmp/rcore_nocsum_nb_mount.log
e2fsck -fn /tmp/sdcard-rv-nocsum.img
```

预期：

- 不出现 panic
- `mount return: 0`
- `=== All tests completed ===`
- `e2fsck -fn` 返回 `0`

## 影响范围

这次修复保留 ext4 模式下启动补路径的持久化语义：

- `/etc`、`/dev`、`/tmp`、`/root` 继续写入 ext4 镜像。
- `/bin`、`/usr/bin`、`/lib` 等继续由内核启动时补齐，服务 BusyBox、脚本和动态链接器兼容路径。
- 镜像侧需要关闭 `metadata_csum`，否则当前 vendor lwext4 集成仍可能写出 Linux ext4 不接受的目录 checksum。

## 后续关注

1. 若未来必须支持开启 `metadata_csum` 的镜像，应补一个最小 C/Rust 侧 mkdir repro，对比 Linux e2fsck 的目录 checksum 计算。
2. 建议把 `e2fsck -fn` 加入 ext4 单测后的开发验证脚本，避免“测试通过但镜像已坏”的问题再次悄悄出现。
3. `rv2.log` 中 `inotify03` 的第一阻塞点不是 inotify 语义，而是 `fallocate(300MiB)` 导致内核大分配 panic；当前复现中 panic 已消失，但仍卡在 LTP loop device acquisition，应作为单独稳定性问题继续处理。
4. write-through 会降低写性能，但换来 ext4 元数据一致性。后续可以在确认 lwext4 flush/barrier 语义后，再考虑有序写回或显式 sync API。
