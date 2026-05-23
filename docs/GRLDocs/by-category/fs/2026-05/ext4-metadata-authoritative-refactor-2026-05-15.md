# ext4 元数据权威化重构报告

日期：2026-05-15

## 背景

原先 ext4 路径上的 `stat/fstat/statx` 以及 `chmod/chown/utimens/link/symlink/xattr/statfs`
有一部分元数据由内核 syscall 层用全局 map 或临时值模拟维护，例如 path hash inode、
路径级 mode/owner/timestamp/link group/symlink/xattr 表。这会导致镜像内真实 ext4 inode
和 syscall 返回值之间出现两套状态源，也会让 hard link、权限、时间戳和持久化 xattr 的语义不稳定。

本次重构目标是：ext4 文件系统中 lwext4 已支持维护的元数据全部交给 lwext4 作为唯一权威；
内核只为 procfs、pipe、timerfd、内建 `/dev/*` 等虚拟对象合成必要 metadata。

## 主要改动

### VFS 接口扩展

- 扩展 `VfsNodeKind`，加入 `Symlink/Char/Block/Fifo/Socket/Unknown`，使 VFS 能表达 ext4 inode mode 中的文件类型。
- 扩展 `VfsMetadata`，加入 `kind/uid/gid/rdev/blksize`，使 `stat/statx/fstat` 可以从文件系统元数据一次性填充关键字段。
- 新增 `VfsStatFs` 以及一组可选 VFS 操作：`chmod/chown/utimens/link_to/symlink/readlink/mknod/xattr/statfs`。
  默认实现返回不支持，ext4 后端覆盖实现。

### ext4 后端权威化

- `metadata()` 改为通过 `ext4_raw_inode_fill` 读取真实 inode 号、mode、nlink、size、blocks、atime、mtime、ctime，
  并通过 `ext4_owner_get` 读取 uid/gid。
- `lookup()` 改为一次 `ext4_raw_inode_fill` 后根据 mode 判断 `VfsNodeKind`，删除目录和普通文件两次探测。
- 删除 `/foo` 与 `foo` 双路径尝试，所有传给 lwext4 的 ext4 路径统一为 mount point 下的绝对路径。
- `chmod/chown/utimens/link/symlink/readlink/mknod/xattr/statfs` 分别落到 lwext4 的对应 API：
  `ext4_mode_set`、`ext4_owner_set`、`ext4_atime_set`、`ext4_mtime_set`、`ext4_ctime_set`、
  `ext4_flink`、`ext4_fsymlink`、`ext4_readlink`、`ext4_mknod`、`ext4_*xattr`、`ext4_mount_point_stats`。
- `size()` 统一从 raw inode metadata 读取，避免 cached `ext4_file` 的 size 陈旧问题。

### syscall 层收敛

- `stat/fstat/fstatat/statx` 对普通 ext4 inode 直接使用 `VfsMetadata` 填充 `mode/uid/gid/ino/nlink/size/blocks/time/rdev`。
- `chmod/fchmod/chown/fchown/utimens/link/symlink/xattr/statfs` 不再直接维护 ext4 path metadata map，而是通过 VFS inode 操作转发到 ext4。
- 删除 ext4 路径对以下模拟状态的依赖：
  `PATH_MODES`、`PATH_OWNERS`、`TIMESTAMPS`、`PATH_LINK_GROUP`、`LINK_GROUP_COUNT`、
  `SYMLINK_TARGETS`、`XATTRS`、path hash inode。
- 字符设备和其他虚拟对象仍使用内核合成 metadata，但这些合成逻辑不再作为 ext4 metadata fallback。

## 验证

已执行：

```bash
cd os && cargo check --release --features ext4
cd os && cargo check --release
git diff --check
```

结果：全部通过。构建过程中仍有 vendored `riscv/smoltcp` 的既有 `unexpected_cfgs` warning，
与本次重构无关。

## 语义变化

- ext4 `stat/fstat/statx` 返回的 inode、nlink、uid/gid、mode、时间戳等字段以底层 ext4 inode 为准。
- lwext4 当前时间 API 只支持秒级，因此 `stat` 的纳秒字段在 ext4 metadata 路径上固定为 0。
- ext4 metadata 查询失败时不再回退到 path hash 或内存 map，而是返回对应错误路径。
- xattr 和 symlink 现在由 ext4 持久化维护，不再依赖 syscall 层的内存表。

## 后续关注

- `renameat2` 当前仍走已有的复制/删除路径，并非完整 ext4 rename 语义；后续可继续下沉到 lwext4 rename 能力。
- 非 ext4 后端保留默认 VFS 实现，后续如需同等语义，需要分别为 fat32/easyfs/procfs 补 metadata 能力。
- 特殊文件 `rdev` 已从 raw inode block payload 尽力读取；如后续需要更严格 Linux 设备号编码，可补专门 helper。
