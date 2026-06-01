# Syscall hot path optimizations (clock_gettime/read)

## 背景
这次优化聚焦于 `clock_gettime` 和 `read` 两条 LTP 高频路径，目标是降低每次系统调用的固定开销（分配、拷贝、日志判断），并在不改变语义的前提下走更短的内核路径。

## 具体优化点与收益

### 1) 用户态写回：无 Vec 分配的 slice 遍历
- 新增 `for_each_user_write_slice` / `copy_to_user_inline` / `write_value_to_user`，直接遍历用户页切片，把小结构体写回用户态，避免构建 `Vec<&mut [u8]>` 的分配和中间拷贝。[os/src/syscall/user_mem.rs](os/src/syscall/user_mem.rs#L109-L191)
- 写回策略继续保留：
  - COW + fork fallback (`DemandCowWithForkFallback`) 路径不变；
  - `RelaxedReadableMapping` 允许“只读映射 + 写回”场景的快速写入。[os/src/syscall/user_mem.rs](os/src/syscall/user_mem.rs#L109-L191)

**为什么更快：**
- 对于 `TimeSpec` 等小结构体，直接遍历切片写回能减少内存分配和 `Vec` 管理成本；
- 省掉一次“先收集页切片再逐片拷贝”的间接层。

### 2) `clock_gettime` / `clock_getres` 走小结构体快路径
- 直接在栈上构造 `TimeSpec`，用 `write_value_to_user` 写回；避免临时 `Vec` 和大范围页收集。[os/src/syscall/process.rs](os/src/syscall/process.rs#L2043-L2098)
- 使用 `RelaxedReadableMapping`，减少“必须可写映射”的严格要求，从而降低失败重试/额外检查成本。[os/src/syscall/process.rs](os/src/syscall/process.rs#L2043-L2098)

**为什么更快：**
- `clock_gettime` 是 LTP 的高频调用，单次优化收益可被放大；
- 小结构体“直写”避免了内核侧的额外分配与拷贝。

### 3) `sys_read` 优先走文件对象快路径
- `sys_read` 先尝试 `file.read_user_buffer(...)`；只有当文件没有快路径实现时才回退到 `translated_user_write_buffer + file.read(UserBuffer)` 的通用路径。[os/src/syscall/fs.rs](os/src/syscall/fs.rs#L740-L776)
- 仍保留原有语义：
  - `O_DIRECT` 对齐检查；
  - `timerfd` 的 `EAGAIN` sentinel；
  - socket 可读等待与信号中断；
  - 目录 `EISDIR` 处理。[os/src/syscall/fs.rs](os/src/syscall/fs.rs#L740-L776)

**为什么更快：**
- 对于能直接生成/读取数据的设备或文件，实现可以绕开 `Vec` 切片收集，减少内存管理与拷贝。

### 4) `File` trait 新增可选读快路径
- `File` 新增 `read_user_buffer` 默认实现，允许文件类型选择性覆盖。[os/src/fs/mod.rs](os/src/fs/mod.rs#L24-L37)
- 具体覆盖：
  - VFS regular file：直接按页切片读 inode，避免中间 `UserBuffer` 组装。[os/src/fs/vfs/file.rs](os/src/fs/vfs/file.rs#L117-L141)
  - `/dev/zero`：直接填充用户切片为 0；
  - `/dev/urandom`：直接填充用户切片为随机字节。[os/src/fs/stdio.rs](os/src/fs/stdio.rs#L193-L260)

**为什么更快：**
- `read` 热点多为“简单数据源”，直接写用户内存可以省去中间 `Vec` 与 copy；
- 设备类文件不需要走 inode 缓存/页收集逻辑。

### 5) syscall trace 判断更轻量
- `should_trace_syscall()` 在未启用 syscall 日志时直接返回，避免多余的 PID/名称判断。[os/src/syscall/mod.rs](os/src/syscall/mod.rs#L672-L693)
- `syscall()` 只在真的需要 trace 时才克隆进程名；否则不做字符串分配。[os/src/syscall/mod.rs](os/src/syscall/mod.rs#L701-L723)

**为什么更快：**
- 日志未启用时，系统调用不再支付额外的字符串克隆与判断开销；
- 这一优化会遍历所有 syscall 路径，收益广泛。

## 效果预期
- `clock_gettime`/`clock_getres`：单次调用减少分配和多次切片遍历，适合高频场景。
- `read`：设备文件和常规文件在热路径上减少 `Vec`/`UserBuffer` 构建与拷贝。
- 全局 syscall：当日志未启用时减少 tracing 相关的固定开销。

## 语义保持说明
- 仍保留 COW / fork fallback / relaxed mapping 的用户内存策略。[os/src/syscall/user_mem.rs](os/src/syscall/user_mem.rs#L109-L191)
- `sys_read` 仍保持对 `O_DIRECT`、socket 可读等待、`timerfd` sentinel 和 `EISDIR` 的原语义处理。[os/src/syscall/fs.rs](os/src/syscall/fs.rs#L740-L776)
