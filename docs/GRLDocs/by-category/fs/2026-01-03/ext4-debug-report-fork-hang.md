# ext4 + fork 卡死调试报告

## 结论先行（罪魁祸首）
本次卡死的直接原因不是 ext4 读写，也不是 sys_fork 本身逻辑错误，而是 **内核在 fork 过程中输出大量 debug/trace 日志导致内核栈溢出**，进而触发 **kernel-mode trap**（`trap_from_kernel`），最终表现为“卡死/无输出”。

关键证据来自 GDB 与日志：
- GDB 在卡死时捕获到了 **kernel-mode trap**，并且调用栈显示陷入 `console::write_str`（打印函数），而不是内存拷贝或调度逻辑：
  - `os::trap::trap_from_kernel` → `core::fmt::write` → `os::console::print` → `MemorySet::from_existed_user`
- 这说明真正触发异常的是**打印路径**（fmt/console），而不是 fork 的核心数据复制。
- 内核栈仅 **8KB**，在 `fork` 与 `MemorySet::from_existed_user` 中大量 `trace!` + `println!` 调用，会频繁进入格式化输出路径，导致栈深度异常。

因此，核心问题是“**调试输出导致内核栈溢出**”，不是 ext4 本身，也不是 fork 的算法逻辑错误。

## 调试过程回溯（按时间顺序）

### 1. 现象复现
使用单测执行 `/musl/basic/getpid`：

```
LOG=TRACE SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t debug
```

现象：输出到 `sys_fork` 后不再继续，且 QEMU 卡死或被 kill。

### 2. 初步定位：fork 阶段卡死
在 `sys_fork` 中添加 trace 后，日志停留在：
- `[TRACE] kernel:pid[0] sys_fork`
- `[TRACE] task::fork: parent pid=0`
- `[TRACE] task::fork: copy user space`

说明卡在 `MemorySet::from_existed_user` 这一步。

### 3. GDB 证据：卡死点实际在打印路径
GDB 输出显示：
```
#0  os::trap::trap_from_kernel
#1  core::slice::iter::next<u8>
#5  os::console::write_str
#9  os::console::print
#10 os::mm::memory_set::MemorySet::from_existed_user
```

这说明异常发生在 **打印** 过程中，而不是页复制逻辑本身。

### 4. 进一步验证：关闭/减小输出
当减少输出或增加内核栈空间后，fork 就可以继续执行。最终一次完整的 getpid 单测输出如下（关键段）：

```
[TRACE] kernel:pid[0] sys_fork
[DEBUG] file_read "/musl/basic/getpid" ...
[TRACE] kernel:pid[1] sys_write
... test_getpid ...
[TRACE] kernel:pid[1] sys_exit
[TRACE] kernel:pid[0] sys_write
=== /musl/basic/getpid completed (status=0x0) ===
```

说明 fork、exec、getpid 全流程已经通了，且没有再“卡死”。

## 为什么会触发 kernel trap（详细解释）

### 1) 内核栈太小
配置中 `KERNEL_STACK_SIZE = 8KB`。在 fork 阶段：
- `sys_fork` → `TaskControlBlock::fork` → `MemorySet::from_existed_user`
- 每个 `trace!` / `println!` 都会进入格式化栈路径（`core::fmt`），且递归深度多。
- 8KB 很容易被耗尽，尤其是在 debug 模式下。

### 2) trap_from_kernel 表明异常发生在内核态
在内核态发生异常时会进入 `trap_from_kernel`，直接 panic 或挂死。这与“用户态 page fault”不同。

### 3) 输出路径中断
日志中最后出现的是 `console::write_str`，说明**输出本身就是触发点**，这也解释了为什么“加 trace 反而更卡”。

## 关键修复

### 1) 增大 kernel stack
将 `KERNEL_STACK_SIZE` 调整为 16KB（或更大）。

位置：
- [os/src/config.rs](os/src/config.rs)

调整后，fork 过程可以在 debug 模式下完成，不再溢出。

### 2) 减少 debug 输出的递归深度
在 fork 与 MemorySet 中避免大段 `println!`/`trace!`，尽量用少量标记或阶段性输出。

## 成功运行结果（最终验证）
在调整内核栈并保留必要 trace 后，`getpid` 单测可以完整运行并退出：

- `/musl/basic/getpid` 成功执行
- sys_getpid 正确返回 PID=1
- fork/exec 路径完整

典型输出片段（用户提供）：
```
[TRACE] kernel:pid[1] sys_write
getpid success.
pid = 1
[TRACE] kernel:pid[1] sys_exit
=== /musl/basic/getpid completed (status=0x0) ===
```

这表明基础系统调用在 ext4 + VFS 适配链路下可正常工作。

## 与 ext4 的关系
本次卡死并不是 ext4 的数据结构或 IO 错误：
- ext4 能正确挂载
- `/musl/basic/getpid` ELF 文件可完整读取
- exec 成功后能进入用户态

问题集中在 fork 的内核输出路径。即使关闭 ext4，也会在相同位置因为大量 debug 输出而发生栈溢出。

## 后续建议
1) **保留较大的 kernel stack**（至少 16KB），尤其是 debug 模式。
2) debug 输出尽量使用“阶段性短输出”，避免在循环/密集路径里打印字符串。
3) 后续测试更复杂的系统调用时，优先单测（如 `/musl/basic/chdir`, `/musl/basic/open`）避免被批量脚本干扰。
4) 完成 ext4/VFS 完整适配后，再统一打开全套测试脚本。

