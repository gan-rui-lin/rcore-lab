# VFS 短锁收敛与文件 I/O 热点微调报告（stage9）

## 摘要

本次 stage9 是对 stage7 FS/VFS 文件层锁拆分后的补强：stage7 已经把 `VfsFile`、`Pipe`、`MemFd`、`ROOT_VFS` 的锁形态拆细，但审计后发现仍有几类“锁对象变细了，但持锁范围仍包住慢路径”的问题。本轮只做短锁收敛与热点路径微调，不引入 sleep lock，不改 lwext4/块设备全局模型，也不改变文件系统语义。

核心收益：

- `ROOT_VFS` 只保护 mount 表快照，不再包住 inode `lookup/create/list/remove/truncate` 等慢路径。
- `sync/shutdown` 不再持 `ROOT_VFS` 锁执行 ext4 `flush/shutdown`。
- `VfsFile::read_user_buffer()` 不再把用户页解析/用户内存写入包在 file offset 锁里。
- `Pipe` 在阻塞前锁外检查 `O_NONBLOCK` 和 pending signal，避免 pipe 锁与 task/process signal 锁交叉。

本轮改动文件：

- `os/src/fs/vfs/core.rs`
- `os/src/fs/vfs/file.rs`
- `os/src/fs/vfs/mount.rs`
- `os/src/fs/pipe.rs`

## 背景问题

stage7 后的锁结构已经比旧版本清晰，但仍有三个关键风险点：

1. **ROOT_VFS helper 仍可能包住 inode 慢路径**
   - `open_file()` 通过 `with_root_vfs_read()` 调用 `resolve_quiet()` / `resolve_parent()`，而这些函数内部会执行 inode `lookup()`。
   - `O_CREAT` 路径还会在 root VFS read lock 内执行 `parent.create()`。
   - 这与 `core.rs` 中“ROOT_VFS 锁只短持，不跨 inode 操作”的注释不一致。

2. **ext4 flush/shutdown 持有 ROOT_VFS 锁**
   - `sync_filesystems()` 在 `with_root_vfs_read()` 内执行 `vfs.flush_ext4()`。
   - `shutdown_filesystems()` 在 `with_root_vfs_write()` 内执行 `vfs.shutdown_ext4()`。
   - 这类操作可能进入 ext4/block 设备路径，不适合被 root mount table 锁包裹。

3. **VfsFile direct read 路径持 offset 锁处理用户页**
   - `read_user_buffer()` 原先在持有 `offset` 锁时调用 `for_each_user_write_slice()`。
   - 该路径可能触发用户页表检查、COW/demand fault fallback 和逐 slice 用户内存写入。
   - offset 锁应只保护 shared file description 的 offset 更新，不应覆盖用户内存大循环。

4. **Pipe 空/满阻塞前在 pipe 锁内检查 signal**
   - `has_pending_unmasked_signal()` 会访问 task/process signal 状态。
   - 虽然当前是单核 + 中断屏蔽模型，但这会让 pipe 内部锁与 task/process 锁产生不必要交叉，不利于后续 sleep lock/SMP 化。

一句话根因：stage7 已经拆了锁对象，本轮要继续把“锁保护的数据”和“锁外慢操作”分开，避免细锁仍被长临界区拖成粗锁。

## 核心改动

### 1) ROOT_VFS mount 快照化

新增 mount snapshot helper：

- `resolve_mount_snapshot(path)`
- `resolve_inode_quiet(path)`
- `resolve_inode(path)`
- `resolve_parent_inode(path)`
- `root_inode_snapshot()`

新的路径分两段：

1. 在 `ROOT_VFS.read()` 内只 clone 当前最匹配 mount 的 root inode、相对路径和 ext4 guard。
2. 释放 `ROOT_VFS` 后，根据 snapshot 执行 inode `lookup()` 链。

这样 `open_file/list_apps/path_is_dir/create_dir/path_exists/remove_path` 都不再持 root mount table 锁执行 inode 操作。`O_CREAT` 的 `parent.create()` 也被移到 ROOT_VFS 锁外。

收益：

- mount table 锁只保护 mount 表本身，语义边界更干净。
- 大量 `openat/fstatat/path lookup` 类路径不再把 inode 层慢操作压进全局 VFS 锁。
- 为后续引入 inode sleep lock 或真正多核锁打基础。

### 2) ext4 flush/shutdown 脱离 ROOT_VFS 锁

新增：

- `ext4_guards_snapshot()`
- `take_ext4_guards()`

`sync_filesystems()` 现在只在短读锁内 clone `Arc<Ext4Fs>` 列表，锁外逐个 `flush()`。

`shutdown_filesystems()` 只在短写锁内 take ext4 guard 列表，锁外逐个 `shutdown()`。

收益：

- 避免 ROOT_VFS 锁覆盖 ext4/block 层 I/O。
- shutdown/sync 不再阻塞普通路径对 root mount table 的读访问。

### 3) VfsFile::read_user_buffer() offset 锁短持

改动前：

- 持有 `self.offset.lock()`。
- 在锁内调用 `for_each_user_write_slice()`。
- 用户页解析、COW fallback、用户内存写入与 inode read 都在 offset 锁范围内。

改动后：

- 先调用 `ensure_user_writable(..., DemandCowWithForkFallback)` 预热/确认用户写范围。
- 使用 16 KiB bounce buffer 分块读取。
- 每个分块只在 offset 锁内执行：
  - `inode.read_at(*offset, bounce)`
  - `*offset += n`
- 释放 offset 锁后，再 `copy_to_user_inline()` 写回用户空间。

语义保持：

- 普通 `read/write` 的 shared file offset 串行语义不做激进拆分。
- EOF 返回已读字节。
- copy 失败时，如果已有部分读入则返回部分字节；否则返回 `-EFAULT`。

收益：

- offset 锁不再覆盖用户页表处理和用户内存大循环。
- `read` 是 `hotspot.md` 中第二高频 syscall（约 110k 次），该路径的临界区缩短对整体 I/O 热点更直接。
- 对后续进一步优化 `readv/sendfile/direct I/O` 有更清晰边界。

### 4) Pipe 阻塞前锁外检查 nonblock/signal

改动点：

- `read()` 不再在 pipe 锁内读取固定的 `nonblock` 快照，而是在每次准备阻塞前重新读取 atomic flag。
- `read/write/write_user_buffer()` 在判断需要阻塞后先释放 pipe 锁，再检查：
  - `self.nonblock.load(Ordering::Relaxed)`
  - `has_pending_unmasked_signal(true)`
- `write_user_buffer()` 补齐阻塞前 `O_NONBLOCK` 返回 `-EAGAIN` 的路径，并保持 EINTR sentinel。

收益：

- pipe 内部锁不再与 task/process signal 锁嵌套。
- `fcntl(F_SETFL, O_NONBLOCK)` 改动更容易被等待循环观察到。
- 继续保证 `suspend_current_and_run_next()` 不在 pipe 锁内执行。

## 收益分析

结合 `hotspot.md`，本轮收益最相关的路径是：

- `read`：约 110k 次，是最直接受益路径。`read_user_buffer()` 现在把用户页处理移到 offset 锁外。
- `fstatat/openat`：路径解析类调用受益于 ROOT_VFS snapshot，root mount table 锁不再包住 inode lookup。
- `write/writev/pipe`：pipe 阻塞路径锁序更干净，减少未来 signal/futex/task 锁交叉风险。
- `tmp-iozone`：读吞吐路径对 VFS/file offset 和 inode read 更敏感，本轮验证中没有出现吞吐回退或 hang。

这次不是“算法级”优化，而是“锁持有范围”优化。它的主要价值在于：

1. 降低热点路径中断关闭/临界区时间。
2. 避免全局 VFS 锁被 inode/ext4/block 慢路径拖长。
3. 把后续 sleep lock / SMP 化前最危险的锁嵌套先拆开。

## 验证记录

### 编译验证

```bash
cargo check --release --target riscv64gc-unknown-none-elf --features ext4
cargo check --release --target loongarch64-unknown-none --features ext4
make -C os rv
make -C os la
```

结果：均通过。

说明：并行执行 `make -C os rv` 与 `make -C os la` 时曾遇到 `user/target-user` 被并发 clean/rebuild 的构建目录竞态；单独重跑 `make -C os la` 后通过。这不是本轮代码问题。

### 静态验收

```bash
rg "with_root_vfs_read\\(|flush_ext4\\(|shutdown_ext4\\(|has_pending_unmasked_signal" os/src/fs
git diff --check
```

结果：

- `with_root_vfs_read()` 只剩 snapshot helper 内短锁使用。
- `flush_ext4()/shutdown_ext4()` 旧方法已移除，mount 层改为 guard snapshot/take 后锁外调用。
- `pipe.rs` 中的 `has_pending_unmasked_signal()` 均位于 pipe lock scope 之外。
- `git diff --check` 通过。

### RV 基础回归

默认 `sdcard-rv.img` 在当前工作区启动即出现 ext4 mount panic，因此本轮回归使用从 `sdcard-rv.img.xz` 解压出的干净镜像：

```bash
xz -dc sdcard-rv.img.xz > /tmp/rcore_stage9_sdcard-rv.img
```

执行：

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
SINGLE_TEST=/musl/basic/fork  LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
SINGLE_TEST=/musl/basic/wait  LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
SINGLE_TEST=busybox          LOG=INFO timeout 300 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f /tmp/rcore_stage9_sdcard-rv.img -t rv
```

结果：

- `/musl/basic/write`：通过
- `/musl/basic/fork`：通过
- `/musl/basic/wait`：通过
- `/musl/basic/mount`：通过
- `busybox`：通过
- `tmp-iozone`：通过

`tmp-iozone` 关键输出：

```text
Children see throughput for  4 readers       =   18427.04 kB/sec
Parent sees throughput for  4 readers        =   16253.55 kB/sec
Children see throughput for 4 re-readers     =    8256.41 kB/sec
Parent sees throughput for 4 re-readers      =    7760.56 kB/sec
Children see throughput for 4 random readers =    1998.40 kB/sec
Parent sees throughput for 4 random readers  =    1954.81 kB/sec
```

按计划未跑 `SINGLE_TEST=cyclictest`。

## 已知边界与风险

1. **ROOT_VFS snapshot 不提供跨 mount 变更的一致视图**
   - lookup 链开始后，如果未来出现真正并发 mount/unmount，snapshot 只保证持有旧 root 的 `Arc` 生命周期。
   - 当前系统缺少复杂运行时 mount/unmount 并发语义，本轮选择保持现有行为边界。

2. **read_user_buffer 使用 bounce buffer 会增加一次 copy**
   - 收益是 offset 锁明显缩短，代价是每块多一次 kernel buffer copy。
   - 对当前目标（降低锁持有时间/避免用户页处理持锁）是合理取舍。

3. **普通 read/write 仍保持 offset 锁覆盖 inode I/O**
   - 这是为了保守维持 shared file offset 的原子更新语义。
   - 后续如果要继续优化，可设计“reserve offset + commit”或 per-inode append lock，但需要更仔细定义 partial read/write 与 O_APPEND 语义。

4. **Pipe 仍使用 sentinel 表达 EINTR/EAGAIN**
   - 这延续现有 `File` trait 约定。
   - 后续若统一 `File::read/write` 返回 `Result<usize, isize>`，可清理这类 sentinel。

## 下一阶段建议

下一阶段锁重构建议不要继续扩大 VFS 表层，而应进入 **ext4 inode/file 与 block cache 的慢路径锁模型**：

1. **Ext4Inode.file / lwext4 调用边界**
   - 当前 `with_data_file()` 仍可能把 lwext4 文件对象访问串行化。
   - 建议先审计 `read_at/write_at/truncate/stat/metadata/xattr` 的锁顺序与 I/O 时间，再决定是否引入 sleep lock 或 inode-level rw lock。

2. **块设备 cache manager**
   - `tmp-iozone` 中 initial writer / random writer 仍明显低于 reader。
   - 下一阶段收益更可能来自 block cache 写回、dirty page/cache flush、virtio block 队列锁，而不是继续微调 `VfsFile` 表层。

3. **read/writev 与 page cache 路径**
   - `readv/preadv/writev/pwritev` 当前仍有较多用户 iovec 转换与 inode I/O 混合逻辑。
   - 可复用本轮短锁原则：用户内存解析在锁外，offset/inode 只短持必要状态。

4. **统一 File trait errno 返回**
   - Pipe/MemFd/VfsFile 对 EINTR/EAGAIN/EFAULT 的表达仍不统一。
   - 后续可以把 `File::read/write` 逐步迁到 `Result<usize, isize>`，减少 sentinel 分支。
