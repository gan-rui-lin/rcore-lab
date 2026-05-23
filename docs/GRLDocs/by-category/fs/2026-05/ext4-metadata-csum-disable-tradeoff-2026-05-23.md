# ext4 关闭 metadata_csum 的权衡说明

日期：2026-05-23

## 背景

rCore-lab 的 RV ext4 镜像在内核启动阶段会补齐运行时路径，例如 `/etc`、`/dev`、`/tmp`、`/root`、`/bin`、`/usr/bin`、`/lib`。这些路径由 `ensure_basic_paths()` 和 `ensure_busybox_links()` 创建，属于兼容 BusyBox、glibc/musl 动态加载器和 LTP 运行环境的启动准备逻辑。

在开启 `metadata_csum` 的 ext4 镜像上，当前 vendor lwext4 集成执行目录创建后，宿主 Linux 重新检查或挂载镜像时可能失败：

```text
mount: /mnt/sdcard-rv-1: mount(2) system call failed: Structure needs cleaning.
```

`e2fsck -fn` 的典型输出是：

```text
Directory inode 3220, block #1: directory passes checks but fails checksum.
Directory inode 3221, block #1: directory passes checks but fails checksum.
Directory inode 3222, block #1: directory passes checks but fails checksum.
```

`debugfs ncheck` 反查显示这些 inode 对应启动阶段新建的 `/dev/shm`、`/tmp`、`/root` 等目录。

## 选择方案

本轮选择方案 2：关闭测试镜像的 `metadata_csum`，并继续允许内核侧 mkdir。

推荐只在测试副本上操作：

```bash
rm -f /tmp/sdcard-rv-nocsum.img
cp --reflink=auto sdcard-rv.img /tmp/sdcard-rv-nocsum.img
e2fsck -fy /tmp/sdcard-rv-nocsum.img
tune2fs -O ^metadata_csum /tmp/sdcard-rv-nocsum.img
e2fsck -fn /tmp/sdcard-rv-nocsum.img
dumpe2fs -h /tmp/sdcard-rv-nocsum.img | rg "Filesystem features|Filesystem state"
```

预期结果：

```text
Filesystem features:      ... 不包含 metadata_csum
Filesystem state:         clean
```

不要直接修改仓库中的基线 `sdcard-rv.img`，除非明确要更新项目分发镜像。

## 为什么不改成 ramfs

把 `/etc`、`/dev`、`/tmp`、`/root` 挂成 ramfs 可以避免运行时目录写入 ext4，短期能绕开 checksum failure。但它会改变系统语义：

- `/etc/passwd`、`/etc/group`、`/tmp` 内容关机后不再留在镜像中。
- `/bin`、`/usr/bin`、`/lib` 等 BusyBox/动态链接器兼容路径仍需要额外处理。
- 测试环境和真实 ext4 根目录的行为差距变大，后续排查路径问题更容易混淆。

当前更想保留的语义是：内核启动兼容层确实把运行时路径补进 ext4 根文件系统。因此问题应放到镜像特性兼容层处理，而不是把运行时路径迁移到临时文件系统。

## vendor checksum 问题记录

vendor lwext4 不是完全没有 metadata checksum 代码。当前源码里可以看到：

- `vendor/lwext4_rust/c/lwext4/README.md` 声明 `metadata_csum: yes`
- `vendor/lwext4_rust/c/lwext4/include/ext4_config.h` 定义 `CONFIG_META_CSUM_ENABLE`
- `vendor/lwext4_rust/c/lwext4/src/ext4_dir.c` 包含 `ext4_dir_csum_verify()`、`ext4_dir_set_csum()`、`ext4_dir_init_entry_tail()`
- `vendor/lwext4_rust/c/lwext4/src/ext4_dir_idx.c` 也有 indexed directory checksum 相关路径

但 rCore-lab 当前集成下，启动 mkdir 后的镜像仍会被 Linux `e2fsck` 判为目录 checksum 错。这说明“vendor 声称支持 metadata_csum”和“当前目录创建路径写出的镜像能被 Linux 接受”不是同一件事。

因此关闭 `metadata_csum` 是一个兼容性 workaround：

- 它不证明 vendor checksum 实现完全缺失。
- 它避免在尚未完全确认的 checksum 路径上写持久元数据。
- 它让内核侧 mkdir、hardlink、shutdown、write-through 等行为可以继续按 ext4 根文件系统语义验证。

## 代价

关闭 `metadata_csum` 的主要代价是失去 ext4 元数据 checksum 提供的额外损坏检测能力。对于本项目的测试镜像，这个代价目前可接受，因为：

- 测试镜像是可重新生成或恢复的开发资产，不是生产数据盘。
- 当前更高优先级是保证 rCore 写过的镜像能被 Linux 稳定检查、挂载和复用。
- `e2fsck -fn` 仍然可以检查目录结构、inode、bitmap、journal 等一致性问题。

需要注意：这不是最终的 ext4 metadata checksum 支持方案。如果未来需要兼容默认 Linux `mkfs.ext4` 镜像，应回到 vendor/lwext4 目录 checksum 的最小复现和修复。

## 验证命令

关闭 checksum 后，用同一份 `/tmp` 镜像副本跑基础回归：

```bash
SINGLE_TEST=/musl/basic/write LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_write.log 2>&1
e2fsck -fn /tmp/sdcard-rv-nocsum.img

SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_mount.log 2>&1
e2fsck -fn /tmp/sdcard-rv-nocsum.img
```

关键验收：

- 测试日志包含 `=== All tests completed ===`
- 关机日志包含 `lwext4 umount Okay`
- 每轮后 `e2fsck -fn` 返回 `0`

非阻塞 virtio 路径也要覆盖：

```bash
VIRTIO_BLK_NON_BLOCKING=1 SINGLE_TEST=/musl/basic/mount LOG=INFO timeout 150 \
  bash run.sh -f /tmp/sdcard-rv-nocsum.img -t rv \
  > /tmp/rcore_nocsum_nb_mount.log 2>&1
rg -n "Panicked|ERROR|All tests completed|mount return|lwext4 umount" \
  /tmp/rcore_nocsum_nb_mount.log
e2fsck -fn /tmp/sdcard-rv-nocsum.img
```

## 后续工作

1. 做一个最小 mkdir repro：同一份开启 `metadata_csum` 的 ext4 镜像，分别由 Linux 和 vendor lwext4 创建目录，比较目录块 tail、inode generation、fs uuid、checksum 输入。
2. 确认 lwext4 在 non-indexed directory、indexed directory split、journal replay 三条路径上是否都正确更新 checksum。
3. 如果 vendor 修复完成，再恢复 `metadata_csum` 镜像并把 `e2fsck -fn` 纳入 rcore-single-test-loop 的固定验证步骤。
