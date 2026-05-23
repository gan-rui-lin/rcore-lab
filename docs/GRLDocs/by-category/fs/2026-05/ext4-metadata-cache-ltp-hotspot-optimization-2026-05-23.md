# ext4 元数据缓存与 LTP syscall 热点优化记录

日期：2026-05-23

## 背景

`hotspot.md` 记录了 RV LTP `LOG=SYSCALL` 运行期间的 syscall 热点。最后一轮快照中，syscall 总数约 44 万，前几项为：

```text
clock_gettime 112651
read          110113
fstatat        55686
write          16034
close          14473
openat          6513
faccessat       4035
unlinkat        2788
setitimer       2712
```

其中 `clock_gettime/read` 是独立热点，本轮不处理。`fstatat/openat/faccessat/unlinkat` 和一组 LTP 元数据用例高度相关，主要经过 VFS 路径解析、权限检查、stat 构造和 ext4 元数据读取。

## 根因结论

提交 `a12c987af4a1d10e7b263089aaa429d225dccaab` 将路径级元数据从内存表改为每次走 ext4 查询/更新，使 `fstatat/access/chmod/chown/utimens/xattr/statfs` 等高频路径变成反复 `open_file -> inode.metadata() -> ext4_raw_inode_fill + ext4_owner_get`。

放大点主要有三类：

1. `inode_for_path()` 先 `path_is_dir()` 再 `open_file()`，一次路径元数据请求会触发多次 VFS resolve。
2. `access_allowed_egid()` 对路径每个分量分别调用 owner/mode helper，间接重复打开路径并读取 ext4 metadata。
3. ext4 `metadata()/size()/xattr/statfs` 每次都直接调用后端 API，没有缓存；`chmod/chown/utimens` 写完后下一次 stat 又会立刻回源。

## 修改策略

本轮采用“混合缓存”策略：ext4 仍是权威源，写操作继续调用 ext4 API；高频读路径先读内核缓存，写成功后修补或失效缓存。

### ext4 inode 层缓存

修改文件：

```text
os/src/fs/vfs/ext4/inode.rs
```

新增缓存：

```text
EXT4_METADATA_CACHE      path -> VfsMetadata
EXT4_XATTR_CACHE         (path, name) -> Option<Vec<u8>>
EXT4_LISTXATTR_CACHE     path -> Vec<u8>
EXT4_STATFS_CACHE        Option<VfsStatFs>
```

关键行为：

- `lookup()` 使用 `ext4_cached_metadata()`，一次 raw metadata 查询同时完成存在性判断和节点类型识别。
- `metadata()` 和 `size()` 改为先查 metadata cache；miss 时才调用 `ext4_raw_metadata()` 并写入缓存。
- `statfs()` 缓存 `ext4_mount_point_stats()` 结果，ext4 写路径会清掉缓存。
- `getxattr/listxattr` 读缓存；`setxattr/removexattr` 成功后更新或失效对应 xattr 缓存。

### 写路径一致性

以下操作成功后会更新或失效相关缓存：

```text
write_at
truncate
truncate_to
create
create_dir
remove
chmod
chown
utimens
link_to
symlink
mknod
setxattr
removexattr
```

其中：

- `chmod/chown/utimens` 仍先调用 ext4 后端，成功后只修补已缓存的 metadata 字段。
- `write_at/truncate_to` 会更新缓存中的 size、blocks、mtime、ctime。
- create/remove/link/symlink/mknod 会按 path 清掉 metadata/xattr/listxattr/statfs 缓存，避免后续 stat 读到旧路径状态。

### syscall 层减少重复查询

修改文件：

```text
os/src/syscall/fs.rs
```

关键改动：

- `inode_for_path()` 不再先调用 `path_is_dir()`，直接 `open_file(path, OpenFlags::empty()).and_then(|file| file.inode())`。
- `path_exists_for_access()` 优先复用 `metadata_for_path()`，避免存在性检查再次 open。
- `access_allowed_egid()` 对完整路径和中间目录分量各取一次 metadata，并基于同一份 metadata 计算目录类型、mode、uid/gid 权限位。
- `fill_regular_stat()` 在 metadata 缺失时不再 fallback 到 `effective_path_owner()`，避免 stat 构造中二次 resolve；缺失 metadata 的非 ext4 fallback 使用 `(0, 0)`。

## 验证记录

第一次直接在 `os` 目录执行：

```bash
cargo check
```

被现有 debug initcode include 文件缺失挡住：

```text
couldn't read .../user/target-user/riscv64gc-unknown-none-elf/debug/initcode
```

因此先生成 debug 用户程序：

```bash
make -C user build MODE=debug TEST=0
```

随后重新检查内核：

```bash
cd os
cargo check
```

结果：

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) ...
```

编译通过。过程中仍有 vendor 里的 `unexpected cfg` warning，例如 `fuzzing/riscv32/riscv64`，这些是既有第三方依赖告警，不是本次改动引入的类型错误。

额外检查：

```bash
git diff --check -- os/src/fs/vfs/ext4/inode.rs os/src/syscall/fs.rs
```

结果无 whitespace error。

## 预期收益

本轮不会改变 syscall 调用次数，所以 `rv-syscall.log` 中 `fstatat/openat/faccessat` 的计数不一定下降。预期改善点是每次 syscall 内部触发的 ext4 raw metadata、owner、xattr、statfs 后端调用显著减少。

最直接受益路径：

- `fstatat/statx/fstat`：metadata cache 命中后不再反复 raw inode + owner 读取。
- `access/faccessat/openat`：权限检查少走多次 `open_file/path_is_dir/metadata`。
- `chmod/chown/utimens` 后的下一次 stat/access：缓存被直接修补，不需要立即回源。
- `xattr`：CREATE/REPLACE 判断与后续 get/list 能复用 cache。
- `statfs/fstatfs`：重复读取 mount stats 命中缓存。

## 已知风险与后续

1. 当前缓存是全局 path cache，适合当前 rCore-lab 单核 `UPIntrFreeCell` 环境；如果后续支持 SMP 或更复杂 mount namespace，需要重新审视同步与命名空间隔离。
2. `renameat2` 当前实现是复制新文件再删除旧文件，底层 create/remove/write 会使关键路径缓存失效或更新；但它不是原子 rename 语义，后续如果改成 ext4 原生 rename，需要同步补 cache move/invalidate。
3. 如果 ext4 后端在内核外被修改，内核缓存不会感知。当前测试模型中镜像不会被宿主并发改写，因此接受这个假设。
4. `clock_gettime/read` 仍是更大的热点，本轮只处理 ext4 元数据回归；后续应单独分析 timer/vdso-like 快路径和 read 路径缓存/批量读。

## 推荐回归命令

功能回归优先跑元数据相关 LTP 子集：

```bash
SINGLE_TEST=/glibc/ltp/syscalls/fstatat01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/access01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/chmod01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/chown01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/utimensat01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/getxattr01 LOG=INFO timeout 120 bash run.sh -t rv
SINGLE_TEST=/glibc/ltp/syscalls/statfs01 LOG=INFO timeout 120 bash run.sh -t rv
```

性能对比继续使用 `hotspot.md` 中的统计方式：

```bash
date -Is
wc -l rv-syscall.log
awk '/\[syscall\]/ {c++} END{print "syscall lines:",c}' rv-syscall.log
awk '/\[syscall\]/ { if (match($0, /num=[0-9]+\(([A-Za-z0-9_]+)\)/, a)) print a[1] }' rv-syscall.log \
  | sort | uniq -c | sort -nr | head -n 30
```

如果需要确认内部收益，可以临时给 `ext4_raw_metadata()` 加计数日志，对比优化前后 raw metadata 调用量；验证完应移除或默认关闭该日志。
