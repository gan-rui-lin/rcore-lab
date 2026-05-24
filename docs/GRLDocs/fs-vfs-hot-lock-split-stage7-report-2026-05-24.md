# FS/VFS 文件层热点锁拆分与访问点收敛报告（stage7）

## 摘要

本次 stage7 聚焦 FS/VFS「文件对象层（file / fd state）与全局 VFS 根对象」的热点锁形态：把原先若干“把多个语义域字段捆在一起”的粗锁，拆成更贴近访问者语义的细粒度锁，并通过 helper 收敛访问入口，减少调用点对内部字段布局/锁类型的耦合。

提交：`3a5e893626154263db3b8bd548bc5c2d462997f2`（perf(fs):FS/VFS 文件层热点锁拆分）

核心变化概览：

- VfsFile：移除 `VfsFileInner { offset, inode }` 粗锁，改为 `inode: Arc<dyn VfsInode>` + `offset: UPIntrMutex<usize>`。
- Pipe：Pipe 结构改成 `UPIntrMutex<PipeBuffer>`，`nonblock` 改成 `AtomicBool`；阻塞前仍显式释放 pipe 锁。
- MemFd：inode 数据改成 `UPIntrRwLock<MemFdData>`，fd offset 改成 `UPIntrMutex<usize>`。
- ROOT_VFS：改成 `UPIntrRwLock<Vfs>`；新增 `with_root_vfs_read/write` helper，替换直接 `ROOT_VFS.exclusive_access()`。
- ext4 cache：保留底层 `UPIntrFreeCell`，新增 `xattr/listxattr/statfs` cache helper（get/set/remove/invalidate/update 等）做访问收敛。

整体仍然保持当前并发模型（单核 + 中断屏蔽/UPIntr* 系列），不引入 SMP 级 spin lock 或 sleep lock。本次是“锁粒度与访问边界”的工程性优化，目标是：让常规读写路径只拿必须的锁，并且让会阻塞的路径在阻塞前释放锁，降低热点路径之间的互相牵制。

## 背景问题

在 stage7 之前，若干 FS/VFS 代码存在典型的“结构体内部把多类状态用同一把锁保护”的模式：

1. **文件对象层把 inode 与 offset 绑在一起**：
   - 例如 VfsFile 用一个 `UPIntrFreeCell<VfsFileInner>` 同时保护 `offset` 与 `inode`。
   - 这种设计的直接代价是：只要发生读写（需要更新 offset），就不得不把“inode 指针/引用”的访问也纳入同一临界区；而 inode 本身通常是 `Arc<dyn VfsInode>`，并不一定需要和 offset 同步。

2. **Pipe 的锁覆盖范围偏大且 nonblock 也占用锁资源**：
   - pipe buffer 与 open 状态、读写位置等需要互斥；但 nonblock 只是一个轻量 flags。
   - 如果 nonblock 也放在 cell/锁里，会让 `status_flags()` / `set_status_flags()` 这类高频、低代价操作也需要进入临界区。
   - 更重要的是：pipe 的 read/write 在“等待可读/可写”时会调度让出 CPU，如果忘记在阻塞前释放 pipe 锁，会造成非常隐蔽的锁持有跨调度问题。

3. **MemFd 的数据与 offset 同样存在粗锁耦合**：
   - inode 数据（buf/size/seals 等）与 fd 的 seek offset 是两类状态：前者是 inode 级（可被多个 fd/引用共享），后者是“文件描述符级”的游标。
   - 将 inode 数据做成独占访问（UPIntrFreeCell）会把所有读路径都串行化；而在当前模型下，即便并发有限，也会让“只读/只写”访问的语义边界不清晰。

4. **ROOT_VFS 全局对象过去只提供 exclusive_access（写锁语义）**：
   - 大量路径只是 resolve/lookup（纯读逻辑），但也必须走“独占访问”。
   - 调用点会自然出现“拿着 ROOT_VFS 的独占借用做一堆 resolve，然后再做别的事”的模式，导致临界区膨胀，且不利于后续把读写访问分层。

5. **ext4 cache 的访问散落在各处**：
   - 缓存容器仍然是 `UPIntrFreeCell`，但如果 callsite 直接操作 map（get/insert/remove/retain），很容易出现：key 归一化不一致、缓存失效点漏掉、写路径忘记 invalidate statfs 等。
   - 此类问题在 iozone / 大量文件元数据访问场景下会放大，既影响性能，也增加一致性风险。

一句话根因：多个 FS/VFS 热点对象把“不同语义域”的字段捆在同一把锁里，并且缺少统一的访问入口，导致临界区过大、阻塞路径容易持锁跨调度、缓存一致性策略难以收敛。

## 改动范围

本次提交涉及 6 个文件（与 commit `3a5e8936...` 一致）：

- `os/src/fs/vfs/file.rs`
- `os/src/fs/pipe.rs`
- `os/src/fs/memfd.rs`
- `os/src/fs/vfs/core.rs`
- `os/src/fs/vfs/mount.rs`
- `os/src/fs/vfs/ext4/inode.rs`

## 证据与定位路径（可复现）

本报告中的“前后对比”依据来自以下命令输出：

```bash
cd rcore-lab
# 1) 查看提交内容与文件列表
git show 3a5e893 --name-only

# 2) 查看关键 diff（示例：VfsFile）
git show 3a5e893 -- os/src/fs/vfs/file.rs

# 3) 编译验证
make all
```

其中 `git show` 明确展示：

- VfsFile 从 `inner: UPIntrFreeCell<VfsFileInner>` 变为 `inode + offset mutex`。
- ROOT_VFS 从 `UPIntrFreeCell<Vfs>` 变为 `UPIntrRwLock<Vfs>` 并新增 `with_root_vfs_read/write`。
- Pipe 从 `UPIntrFreeCell<Pipe> + UPIntrFreeCell<bool>` 变为 `UPIntrMutex<PipeBuffer> + AtomicBool`。
- MemFd 从 `UPIntrFreeCell<MemFdInner> + UPIntrFreeCell<usize>` 变为 `UPIntrRwLock<MemFdData> + UPIntrMutex<usize>`。
- ext4 inode 中新增并替换了 xattr/listxattr/statfs 相关缓存 helper。

## 核心改动

### 1) VfsFile：inode 与 offset 解耦，offset 单独互斥

改动前：

- `VfsFile` 内部有一个 `VfsFileInner`，包含 `offset` 与 `inode`，整体用 `UPIntrFreeCell` 做独占访问。

改动后：

- `VfsFile` 直接持有不可变的 `inode: Arc<dyn VfsInode>`。
- `offset` 变成单独的 `UPIntrMutex<usize>`。

当前实现形态（示意）：

```rust
pub struct VfsFile {
    inode: Arc<dyn VfsInode>,
    offset: UPIntrMutex<usize>,
    // readable/writable/flags/path/ts_id ...
}
```

语义收益：

- 读写路径只在需要更新文件游标时进入 offset 临界区；inode 访问不再被“顺带”纳入同一锁域。
- callsite 更接近“fd offset 是 fd 私有状态”的真实语义。
- 也为后续进一步做 `pread/pwrite`（不动 offset）路径提供更清晰的实现基础。

此外，本次也把 VFS resolve 相关调用从直接拿 ROOT_VFS 的 exclusive access，迁移为通过 `with_root_vfs_read` 取 root/resolve（见后述第 4 点）。

### 2) Pipe：PipeBuffer 用 Mutex；nonblock 用 AtomicBool；阻塞前释放锁

改动后 pipe 的关键结构如下：

```rust
pub struct PipeEnd {
    pipe: Arc<UPIntrMutex<PipeBuffer>>,
    nonblock: AtomicBool,
    // readable/writable ...
}
```

关键点：

- `PipeBuffer`（原先 Pipe）使用 `UPIntrMutex` 进行互斥，表达“读写缓冲区与 open 状态需要互斥”。
- `nonblock` 改成 `AtomicBool`：
  - `status_flags()`/`set_status_flags()` 不再需要进入临界区，降低不必要的锁竞争。
  - 使用 `Ordering::Relaxed`，因为它只是一个 flags；在单核 + 中断屏蔽模型下，这个选择足够且代价更低。
- read/write 的阻塞逻辑仍然遵循：**阻塞前必须 drop(pipe 锁)**。
  - 例如读端在 buffer 为空且对端仍 open 时，如果不是 nonblock 且无 pending signal，就 `drop(pipe); suspend_current_and_run_next();`。
  - 这样可以避免“持有 pipe 锁跨调度”，降低系统卡死/饥饿风险。

另外，Pipe 的 `write_user_buffer()` 在发现读端关闭且 total==0 时注入 `SIGPIPE` 并返回 `EPIPE_ERRNO` 的语义保持不变；本次改动主要是锁形态与 nonblock flags 的表达方式。

### 3) MemFd：inode 数据使用 RwLock；fd offset 单独 Mutex

MemFd 分成两层：

- inode 层：`MemFdInode { inner: UPIntrRwLock<MemFdData> }`
- fd 层：`MemFdFile { inode: Arc<MemFdInode>, offset: UPIntrMutex<usize> }`

其中 `MemFdData` 包含 `buf/size/seals/allow_sealing`。

关键收益：

- `read_at()` 使用 `inner.read()`，更明确地区分“只读路径”和“写路径”。
- `write_at()` / `truncate()` / `add_seals()` 使用 `inner.write()`。
- fd offset 独立后，逻辑更接近 Linux：offset 是 file description（fd）私有状态；inode 数据是共享状态。

本次还把写入逻辑抽成 `write_data_at(&mut MemFdData, ...)`，用于 inode 写入与 fd 写入复用，并把 seal（F_SEAL_WRITE）检查集中在同一处，减少重复与分叉。

### 4) ROOT_VFS：引入 RwLock + with_root_vfs_read/write 收敛访问入口

改动前：

- `ROOT_VFS: UPIntrFreeCell<Vfs>`，调用点普遍 `ROOT_VFS.exclusive_access()`。

改动后：

- `ROOT_VFS: UPIntrRwLock<Vfs>`，并新增：

```rust
pub(crate) fn with_root_vfs_read<R>(f: impl FnOnce(&Vfs) -> R) -> R
pub(crate) fn with_root_vfs_write<R>(f: impl FnOnce(&mut Vfs) -> R) -> R
```

改动影响：

- `mount.rs` 中 mount/shutdown/sync 等路径改为：
  - mount / shutdown：走 `with_root_vfs_write`。
  - flush：走 `with_root_vfs_read`。
- `file.rs` 中 open/resolve/exists/create_dir 等路径也改用 `with_root_vfs_read` 来做 resolve。

设计上的收益不仅是“读写锁”的形式，更重要的是 **访问点收敛**：

- 让调用者只能通过 closure 的方式短持锁，降低“拿到 vfs 借用后又在外层做大量逻辑”的倾向。
- 在未来如果需要在 helper 内做额外约束（例如统计锁持有时间、注入 debug 检查、统一错误日志策略），调用点无需大改。

### 5) ext4 cache：新增 helper，统一 key 归一化与失效策略

ext4 inode 内本来已经有多个 cache（metadata/xattr/listxattr/statfs），底层仍采用 `UPIntrFreeCell`。

本次的关键变化是：把原先散落在调用点的 map 操作收敛成 helper：

- `ext4_xattr_cache_get/set/remove_path`
- `ext4_listxattr_cache_get/set/remove`
- `ext4_statfs_cache_get/set/invalidate`
- `ext4_metadata_cache_get/set/remove/update`
- 以及统一失效入口：`ext4_cache_remove_path()`、写触摸：`ext4_cache_touch_write()` / `ext4_cache_touch_write_extend()`

并在 `getxattr/setxattr/listxattr` 等路径中，把直接 `EXT4_*_CACHE.exclusive_access().insert/remove/get` 的写法替换为 helper，从而：

- 统一使用 `normalize_path()` 作为 cache key（避免不同调用点对 path 处理不一致）。
- 明确在写路径 invalidate `statfs` cache（避免容量统计长期不刷新）。
- 把“写触摸更新 mtime/ctime/size/blocks”的策略集中起来，减少漏改。

## 收益总结

1. **锁粒度更贴近语义域**：
   - VfsFile：offset 与 inode 解耦。
   - MemFd：inode 数据与 fd offset 解耦。
   - Pipe：buffer/open 状态与 nonblock flags 解耦。

2. **降低热点路径互相牵制**：
   - ROOT_VFS 读路径不再强制走独占访问；并且通过 `with_root_vfs_*` 让持锁范围更小、更可控。

3. **阻塞路径更安全**：
   - pipe 的阻塞前 drop 锁逻辑保持并更清晰，减少“持锁跨调度”的风险。

4. **缓存一致性策略更集中**：
   - ext4 cache helper 把 key/失效/更新集中封装，降低 callsite 漏 invalidate 的概率。

## 已知边界与风险

1. **当前并发模型仍是单核 + 中断屏蔽**：
   - `UPIntrRwLock` 在当下不一定带来“真实并行读”，但它让读写语义更清晰，也能减少调用点写成独占借用的惯性。

2. **Pipe 的错误返回仍采用 sentinel 方式**：
   - `EAGAIN`/`EINTR` 在 `read()` 中用 `usize::MAX-1/usize::MAX` 作为 sentinel（与现有 File trait 约定相关）。
   - 这部分不是本次提交引入的，但后续如果要更严格对齐 Linux errno 传播，可能需要统一 trait 返回类型或集中转换。

3. **ext4 cache 切换为 UPIntrRwLock，但未引入过期/淘汰策略**：
   - 读路径共享读锁、写路径独占写锁，降低热点读访问的串行化。
   - 仍需通过实际回归确认性能收益与无副作用。

4. **MemFd write 路径的锁组合**：
   - `MemFdFile::write()` 当前会同时持有 offset mutex 与 inode inner write（先锁 offset，再锁 inode）。
   - 在当前单核模型下风险较低，但建议后续明确 FS 层锁顺序约定，避免未来其他路径反向获取导致潜在死锁模式（尤其如果引入 sleep lock 或多核）。

## 验证结果

本次在提交 `3a5e8936...` 上执行编译验证：

```bash
cd rcore-lab
make all
```

结果：编译通过（`Finished release profile [optimized]`）。输出中存在来自 vendor 依赖（例如 riscv/smoltcp）的 `unexpected_cfgs` 警告，但与本次 FS/VFS 改动无直接关联。

由于本次提交定位为锁形态/访问收敛的结构性调整，建议在后续回归中补充至少以下覆盖面：

- Pipe 行为：阻塞/非阻塞、EPIPE/SIGPIPE、EINTR。
- MemFd：基本 read/write/seek、seal 行为（F_SEAL_WRITE）与并发读写交错。
- VfsFile：常见 open/read/write/append/truncate；以及 `exec()` 读 ELF 的 `read_all()` 性能/正确性。
- ext4：xattr/listxattr/statfs 的一致性与缓存失效；iozone 场景下元数据/写入触摸更新。

## 实现补充：并发安全 / 性能 / 可维护性

### 并发安全

- **锁顺序明确化**：ROOT_VFS 仅用于 mount 表与 resolve 入口，严禁持锁进入 inode ops 或 lwext4；fd offset 锁不跨可能阻塞路径。
- **pipe 阻塞前释放锁**：read/write/write_user_buffer 在需要 `suspend_current_and_run_next()` 前均确保已释放 pipe buffer 锁，避免持锁跨调度。
- **cache 锁不跨 lwext4**：ext4 cache helper 保证数据拷贝后再进入 lwext4 调用，避免锁持有覆盖潜在阻塞/IO。

### 性能

- **read_at/write_at 绕过共享 offset lock**：新增 VfsFile `read_at`/`write_at`，`read_all` 使用显式 offset 批量读，降低热点读路径对 offset mutex 的依赖。
- **ext4 cache 读路径共享读锁**：ext4 metadata/xattr/listxattr/statfs cache 使用 `UPIntrRwLock`，读路径并发共享读锁，写路径独占更新。

### 可维护性

- **pipe guard 化的阻塞规则**：读写路径以“锁域 + 阻塞点分离”形式编码，防止未来改动把 `suspend` 放回持锁区。
- **cache helper 固化规则**：统一 key 归一化、失效策略与写触摸更新，降低 callsite 漏改风险。
- **lock-order 文档化**：ROOT_VFS 与 inode/pipe/cache 的锁序规则通过注释固定，后续演进更不易踩坑。

## 后续建议

1. **加入静态约束，防止 ROOT_VFS 访问倒退**：
   - 在 `os/src/fs/vfs/` 或更高层增加 lint/CI 检查，禁止重新引入 `ROOT_VFS.exclusive_access()` 风格的长持锁访问。

2. **统一 FS 层锁顺序约定**：
   - 明确“fd offset 锁”和“inode 内部锁”的获取顺序，避免未来演进引入死锁模式。

3. **为 pipe nonblock/EINTR 增加回归用例**：
   - 尤其是“阻塞读/写过程中收到信号”与“读端关闭触发 SIGPIPE + EPIPE”的组合场景。

4. **ext4 cache 增加 debug/统计开关**：
   - 在 debug 模式下增加 cache hit/miss 计数或关键 invalidate 日志，便于 iozone 场景下定位性能变化是否来自缓存策略。
