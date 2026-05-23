# RV iozone short: lazy writeback 后的剩余瓶颈

## 结论

这轮 lazy block writeback 已经验证了一个明确瓶颈：旧实现中每个 512B cached write 都立即落到底层 VirtIO，严重拖慢重复写路径。

但是 `tmp-iozone` 里最慢的 `initial writers` 仍然只在 `40 kB/sec` 左右。这个现象说明：当前最大短板已经从“每 512B 数据写都打 VirtIO”转移到 ext4 的首次创建/追加分配路径，也就是新文件第一次写入时的 inode/block allocation、extent 更新、bitmap/group metadata 更新、journal/flush 和全局锁路径。

## 数据现象

短测命令：

```bash
LOG=OFF SINGLE_TEST=tmp-iozone timeout 600 bash run.sh -f /tmp/sdcard-rv-iozone-lazy2.img -t rv
```

write-through 基线：

- initial writers: `29.88 kB/sec`
- rewriters: `539.34 kB/sec`
- random initial writers: `25.67 kB/sec`
- random writers: `384.62 kB/sec`

lazy writeback 后的稳定短测：

- initial writers: `38.44 kB/sec`
- rewriters: `2348.08 kB/sec`
- random initial writers: `40.99 kB/sec`
- random writers: `936.07 kB/sec`

观察：

1. `rewriters` 从 `539.34` 到 `2348.08 kB/sec`，说明延迟写回确实绕开了大量 512B 立即写盘。
2. `random writers` 从 `384.62` 到 `936.07 kB/sec`，也有明显收益。
3. `initial writers` 只从 `29.88` 到 `38.44 kB/sec`，收益很小，说明它主要卡在首次分配/元数据路径，而不是单纯数据块写盘。

## 为什么 initial writer 和 rewriter 差这么多

iozone 的 `initial writers` 是新文件第一次写入。对 ext4 来说，这不是“把数据写到已有块”这么简单，而是要一边写数据，一边为文件追加实际磁盘块。

在 vendor lwext4 的 `ext4_fwrite()` 里，关键分支是：

```c
if (iblk_idx < ifile_blocks) {
    r = ext4_fs_init_inode_dblk_idx(&ref, iblk_idx, &fblk);
} else {
    rr = ext4_fs_append_inode_dblk(&ref, &fblk, &iblk_idx);
}
```

含义：

- `iblk_idx < ifile_blocks`：这个逻辑块已经属于文件，走已有块查询路径。
- `else`：文件还没有这个逻辑块，需要通过 `ext4_fs_append_inode_dblk()` 为文件追加新块。

`rewriter` 重写已有文件，大部分时间走第一条路径：查已有 extent/block mapping，然后把数据写入已有块。lazy block cache 能把这些写合并/延后，所以提升明显。

`initial writer` 写新文件，大量时间走第二条路径：每追加一批块，都可能牵涉：

- 分配 data block；
- 更新 block bitmap；
- 更新 block group free count；
- 更新 inode size / blocks；
- 更新 extent tree；
- journal transaction；
- 目录项和 inode 元数据；
- lwext4 mount point 全局锁 `EXT4_MP_LOCK`。

这些操作有大量小粒度元数据读写和同步点。lazy block cache 只能减少最终打到底层 VirtIO 的次数，不能消除“分配一个新块要改很多 ext4 元数据”的 CPU 和锁开销。

## 为什么 random writer 也能提升

`tmp-iozone` 的 random writer 阶段不是从空文件开始随机写；它通常基于已经创建并写过的测试文件。也就是说，文件块多数已经分配好了。

因此 random writer 更接近“已有块覆盖写”，和 rewriter 类似，能吃到 lazy block cache 的收益。

## 当前判断

所以，“剩余最大短板是 ext4 初次创建/追加分配路径”的意思是：

1. 数据覆盖写路径已经被 lazy writeback 明显改善。
2. 新文件初次写入仍慢，说明慢点主要发生在写入前后的 ext4 元数据分配与维护。
3. 继续只优化底层 512B write-through，收益会递减；下一步应该看 ext4 append/allocation 路径本身。

## 下一步优化方向

优先级从低风险到高收益：

1. 打开/增加 `TRACE_EXT4_IO_STATS` 与 block cache stats，对 initial writer 统计 `kdev_write_calls`、`dev_writes`、partial writes、backend writes。
2. 看 `ext4_fs_append_inode_dblk()` 是否逐块分配，是否可以批量预分配连续块。
3. 看 `ext4_fwrite()` 对 1KiB record 的 syscall/锁频率：每次 write 都进 `EXT4_MP_LOCK`，4 个进程并发会被全局锁串行化。
4. 考虑面向顺序追加的 extent/block 预分配，减少 bitmap/group/inode 更新次数。
5. 再往下才是 VirtIO 多块请求/batched backend I/O；它对 rewriter 可能继续有用，但不是 initial writer 的第一瓶颈。

## 注意

完整 iozone 的 glibc 阶段曾在 1800s timeout 被外部杀掉，之后镜像 `e2fsck -fn` 报错。这不是 lazy writeback 正常退出后的结果，而是测试进程被 SIGTERM 中断、没有正常 shutdown/umount 的结果。

正常完成的短测和 basic 回归结果：

- `/musl/basic/write`: pass，`e2fsck -fn` clean
- `/musl/basic/mount`: pass，`e2fsck -fn` clean
- `tmp-iozone`: pass，`e2fsck -fn` clean
