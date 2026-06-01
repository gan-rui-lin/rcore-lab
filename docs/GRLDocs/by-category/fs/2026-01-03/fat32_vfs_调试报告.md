# FAT32 下 chdir/mkdir/pipe 失败与 panic 的调试报告

> 这是一份**调试任务**报告，首先明确“罪魁祸首”，并结合具体 debug 输出给出逻辑化分析，再说明修复方案与验证路径，最后补充背景知识与可扩展建议。

## 一、调试结论（罪魁祸首）

**罪魁祸首有两个：**

1. **`sys_mkdirat` 仍然硬编码走 ext4 的 `ext4_dir_mk`，而非 VFS 抽象**，导致在 FAT32 挂载为根时目录创建失败；进而 `chdir` 失败。该问题直接由日志中的 `ext4_dir_mk` 错误输出定位。
2. **`sys_waitpid` 对 `exit_code_ptr` 进行无条件写入，未处理 `wait(NULL)` 的合法场景**，导致在 `pipe` 测试中 `wait(NULL)` 触发空指针写入，最终在 `address.rs:187` 的 `unwrap()` 处 panic。该问题由 panic 栈与 `waitpid` 日志组合定位。

这两个问题分别对应**功能性失败（mkdir/chdir）**与**内核 panic（pipe）**，是本次测试失败的主要原因。

---

## 二、关键日志与推理过程

### 1) chdir / mkdir 失败的证据链

日志片段（摘要）：

```
[TRACE] kernel:pid[2] sys_mkdirat
[TRACE] vfs: resolve path=/test_chdir rel=test_chdir
[TRACE] vfs: resolve failed at test_chdir for /test_chdir
[TRACE] vfs: path_is_dir not found /test_chdir
[DEBUG] directory create: /test_chdir
[ERROR] ext4_dir_mk: rc = 2
[TRACE] [syscall] pid=2 name=chdir ... ret=-5
[TRACE] kernel:pid[2] sys_chdir
[TRACE] vfs: resolve path=/test_chdir rel=test_chdir
[TRACE] vfs: resolve failed at test_chdir for /test_chdir
[TRACE] [syscall] pid=2 name=chdir ... ret=-2
chdir ret: -2
```

从测试源码可以确认：`chdir` 测试会先 `mkdir("test_chdir")`，随后 `chdir("test_chdir")`。

- 这里 `mkdir` 返回 `-5`（`EIO`），说明**目录创建本身失败**；而 `chdir` 返回 `-2`（`ENOENT`），说明目录确实没被创建。
- 日志里出现 `[ERROR] ext4_dir_mk: rc = 2`，说明**内核试图走 ext4 的创建逻辑**，而此时根文件系统已是 FAT32。
- 关键证据是 `vfs: mounted fat32 as root` + `ext4_dir_mk`，这说明**syscall 层没有走 VFS 抽象，而是被硬编码锁死在 ext4**。

因此，**mkdir/chdir 失败的根因就是 `sys_mkdirat` 仍绑定 ext4 分支，未走 VFS**。

### 2) pipe 测试触发 panic 的证据链

日志片段（摘要）：

```
[TRACE] kernel:pid[4] sys_close
[TRACE] kernel:pid[4] waitpid: child pid 5 has 2 refs
[kernel] Panicked at src/mm/address.rs:187 called `Option::unwrap()` on a `None` value
```

而 `pipe` 测试源码是：

```c
wait(NULL);
```

即它会调用 `wait(NULL)`，在 Linux 语义里这是合法的：表示父进程不关心子进程退出码。

但内核中 `sys_waitpid` 原实现为：

```
*translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
```

这里完全**未检查 `exit_code_ptr` 是否为空**。当 `wait(NULL)` 进入时，`exit_code_ptr` 为 0，`translated_refmut` 会去做 VA → PA 映射并 `unwrap`，最终在：

```
os/src/mm/address.rs:187
unsafe { (self.0 as *mut T).as_mut().unwrap() }
```

触发 panic。

因此，**pipe 的 panic 根因是 `sys_waitpid` 写空指针导致的非法地址访问**。

---

## 三、修复思路与实现要点

### 1) mkdir/chdir：让 VFS 承担目录创建语义

核心原则：**syscall 层不绑定具体文件系统，而应该透过 VFS 的抽象统一访问**。

#### 修复路径：
1. **在 `VfsInode` 上新增 `create_dir` 接口**，为所有 FS 提供统一目录创建入口。
2. **为 easy-fs / ext4 / fat32 分别实现 `create_dir`**：
   - easy-fs：新增 `Inode::create_dir`，初始化 `DiskInodeType::Directory`。
   - ext4：调用 `Ext4File::dir_mk`。
   - fat32：调用 `fatfs::Dir::create_dir`。
3. **在 VFS 层新增 `create_dir(path)`**，进行路径解析并调用 `parent.create_dir`。
4. **sys_mkdirat 改为走 `create_dir`**，彻底移除 ext4-only 的硬编码路径。

这样可保证：**无论根挂载的是 FAT32/ext4/easy-fs，mkdir 都能通过统一逻辑创建目录**。

### 2) pipe panic：wait(NULL) 的安全处理

修复思路：**`sys_waitpid` 在写 exit_code 前判断 `exit_code_ptr` 是否为空**。

伪代码：

```
if !exit_code_ptr.is_null() {
    *translated_refmut(...) = exit_code;
}
```

这样保持了 Linux 语义：`wait(NULL)` 合法且不写回退出状态，同时避免空指针映射。

---

## 四、验证要点与预期结果

### 1) mkdir/chdir 测试

预期：
- `mkdir ret: 0`
- `chdir ret: 0`
- `getcwd` 输出包含 `/test_chdir`

验证路径：
1. FAT32 挂载为根后执行 `chdir` 测试
2. 观察 log 中不再出现 `ext4_dir_mk` 相关错误
3. 若 `test_chdir` 通过，`mkdir` 应该同样通过

### 2) pipe 测试

预期：
- `Write to pipe successfully.` 能完整输出
- 不再出现 `address.rs:187` panic

验证路径：
1. 运行 `pipe` 测试
2. 确认 `wait(NULL)` 执行后内核继续运行

---

## 五、背景知识补充

1. **VFS 的职责是统一文件系统接口**，而不是让 syscall 去选择 ext4/fat32。否则每加入一种 FS 都要重写 syscall 分支。
2. **mkdir/chdir 是典型的“目录语义”操作**，如果在 syscall 层做 FS 绑定，就会使 FAT32 等 FS 无法适配。
3. **wait(NULL) 在 Unix 语义下是允许的**，它只是不要求返回退出码。内核必须支持该用法，否则用户程序测试会稳定失败。

---

## 六、后续建议与潜在扩展

1. **进一步完善 VFS inode 的目录语义**：
   - 当前 VFS 已支持 `create_dir`，但 `unlink`/`rmdir` 类接口仍可统一抽象，以避免 syscall 绑定特定 FS。

2. **补充 FAT32 的元数据与权限语义**：
   - 目前 mkdir 只创建目录，不处理权限；若将来接入更严格的权限模型，可增加 `mode` 传递与校验。

3. **增加 wait 系统调用的健壮性**：
   - `waitpid` 空指针检查已经修复，但仍可以增加 `translated_refmut` 的安全 wrapper，避免类似问题在其它 syscall 再出现。

---

## 七、小结

本次调试的两大“罪魁祸首”分别是：

- **`sys_mkdirat` 硬编码 ext4 导致 FAT32 目录创建失败**，引发 `chdir` 失败；
- **`sys_waitpid` 未处理 `wait(NULL)` 导致空指针写入 panic**。

修复后：
- 目录创建通过 VFS 统一抽象，FAT32/ext4/easy-fs 均能正确 `mkdir`；
- `wait(NULL)` 不再触发 panic；
- `chdir` 与 `pipe` 测试能继续执行，避免系统中途崩溃。

这两个问题的共同点是“**过度依赖具体实现，缺乏抽象与边界检查**”。修复后系统行为更符合 Unix 语义，也更利于未来继续扩展文件系统适配与测试套件。
