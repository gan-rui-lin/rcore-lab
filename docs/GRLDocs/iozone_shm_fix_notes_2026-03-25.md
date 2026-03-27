# iozone SHM 修复记录（2026-03-25）

## 1. 最终结论

`iozone-test` 分支上的 System V SHM 已从占位实现改为真实映射实现，`glibc-iozone` 已恢复为满项通过：

- `judge_iozone-glibc.py`：`20/20`（`sum_score=20.0`）
- 无 `Unable to get shared memory segment` / `Error 22`

## 2. 本次实际修复内容

### 2.1 SHM 后端改为真实物理页帧

文件：`os/src/syscall/ipc.rs`

- `ShmSegment` 从 `Vec<u8>` 改为 `Vec<Arc<FrameTracker>>`
- `shmget` 按页分配 frame（页对齐）

### 2.2 `shmat/shmdt` 接入真实地址空间映射

文件：`os/src/syscall/ipc.rs` + `os/src/mm/memory_set.rs`

- `shmat` 不再返回伪地址，改为把共享页映射进当前进程 `MemorySet`
- `shmdt` 根据 `(pid, addr)` 精确 detach，并从页表解除映射
- `MemorySet` 新增 `insert_shared_framed_area`
- `MapArea` 新增共享页记录（`shared_ppns`）

### 2.3 `IPC_RMID` 改为延迟删除语义

文件：`os/src/syscall/ipc.rs`

- `IPC_RMID` 只做 `marked_for_delete`
- `attach_count == 0` 才真正回收段

### 2.4 fork 语义修正：共享区不深拷贝

文件：`os/src/mm/memory_set.rs`

- `MemorySet::from_existed_user` 对共享区保持映射同一组物理页
- 避免把 SHM 误克隆成私有内存

### 2.5 兼容性降噪

文件：`os/src/syscall/mod.rs`

- `sync(81)` 返回 `0`，避免 iozone 里无意义 `ENOSYS` 噪音

## 3. 二次故障与补丁（关键）

在第一轮修复后，出现过“前几组通过、后续 `shmat` 反复 `-EINVAL`”的问题。

根因：
- SHM attachment 表以 `(pid, addr)` 为 key；
- 进程退出时未清理该 pid 的 attach 记录；
- pid 复用后，新进程 `shmat` 命中旧记录，误判“地址冲突”，报 `Error 22`。

修复：
- 新增退出清理路径：进程结束时清理该 pid 所有关联 SHM attachment；
- 同步下调对应段 `attach_count`，若已 `IPC_RMID` 且引用归零则立即回收。

涉及文件：
- `os/src/syscall/ipc.rs`（`cleanup_shm_attachments_for_pid`）
- `os/src/syscall/mod.rs`（导出 `cleanup_shm_for_process_exit`）
- `os/src/task/mod.rs`（进程退出路径调用清理）

## 4. 验证记录

### 4.1 失败样例（修复前）

- 日志：`/tmp/la-iozone-log-off-after-reapply.log`
- 现象：第 1 组通过，后续出现 `Unable to get shared memory segment ..Error 22`
- 分数：`4/20`

### 4.2 成功样例（修复后）

- 日志：`/tmp/la-iozone-log-off-after-exit-cleanup.log`
- 结果：
  - `iozone test complete` 全部子组完成
  - `#### OS COMP TEST GROUP END iozone-glibc ####`
  - `QEMU exited`
- 判分：`20/20`

## 5. 复验命令

```bash
LOG=OFF SINGLE_TEST=glibc-iozone bash run-la.sh > /tmp/la-iozone.log 2>&1
python3 /home/grl/codeRepo/autotest-for-oskernel/kernel/judge/judge_iozone-glibc.py < /tmp/la-iozone.log
rg -n "Error 22|Unable to get shared memory segment|OS COMP TEST GROUP END|QEMU exited" /tmp/la-iozone.log
```
