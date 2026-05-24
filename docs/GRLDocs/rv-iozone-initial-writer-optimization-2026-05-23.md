# RV iozone initial writer 性能优化记录

## 背景

lazy block writeback 之后，`tmp-iozone` 的 rewriter/random writer 已经明显变快，但 initial writer 仍长期停在 `30-50 kB/sec`。这说明底层 512B write-through 不是唯一瓶颈。新文件第一次写入时，ext4 还要执行 block allocation、extent 更新、bitmap/group metadata 更新、inode blocks/size 更新等追加分配路径。

本轮目标是提高 RV 上 `tmp-iozone` 的 initial writer 性能，同时保持测试镜像正常退出后 `e2fsck -fn` clean。

## 现象和证据

基线短测命令：

```bash
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f /tmp/sdcard-rv-iozone-lazy2.img -t rv
```

lazy writeback 后的稳定短测结果：

- initial writers: `38.44 kB/sec`
- rewriters: `2348.08 kB/sec`
- random initial writers: `40.99 kB/sec`
- random writers: `936.07 kB/sec`

这组数据说明：

1. 已有块覆盖写已经受益于 lazy writeback。
2. 新文件 initial writer 仍慢，瓶颈集中在 ext4 首次追加分配。
3. 只减少底层 VirtIO 写次数，无法消除每个新 ext4 block 的元数据维护成本。

进一步做 1KiB/4KiB record 对比后可以看到：`tmp-iozone` 默认 `-r 1k`，而 ext4 block size 是 4KiB。也就是说用户每次 `write(1KiB)` 进入 lwext4 时，通常还没形成完整 ext4 block，更不可能触发多块 extent 批量追加。

## 根因

根因可以概括为一句话：

`tmp-iozone` 用 1KiB record 顺序写新文件，内核每次都把 1KiB 小写直接交给 lwext4；vendor lwext4 的追加分配路径又倾向逐块分配，因此 initial writer 同时承担高频 syscall、lwext4 全局锁、部分块写、逐块分配和元数据更新成本。

关键路径分两层。

### 1. VFS 层的小写放大

`os/src/fs/vfs/file.rs` 原先每次 `write()` 都立即调用：

```rust
inner.inode.write_at(inner.offset, *slice)
```

对 `tmp-iozone -r 1k -s 1m -t 4` 来说，每个进程写 1MiB 文件，需要约 1024 次 1KiB write。4 个进程就是数千次小写进入 ext4。

这些小写每次都会走到：

```rust
Ext4Inode::write_at()
Ext4File::file_write()
ext4_fwrite()
```

因此 lwext4 的 mount point lock、seek/write、事务和元数据路径被高频触发。

### 2. lwext4 层的逐块追加

在 vendor lwext4 的 `ext4_fwrite()` 中，已有块和新块路径不同：

```c
if (iblk_idx < ifile_blocks) {
    r = ext4_fs_init_inode_dblk_idx(&ref, iblk_idx, &fblk);
} else {
    rr = ext4_fs_append_inode_dblk(&ref, &fblk, &iblk_idx);
}
```

rewriter 多数时候走已有块查询路径，所以 lazy block cache 可以显著减少后端写入。

initial writer 需要不断 append 新块，涉及：

- data block allocation；
- block bitmap 修改；
- block group free count 修改；
- superblock free count 修改；
- inode blocks count 修改；
- extent tree 更新；
- inode size/mtime/ctime 更新；
- lwext4 全局锁和 transaction 开销。

如果每 4KiB 只分配一次，甚至每 1KiB 就进一次 lwext4，metadata cost 会压过数据写本身。

## 修复策略

本轮采用两段式优化。

### 1. VFS fd 层合并顺序小写

在 `VfsFile` 内增加 per-file write buffer：

- buffer 大小：`32KiB`
- 只对非 `O_DIRECT` 普通写启用
- 顺序写时先追加到 fd 本地 buffer
- buffer 满、读、seek、fsync/fdatasync、sync、drop 时 flush
- 非连续写前先 flush，避免乱序覆盖

关键文件：

- `os/src/fs/vfs/file.rs`
- `os/src/fs/mod.rs`
- `os/src/syscall/fs.rs`
- `os/src/syscall/mod.rs`

这一步的作用是把 iozone 的 1KiB record 合并为更大的 `inode.write_at()`。这样可以减少：

- 进入 `Ext4Inode::write_at()` 的次数；
- 进入 `ext4_fwrite()` 的次数；
- lwext4 全局锁获取次数；
- 部分块写处理次数；
- metadata transaction 频率。

32KiB 是折中值：足以把 1KiB record 合并成 8 个 ext4 block，又比 64KiB 更少触发 iozone children throughput 的 `nan` 汇总现象。

### 2. lwext4 extent 顺序追加批量分配

只合并到 4KiB 还不够，因为一次 flush 只追加一个 ext4 block，仍然会逐块分配。于是 vendor lwext4 增加连续块批量分配路径：

- `ext4_balloc_alloc_blocks()`：在同一个 block group bitmap 中寻找连续空闲 run。
- `ext4_new_meta_blocks()`：当 extent 层请求 `count > 1` 时真正批量分配，而不是永远只分配 1 个 block。
- `ext4_fwrite()`：对 extent-enabled 文件的顺序追加完整块，调用 `ext4_extent_get_blocks(..., wanted, ..., &got)`，一次追加多个连续块。

关键文件：

- `vendor/lwext4_rust/c/lwext4/include/ext4_balloc.h`
- `vendor/lwext4_rust/c/lwext4/src/ext4_balloc.c`
- `vendor/lwext4_rust/c/lwext4/src/ext4_extent.c`
- `vendor/lwext4_rust/c/lwext4/src/ext4.c`
- `vendor/lwext4_rust/c/lwext4/liblwext4-riscv64.a`

批量分配成功时一次性更新：

- block bitmap；
- bitmap checksum；
- superblock free block count；
- block group free block count；
- inode blocks count。

找不到足够连续空间时退化为单块分配，避免扩大失败面。

### 3. 顺序读写跳过重复 seek

`vendor/lwext4_rust/src/file.rs` 暴露 `file_pos()`，`Ext4Inode::read_at/write_at()` 在 cached `Ext4File` 当前 `fpos` 已经等于目标 offset 时跳过 `file_seek()`。

这个改动本身不是主要提速来源，单独测试时 initial writer 基本没有明显变化，但它减少了顺序读写热路径上的无用调用。

## 为什么能提高性能

这次性能提升来自“减少次数”，不是让单次 ext4 分配神奇变快。

原路径近似是：

```text
1024 次 1KiB write
  -> 1024 次 VfsFile::write
  -> 1024 次 Ext4Inode::write_at
  -> 1024 次 ext4_fwrite
  -> 每 4 次才形成 1 个 ext4 block
  -> 约 256 次逐块 append allocation
```

新路径近似是：

```text
1024 次 1KiB write
  -> VfsFile fd buffer 合并
  -> 约 32 次 32KiB flush
  -> 约 32 次 ext4_fwrite
  -> 每次最多追加 8 个连续 ext4 block
  -> 大幅减少 append allocation 和 metadata 更新次数
```

因此 initial writer 变快的直接原因是：

1. syscall 后面的 ext4 写入调用被合并。
2. lwext4 全局锁持有/释放次数减少。
3. 部分块写变少，更多写入变成完整 block 范围。
4. extent 追加从逐块分配变成范围分配。
5. bitmap/group/superblock/inode blocks count 更新次数减少。
6. lazy block cache 继续把实际后端写延迟到 flush/eviction/shutdown。

rewriter/random writer 也会受益，因为顺序或局部写可以被 fd buffer 和 block cache 双层合并。但 initial writer 收益更关键，因为它额外绕开了大量逐块 append metadata 成本。

## 验证

编译命令：

```bash
cd os
cargo check --release --target riscv64gc-unknown-none-elf --features ext4
```

结果：通过。仍有既有 `unexpected cfg` warning。

测试镜像准备命令：

```bash
cp --reflink=auto sdcard-rv.img /tmp/sdcard-rv-initial-writer-buffer32.img
e2fsck -fy /tmp/sdcard-rv-initial-writer-buffer32.img || true
tune2fs -O ^metadata_csum /tmp/sdcard-rv-initial-writer-buffer32.img || true
e2fsck -fy /tmp/sdcard-rv-initial-writer-buffer32.img || true
e2fsck -fn /tmp/sdcard-rv-initial-writer-buffer32.img
```

smoke：

```bash
LOG=INFO SINGLE_TEST=/musl/basic/write timeout 150 bash run.sh -f /tmp/sdcard-rv-initial-writer-buffer32.img -t rv
e2fsck -fn /tmp/sdcard-rv-initial-writer-buffer32.img

LOG=INFO SINGLE_TEST=/musl/basic/mount timeout 150 bash run.sh -f /tmp/sdcard-rv-initial-writer-buffer32.img -t rv
e2fsck -fn /tmp/sdcard-rv-initial-writer-buffer32.img
```

结果：

- `/musl/basic/write completed (status=0x0)`
- `/musl/basic/mount completed (status=0x0)`
- 两轮后 `e2fsck -fn` clean
- `lwext4 umount Okay`

短测：

```bash
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f /tmp/sdcard-rv-initial-writer-buffer32.img -t rv > /tmp/rcore_tmp_iozone_buffer32.log 2>&1
rg -n "nan|initial writers|rewriters|random writers|completed|Panicked|ERROR" /tmp/rcore_tmp_iozone_buffer32.log
e2fsck -fn /tmp/sdcard-rv-initial-writer-buffer32.img
```

关键结果：

- write/read initial writers: `134.59 kB/sec`
- write/read rewriters: `12014.27 kB/sec`
- random-read initial writers: `11191.72 kB/sec`
- random-read rewriters: `11750.64 kB/sec`
- random writers: `3038.75 kB/sec`
- `tmp-iozone` completed
- 无 `Panicked`
- 无 `ERROR`
- 无 `nan`
- 运行后 `e2fsck -fn` clean

对比 lazy writeback 后的 initial writer `38-41 kB/sec`，第一段 children 口径提升到 `134.59 kB/sec`，已经超过本轮 `>100 kB/sec` 的接受线。

## 指标口径说明

iozone 同时输出 children 和 parent throughput。children 口径更接近各 worker 自身观测到的 I/O 完成速度，parent 口径包含父进程等待、调度和汇总开销。

32KiB buffer 下：

- 第一段 children initial writer 达到 `134.59 kB/sec`
- 第一段 parent initial writer 为 `49.74 kB/sec`
- 第二段 parent initial writer 为 `148.26 kB/sec`

因此本轮结论是：内核实际写入热路径已经明显改善，但 parent 口径仍受调度/等待/汇总影响，后续若要追求完整 iozone 分数，需要继续看调度、进程并发和 lwext4 全局锁串行化。

## 风险和边界

1. fd buffer 是内核侧 writeback buffer，不提供异常断电一致性保证。正常路径依赖 read/lseek/fsync/fdatasync/sync/drop/shutdown flush。
2. `sync(2)` 当前只 flush 当前进程 fd table 中的 file-local buffer，然后调用 filesystem/block cache sync。跨进程全局 fd buffer flush 还不是完整实现。
3. `Drop` 中使用 `try_exclusive_access()`，正常最后引用释放时应能拿到锁，但并发异常路径仍需要后续压力测试。
4. vendor lwext4 的 RV/LoongArch 静态库都已重建：`liblwext4-riscv64.a` 和 `liblwext4-loongarch64.a` 均包含本轮批量分配改动。
5. 批量分配仅针对 extent-enabled、顺序追加、完整块写入路径。非 extent、随机新洞、部分块写保持原路径。

## 后续建议

1. 跑完整 `SINGLE_TEST=iozone timeout 1800`，用 judge 脚本确认 musl/glibc 分数。
2. 增加跨进程全局 open file flush 注册表，让 `sync(2)` 不只覆盖当前进程 fd。
3. 对 fd buffer 增加 trace 计数：buffered bytes、flush count、direct write count、partial flush count。
4. 继续分析 parent throughput 偏低问题，重点看调度和 `EXT4_MP_LOCK` 竞争。
5. 如果继续优化 initial writer，下一步考虑 lwext4 extent preallocation 或减少 transaction/metadata 写入频率。
