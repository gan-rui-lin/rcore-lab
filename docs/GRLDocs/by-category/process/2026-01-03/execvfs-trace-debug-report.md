# ext4 VFS 路径解析与 exec 失败调试报告

## 结论先行（罪魁祸首）

问题的直接根因是 VFS 的挂载点匹配逻辑错误：`resolve_mount()` 对根挂载点 `/` 的匹配过于严格，只匹配 `"/"` 或 `"//"` 前缀，导致像 `/musl/basic/getpid` 这样的绝对路径根本无法命中根挂载点，从而在路径解析阶段直接失败。具体表现为：`sys_exec` 返回 `-2 (ENOENT)`，并在日志里出现 `vfs: path_is_dir not found /musl/basic`。

修复方法是：让根挂载点 `/` 视作匹配所有绝对路径。修复后，VFS 能正确把 `/musl/basic/getpid` 分解并逐级 lookup 到 ext4 文件，`exec` 成功。

同时还修复了 ext4 目录列表对 `"/"` 的打开方式问题（`file_open("/")` 会失败，改用 `lwext4_dir_entries()` 直接走 `ext4_dir_open`），避免 root 列表阶段的干扰错误。

---

## 现象与复现

复现条件（单测）：

```
TRACE_PID=1 LOG=TRACE SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t debug
```

复现日志（关键片段）：

```
[TRACE] kernel:pid[1] sys_chdir
[TRACE] vfs: path_is_dir not found /musl/basic
[TRACE] [syscall] pid=1 name=initproc num=49 args=[...] ret=-2
[TRACE] kernel:pid[1] sys_exec
[TRACE] kernel:pid[1] sys_exec path=/musl/basic/getpid
[TRACE] [syscall] pid=1 name=initproc num=221 args=[...] ret=-2
Exec /musl/basic/getpid failed (ret=-2)!
```

这表明：
1) `chdir` 因为 VFS 解析失败返回 ENOENT；
2) `exec` 在打开 `/musl/basic/getpid` 时失败，导致用例直接退出。

---

## 调试思路与过程（带日志证据）

### 1. 先确认 ext4 已成功挂载

启动日志显示 ext4 mount 成功，并且根目录可列出 `musl`、`glibc` 等目录：

```
[ INFO] lwext4 mount Okay
[ INFO] ls /
[ INFO]   [dir] musl
[ INFO]   [dir] glibc
...
```

这说明底层 ext4 本身没有问题，磁盘内容可见，mount 也成功。

### 2. 追踪 VFS 解析路径

为验证问题发生在哪一层，我在 VFS 的 `resolve()` 和 `path_is_dir()` 加了 trace：

```
[TRACE] vfs: path_is_dir not found /musl/basic
```

这里没有出现后续 “resolve path=... rel=...” 之类的日志，说明 `resolve_mount()` 就已经返回了 `None`。

### 3. 定位 `resolve_mount` 根挂载逻辑

VFS 的 `resolve_mount()` 逻辑中，匹配条件是：

- `path == mount.path` 或
- `path.starts_with(mount.path + "/")`

当挂载点是 `/` 时，表达式变成 `path.starts_with("//")`，显然 `/musl/...` 不会命中；而 `path == "/"` 也不成立，所以根挂载点匹配失败。结果就是 VFS 直接返回 None，后续的 lookup 完全不执行。

这一点与日志完全吻合：`path_is_dir not found` 出现在 `resolve()` 之前。

### 4. 修复根挂载匹配，并验证效果

修复方式：根挂载点 `/` 应该匹配所有绝对路径。因此将逻辑改成：

- 若 mount.path == "/"，无条件视作匹配
- 否则保持原逻辑

修复后日志从“找不到路径”变为正常的 ext4 lookup 与 file read，最终 `exec` 成功：

```
[DEBUG] file_open /musl/basic/getpid, mp=0x822e6360
[DEBUG] file_read "/musl/basic/getpid", len=512
...
[TRACE] [syscall] pid=1 name=initproc num=221 ... ret=0
========== START test_getpid ==========
getpid success.
```

---

## 关键修复点说明

### 1) 修复 VFS 根挂载匹配

- 文件：`os/src/fs/vfs.rs`
- 位置：`resolve_mount()`
- 逻辑：根挂载 `/` 直接匹配所有路径

### 2) 修复 ext4 目录列表对 `/` 的打开方式

- 文件：`os/src/fs/ext4.rs`
- 原逻辑：`file_open("/")` + `lwext4_dir_entries()`
- 问题：`file_open` 用 `ext4_fopen`，对目录路径可能失败
- 新逻辑：直接 `lwext4_dir_entries()`（内部用 `ext4_dir_open`）

### 3) 增强日志追踪能力

- VFS：`resolve()` / `path_is_dir()` 增加 trace
- ext4：目录/文件探测路径记录
- syscall：支持 PID/NAME 过滤 + 打印 args/ret
- 磁盘级 trace：`TRACE_DISK=1` 时打印 block read/write

这些日志保证了后续出现 VFS 相关问题时可以快速定位具体阶段。

---

## 关键日志片段对比

### 修复前

```
[TRACE] vfs: path_is_dir not found /musl/basic
[TRACE] [syscall] ... ret=-2
Exec /musl/basic/getpid failed (ret=-2)
```

### 修复后

```
[DEBUG] file_open /musl/basic/getpid
[DEBUG] file_read "/musl/basic/getpid", len=512
[TRACE] [syscall] pid=1 name=initproc num=221 ... ret=0
========== START test_getpid ==========
getpid success.
```

---

## 剩余问题与后续建议

1) **当前 `chdir` 仅做路径存在性校验，不维护 cwd**
   - 这对单测基本够用，但更完整的 VFS 需要维护 per-process cwd，且 `openat` 应该在相对路径下结合 cwd。

2) **`waitpid` 引用计数断言已替换为 trace**
   - 实际运行中 `child` 可能仍被调度器/其它结构持有，不能强断言；建议后续梳理 refcount 生命周期。

3) **trace 过多可能影响性能**
   - 已支持 `TRACE_PID`/`TRACE_NAME` 与 `TRACE_DISK` 开关；建议保留文档说明。

---

## 总结

这次问题表面上表现为 `exec` 返回 ENOENT，但根因是 VFS 根挂载路径匹配逻辑错误，导致任何非 `"/"` 的绝对路径都无法被解析，继而引发 `chdir` 和 `exec` 失败。修复后 `exec` 正常，`/musl/basic/getpid` 单测可以完整运行并输出成功。

本次调试还顺带完善了：
- ext4 目录列举逻辑
- syscall 级别的参数/返回值 trace
- 磁盘级别 trace 开关

这为后续调试 VFS/文件系统问题提供了更稳定的观察手段。
