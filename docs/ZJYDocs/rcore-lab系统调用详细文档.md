# rCore-lab 操作系统系统调用详细文档

## 项目概述

**项目路径**: `/Users/mac/Desktop/project/rcore-lab/os`
**架构**: RISC-V 64位
**系统调用总数**: 55个（已实现）
**兼容性**: 部分Linux ABI兼容

rCore-lab 是一个教学操作系统，基于Rust语言开发，实现了完整的进程管理、文件系统、内存管理和同步机制。

---

## 系统调用架构

### 调用流程

```
用户程序 → ecall指令
    ↓
陷阱处理 (crate::trap::trap_handler)
    ↓
系统调用分发 (syscall()) [syscall/mod.rs]
    ↓
具体实现函数 (process.rs/fs.rs/thread.rs/sync.rs)
    ↓
返回值通过a0寄存器返回用户程序
```

### 参数传递约定

- **系统调用号**: 通过 `syscall_id` 参数传递（从a7寄存器获取）
- **参数1-6**: `args[0]`-`args[5]`（从a0-a5寄存器获取）
- **返回值**: `isize` 类型，通过a0寄存器返回
- **错误码**: 负数表示错误（Linux errno约定）

---

## 一、进程管理系统调用

### 1. exit - 进程退出

**系统调用名**: exit
**系统调用函数**: `sys_exit`
**系统调用号**: 93 (SYSCALL_EXIT)
**入参**:
- exit_code (i32): 退出状态码

**功能**: 终止当前进程的执行。释放进程资源，将退出状态保存给父进程，唤醒等待的父进程。
**返回值**: 不返回（进程已终止）

**实现位置**: `os/src/syscall/process.rs:sys_exit()`

**实现细节**:
```rust
pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit (exit_code={})",
        current_process().pid.0, exit_code);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}
```

---

### 2. exit_group - 进程组退出 ✨新增

**系统调用名**: exit_group
**系统调用函数**: `sys_exit_group`
**系统调用号**: 94 (SYSCALL_EXIT_GROUP)
**入参**:
- exit_code (i32): 退出状态码

**功能**: 终止进程组中的所有线程。在rCore-lab中，行为与exit相同，因为整个进程都会被终止。
**返回值**: 不返回（进程已终止）

**实现位置**: `os/src/syscall/process.rs:sys_exit_group()`

**实现细节**:
```rust
pub fn sys_exit_group(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit_group (exit_code={})",
        current_process().pid.0, exit_code);
    sys_exit(exit_code)
}
```

---

### 3. fork - 创建子进程

**系统调用名**: fork
**系统调用函数**: `sys_fork`
**系统调用号**: 220 (SYSCALL_FORK)
**入参**: 无

**功能**: 创建当前进程的副本。子进程获得父进程的地址空间、打开的文件描述符等的完整拷贝。
**返回值**:
- 父进程: 子进程的PID
- 子进程: 0
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_fork()`

**实现细节**:
- 调用 `current_task().unwrap().fork()`
- 子进程的trap context中a0寄存器设为0
- 父进程返回子进程PID

---

### 4. exec - 执行新程序

**系统调用名**: execve
**系统调用函数**: `sys_exec`
**系统调用号**: 221 (SYSCALL_EXEC)
**入参**:
- path (const char*): 可执行文件路径
- argv (const usize*): 参数数组指针
- envp (const usize*): 环境变量数组指针

**功能**: 用新程序替换当前进程的地址空间。成功则不返回，失败返回-1。
**返回值**:
- 成功: 不返回
- 失败: -1 (返回错误码如-ENOENT)

**实现位置**: `os/src/syscall/process.rs:sys_exec()`

**实现细节**:
- 解析ELF文件
- 解析参数和环境变量
- 替换进程地址空间
- 设置新的入口点和栈指针

---

### 5. waitpid - 等待子进程

**系统调用名**: waitpid
**系统调用函数**: `sys_waitpid`
**系统调用号**: 260 (SYSCALL_WAITPID)
**入参**:
- pid (isize):
  - -1: 等待任意子进程
  - >0: 等待指定PID的子进程
- wstatus (int*): 存储子进程退出状态的指针

**功能**: 等待子进程状态改变（退出），并获取其退出状态。
**返回值**:
- 成功: 子进程的PID
- 无子进程: -ECHILD
- 参数错误: -EINVAL

**实现位置**: `os/src/syscall/process.rs:sys_waitpid()`

**实现细节**:
- 支持等待特定PID或任意子进程
- 如果子进程未退出，将当前进程阻塞
- 子进程退出后，回收资源并返回PID

---

### 6. getpid - 获取进程ID

**系统调用名**: getpid
**系统调用函数**: `sys_getpid`
**系统调用号**: 172 (SYSCALL_GETPID)
**入参**: 无

**功能**: 返回当前进程的进程ID。
**返回值**: 当前进程的PID

**实现位置**: `os/src/syscall/process.rs:sys_getpid()`

**实现细节**:
```rust
pub fn sys_getpid() -> isize {
    current_process().pid.0 as isize
}
```

---

### 7. getppid - 获取父进程ID

**系统调用名**: getppid
**系统调用函数**: `sys_getppid`
**系统调用号**: 173 (SYSCALL_GETPPID)
**入参**: 无

**功能**: 返回当前进程的父进程ID。
**返回值**: 父进程的PID

**实现位置**: `os/src/syscall/process.rs:sys_getppid()`

**实现细节**:
```rust
pub fn sys_getppid() -> isize {
    let process = current_process();
    let inner = process.inner_exclusive_access();
    match inner.parent {
        Some(ref parent) => parent.upgrade().unwrap().pid.0 as isize,
        None => 0,
    }
}
```

---

### 8. getuid - 获取用户ID ✨新增

**系统调用名**: getuid
**系统调用函数**: `sys_getuid`
**系统调用号**: 174 (SYSCALL_GETUID)
**入参**: 无

**功能**: 返回当前进程的用户ID。rCore-lab是单用户系统，始终返回0。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_getuid()`

**实现细节**:
```rust
pub fn sys_getuid() -> isize {
    trace!("kernel:pid[{}] sys_getuid", current_process().pid.0);
    0  // Single-user system
}
```

---

### 9. geteuid - 获取有效用户ID ✨新增

**系统调用名**: geteuid
**系统调用函数**: `sys_geteuid`
**系统调用号**: 175 (SYSCALL_GETEUID)
**入参**: 无

**功能**: 返回当前进程的有效用户ID。rCore-lab是单用户系统，始终返回0。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_geteuid()`

**实现细节**:
```rust
pub fn sys_geteuid() -> isize {
    trace!("kernel:pid[{}] sys_geteuid", current_process().pid.0);
    0  // Single-user system
}
```

---

### 10. getgid - 获取组ID ✨新增

**系统调用名**: getgid
**系统调用函数**: `sys_getgid`
**系统调用号**: 176 (SYSCALL_GETGID)
**入参**: 无

**功能**: 返回当前进程的组ID。rCore-lab是单用户系统，始终返回0。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_getgid()`

**实现细节**:
```rust
pub fn sys_getgid() -> isize {
    trace!("kernel:pid[{}] sys_getgid", current_process().pid.0);
    0  // Single-user system
}
```

---

### 11. getegid - 获取有效组ID ✨新增

**系统调用名**: getegid
**系统调用函数**: `sys_getegid`
**系统调用号**: 177 (SYSCALL_GETEGID)
**入参**: 无

**功能**: 返回当前进程的有效组ID。rCore-lab是单用户系统，始终返回0。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_getegid()`

**实现细节**:
```rust
pub fn sys_getegid() -> isize {
    trace!("kernel:pid[{}] sys_getegid", current_process().pid.0);
    0  // Single-user system
}
```

---

### 12. yield - 主动让出CPU

**系统调用名**: sched_yield
**系统调用函数**: `sys_yield`
**系统调用号**: 124 (SYSCALL_YIELD)
**入参**: 无

**功能**: 主动让出CPU时间片，允许调度器调度其他任务。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_yield()`

**实现细节**:
```rust
pub fn sys_yield() -> isize {
    suspend_current_and_run_next();
    0
}
```

---

### 13. spawn - 产生新进程（xv6兼容）

**系统调用名**: spawn
**系统调用函数**: `sys_spawn`
**系统调用号**: 400 (SYSCALL_SPAWN)
**入参**:
- path (const char*): 可执行文件路径

**功能**: 创建新进程并执行指定程序。相当于fork+exec的组合。
**返回值**:
- 成功: 子进程的PID
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_spawn()`

**实现细节**:
- 读取并解析ELF文件
- 创建新进程
- 设置入口点
- 返回子进程PID

---

### 14. set_priority - 设置进程优先级

**系统调用名**: set_priority
**系统调用函数**: `sys_set_priority`
**系统调用号**: 140 (SYSCALL_SET_PRIORITY)
**入参**:
- priority (isize): 优先级值（2或更高）

**功能**: 设置当前进程的调度优先级。
**返回值**:
- 成功: priority值
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_set_priority()`

---

## 二、线程管理系统调用

### 15. thread_create - 创建线程

**系统调用名**: thread_create
**系统调用函数**: `sys_thread_create`
**系统调用号**: 460 (SYSCALL_THREAD_CREATE)
**入参**:
- entry (usize): 线程入口函数地址
- arg (usize): 线程参数

**功能**: 在当前进程中创建新线程。
**返回值**:
- 成功: 线程ID (TID)
- 失败: -1

**实现位置**: `os/src/syscall/thread.rs:sys_thread_create()`

**实现细节**:
- 在当前进程的tasks向量中分配新线程
- 设置线程入口点和参数
- 添加到调度队列

---

### 16. gettid - 获取线程ID

**系统调用名**: gettid
**系统调用函数**: `sys_gettid`
**系统调用号**: 178 (SYSCALL_GETTID)
**入参**: 无

**功能**: 返回当前线程的线程ID。
**返回值**: 当前线程的TID

**实现位置**: `os/src/syscall/thread.rs:sys_gettid()`

**实现细节**:
```rust
pub fn sys_gettid() -> isize {
    current_task().unwrap().inner_exclusive_access()
        .res.as_ref().unwrap().tid as isize
}
```

---

### 17. set_tid_address - 设置线程ID地址

**系统调用名**: set_tid_address
**系统调用函数**: `sys_set_tid_address`
**系统调用号**: 96 (SYSCALL_SET_TID_ADDRESS)
**入参**:
- tidptr (int*): 线程ID存储地址

**功能**: 设置清除子TID的地址，用于支持CLONE_CHILD_CLEARTID。当线程退出时，内核会在此地址写入0并唤醒futex等待者。
**返回值**: 当前线程的TID

**实现位置**: `os/src/syscall/thread.rs:sys_set_tid_address()`

**实现细节**:
```rust
pub fn sys_set_tid_address(tidptr: *mut i32) -> isize {
    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.clear_child_tid = tidptr as usize;
    let tid = inner.res.as_ref().unwrap().tid;

    // 将当前TID写入用户空间地址
    if !tidptr.is_null() {
        let token = current_user_token();
        *translated_refmut(token, tidptr) = tid as i32;
    }
    tid as isize
}
```

---

### 18. waittid - 等待线程退出

**系统调用名**: waittid
**系统调用函数**: `sys_waittid`
**系统调用号**: 462 (SYSCALL_WAITTID)
**入参**:
- tid (usize): 要等待的线程ID

**功能**: 等待指定线程退出。
**返回值**:
- 成功: 线程的退出状态
- 失败: -1

**实现位置**: `os/src/syscall/thread.rs:sys_waittid()`

**实现细节**:
- 检查TID是否有效
- 如果线程未退出，阻塞当前线程
- 线程退出后返回其退出码

---

## 三、文件I/O系统调用

### 19. read - 读取文件

**系统调用名**: read
**系统调用函数**: `sys_read`
**系统调用号**: 63 (SYSCALL_READ)
**入参**:
- fd (int): 文件描述符
- buf (void*): 读取缓冲区
- count (size_t): 要读取的字节数

**功能**: 从文件描述符读取数据到缓冲区。
**返回值**:
- 成功: 实际读取的字节数
- EOF: 0
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_read()`

**实现细节**:
- 通过translated_byte_buffer获取用户缓冲区
- 调用文件对象的read方法
- 支持普通文件、设备文件、管道等

---

### 20. write - 写入文件

**系统调用名**: write
**系统调用函数**: `sys_write`
**系统调用号**: 64 (SYSCALL_WRITE)
**入参**:
- fd (int): 文件描述符
- buf (const void*): 写入数据缓冲区
- count (size_t): 要写入的字节数

**功能**: 将缓冲区数据写入文件描述符。
**返回值**:
- 成功: 实际写入的字节数
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_write()`

**实现细节**:
- 通过translated_byte_buffer获取用户缓冲区
- 调用文件对象的write方法
- 支持stdout、普通文件、管道等

---

### 21. openat - 打开文件

**系统调用名**: openat
**系统调用函数**: `sys_openat`
**系统调用号**: 56 (SYSCALL_OPENAT)
**入参**:
- dirfd (int): 目录文件描述符（AT_FDCWD=-100表示当前目录）
- path (const char*): 文件路径
- flags (u32): 打开标志
  - O_RDONLY (0): 只读
  - O_WRONLY (1): 只写
  - O_RDWR (2): 读写
  - O_CREATE (0x200): 不存在则创建
  - O_TRUNC (0x400): 截断文件
- mode (u32): 创建模式（当前未使用）

**功能**: 相对于目录文件描述符打开文件。
**返回值**:
- 成功: 文件描述符
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_openat()`

**实现细节**:
- 支持绝对路径和相对路径
- 支持目录文件描述符
- 处理各种打开标志

---

### 22. close - 关闭文件

**系统调用名**: close
**系统调用函数**: `sys_close`
**系统调用号**: 57 (SYSCALL_CLOSE)
**入参**:
- fd (int): 文件描述符

**功能**: 关闭文件描述符，释放相关资源。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_close()`

---

### 23. fstat - 获取文件状态

**系统调用名**: fstat
**系统调用函数**: `sys_fstat`
**系统调用号**: 80 (SYSCALL_FSTAT)
**入参**:
- fd (int): 文件描述符
- st (struct Stat*): stat结构体指针

**功能**: 获取文件的元数据（大小、类型、inode等）。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_fstat()`

**Stat结构**:
```rust
pub struct Stat {
    pub dev: u64,       // 设备ID
    pub ino: u64,       // inode号
    pub mode: u32,      // 文件类型和权限
    pub nlink: u32,     // 硬链接数
    pub pad: [u64; 7],  // 填充
}
```

---

### 24. dup - 复制文件描述符

**系统调用名**: dup
**系统调用函数**: `sys_dup`
**系统调用号**: 23 (SYSCALL_DUP)
**入参**:
- oldfd (int): 要复制的文件描述符

**功能**: 复制文件描述符，新旧描述符指向同一文件。
**返回值**:
- 成功: 新文件描述符
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_dup()`

---

### 25. dup3 - 复制文件描述符到指定位置

**系统调用名**: dup3
**系统调用函数**: `sys_dup3`
**系统调用号**: 24 (SYSCALL_DUP3)
**入参**:
- oldfd (int): 要复制的文件描述符
- newfd (int): 目标文件描述符

**功能**: 复制文件描述符到指定位置。
**返回值**:
- 成功: 新文件描述符
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_dup3()`

---

### 26. pipe2 - 创建管道

**系统调用名**: pipe2
**系统调用函数**: `sys_pipe2`
**系统调用号**: 59 (SYSCALL_PIPE2)
**入参**:
- fds (int[2]): 存储管道文件描述符的数组
- flags (u32): 标志（当前未使用）

**功能**: 创建单向数据通道，fds[0]为读端，fds[1]为写端。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/fs.rs:sys_pipe2()`

---

### 27-31. 目录和路径操作

**mkdirat** - 创建目录 (34)
**unlinkat** - 删除文件 (35)
**linkat** - 创建硬链接 (37)
**chdir** - 改变当前目录 (49)
**getcwd** - 获取当前目录 (17)
**getdents64** - 读取目录项 (61)

**实现位置**: `os/src/syscall/fs.rs`

---

### 32-33. 文件系统挂载

**mount** - 挂载文件系统 (40)
**umount2** - 卸载文件系统 (39)

**实现位置**: `os/src/syscall/fs.rs`

**支持的文件系统**:
- FAT32 - fat
- EXT4 - ext4
- EasyFS - easy-fs

---

## 四、内存管理系统调用

### 34. mmap - 内存映射

**系统调用名**: mmap
**系统调用函数**: `sys_mmap`
**系统调用号**: 222 (SYSCALL_MMAP)
**入参**:
- start (usize): 映射起始地址（建议）
- len (usize): 映射长度
- prot (usize): 保护标志（PROT_READ, PROT_WRITE, PROT_EXEC）
- flags (usize): 映射标志（MAP_SHARED, MAP_PRIVATE）
- fd (usize): 文件描述符（文件映射，当前未实现）
- offset (usize): 文件偏移（当前未实现）

**功能**: 在进程地址空间创建内存映射。
**返回值**:
- 成功: 映射起始地址
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_mmap()`

**实现细节**:
- 将虚拟地址映射到物理页
- 设置页面权限（可读、可写、可执行）
- 支持匿名映射

---

### 35. munmap - 解除内存映射

**系统调用名**: munmap
**系统调用函数**: `sys_munmap`
**系统调用号**: 215 (SYSCALL_MUNMAP)
**入参**:
- start (usize): 映射起始地址
- len (usize): 映射长度

**功能**: 解除进程地址空间的内存映射。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_munmap()`

**实现细节**:
- 解除虚拟地址到物理页的映射
- 释放相关页表项
- 清空TLB

---

### 36. sbrk - 改变堆空间

**系统调用名**: sbrk
**系统调用函数**: `sys_sbrk`
**系统调用号**: 214 (SYSCALL_SBRK)
**入参**:
- size (isize): 增长的字节数（可为负）

**功能**: 扩展或收缩进程的堆内存。
**返回值**:
- 成功: 旧堆顶地址
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_sbrk()`

**实现细节**:
- 修改program_brk值
- 如果扩展，分配新页面
- 如果收缩，释放多余页面

---

## 五、同步原语系统调用

### 37-39. 互斥量操作

**mutex_create** - 创建互斥量 (463)
**mutex_lock** - 互斥量上锁 (464)
**mutex_unlock** - 互斥量解锁 (466)

**实现位置**: `os/src/syscall/sync.rs`

**功能**: 提供进程内线程间的互斥同步机制。

---

### 40-42. 信号量操作

**semaphore_create** - 创建信号量 (467)
**semaphore_up** - 信号量+1 (468)
**semaphore_down** - 信号量-1 (470)

**实现位置**: `os/src/syscall/sync.rs`

**功能**: 提供进程内线程间的信号量同步机制。

---

### 43-45. 条件变量操作

**condvar_create** - 创建条件变量 (471)
**condvar_signal** - 条件变量信号 (472)
**condvar_wait** - 条件变量等待 (473)

**实现位置**: `os/src/syscall/sync.rs`

**功能**: 提供进程内线程间的条件变量同步机制。

---

## 六、信号处理系统调用

### 46. kill - 发送信号

**系统调用名**: kill
**系统调用函数**: `sys_kill`
**系统调用号**: 129 (SYSCALL_KILL)
**入参**:
- pid (usize): 目标进程ID
- signum (i32): 信号编号

**功能**: 向指定进程发送信号。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_kill()`

**支持的信号**:
- SIGKILL (9): 强制终止
- SIGSTOP (19): 停止进程
- SIGCONT (18): 继续进程
- 等等（MAX_SIG = 31）

---

### 47. sigaction - 设置信号处理器

**系统调用名**: rt_sigaction
**系统调用函数**: `sys_sigaction`
**系统调用号**: 134 (SYSCALL_SIGACTION)
**入参**:
- signum (i32): 信号编号
- act (const SignalAction*): 新的信号处理动作
- oldact (SignalAction*): 旧的信号处理动作（输出）

**功能**: 设置信号处理函数。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `os/src/syscall/process.rs:sys_sigaction()`

**SignalAction结构**:
```rust
pub struct SignalAction {
    pub handler: usize,  // 处理函数地址
    pub mask: SignalFlags, // 信号掩码
}
```

---

### 48. sigprocmask - 设置信号掩码

**系统调用名**: rt_sigprocmask
**系统调用函数**: `sys_sigprocmask`
**系统调用号**: 135 (SYSCALL_SIGPROCMASK)
**入参**:
- mask (u32): 新信号掩码

**功能**: 修改进程的信号屏蔽字。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_sigprocmask()`

---

### 49. sigreturn - 从信号处理器返回

**系统调用名**: rt_sigreturn
**系统调用函数**: `sys_sigreturn`
**系统调用号**: 139 (SYSCALL_SIGRETURN)
**入参**: 无（从栈帧恢复上下文）

**功能**: 从信号处理器返回，恢复被中断的执行上下文。
**返回值**: 恢复的a0寄存器值

**实现位置**: `os/src/syscall/process.rs:sys_sigreturn()`

---

## 七、时间相关系统调用

### 50. get_time - 获取当前时间

**系统调用名**: gettimeofday
**系统调用函数**: `sys_get_time`
**系统调用号**: 169 (SYSCALL_GET_TIME)
**入参**:
- tv (TimeVal*): 时间结构指针
- tz (usize): 时区（未使用）

**功能**: 获取当前系统时间（秒和微秒）。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_get_time()`

**TimeVal结构**:
```rust
pub struct TimeVal {
    pub sec: usize,   // 秒
    pub usec: usize,  // 微秒
}
```

---

### 51. nanosleep - 纳秒级睡眠

**系统调用名**: nanosleep
**系统调用函数**: `sys_nanosleep`
**系统调用号**: 101 (SYSCALL_NANOSLEEP)
**入参**:
- req (const TimeSpec*): 请求睡眠时间
- rem (TimeSpec*): 剩余时间（如被信号中断）

**功能**: 使进程睡眠指定的纳秒数。
**返回值**:
- 成功: 0
- 被中断: -EINTR

**实现位置**: `os/src/syscall/process.rs:sys_nanosleep()`

**TimeSpec结构**:
```rust
pub struct TimeSpec {
    pub tv_sec: usize,  // 秒
    pub tv_nsec: usize, // 纳秒
}
```

---

### 52. times - 获取进程时间

**系统调用名**: times
**系统调用函数**: `sys_times`
**系统调用号**: 153 (SYSCALL_TIMES)
**入参**:
- tms (Tms*): 时间结构指针

**功能**: 获取进程和子进程的CPU时间。
**返回值**: 系统启动以来的时钟滴答数

**实现位置**: `os/src/syscall/process.rs:sys_times()`

**Tms结构**:
```rust
pub struct Tms {
    pub tms_utime: i64,  // 用户态CPU时间
    pub tms_stime: i64,  // 内核态CPU时间
    pub tms_cutime: i64, // 子进程用户态时间
    pub tms_cstime: i64, // 子进程内核态时间
}
```

---

## 八、其他系统调用

### 53. uname - 获取系统信息

**系统调用名**: uname
**系统调用函数**: `sys_uname`
**系统调用号**: 160 (SYSCALL_UNAME)
**入参**:
- uts (UtsName*): 系统信息结构指针

**功能**: 获取系统名称和版本信息。
**返回值**: 0

**实现位置**: `os/src/syscall/process.rs:sys_uname()`

**UtsName结构**:
```rust
pub struct UtsName {
    pub sysname: [u8; 65],    // "rCore-lab"
    pub nodename: [u8; 65],   // 主机名
    pub release: [u8; 65],    // 发布版本
    pub version: [u8; 65],    // 版本信息
    pub machine: [u8; 65],    // "riscv64"
    pub domainname: [u8; 65], // 域名
}
```

---

### 54. shutdown - 关闭系统

**系统调用名**: shutdown
**系统调用函数**: `sys_shutdown`
**系统调用号**: 1001 (SYSCALL_SHUTDOWN)
**入参**: 无

**功能**: 关闭系统（调用SBI shutdown）。
**返回值**: 不返回

**实现位置**: `os/src/syscall/process.rs:sys_shutdown()`

---

## 系统调用统计

### 按类别统计

| 类别 | 数量 | 主要文件 |
|------|------|---------|
| 进程管理 | 14 | process.rs |
| 线程管理 | 4 | thread.rs |
| 文件I/O | 16 | fs.rs |
| 内存管理 | 3 | process.rs |
| 同步原语 | 9 | sync.rs |
| 信号处理 | 4 | process.rs |
| 时间管理 | 3 | process.rs |
| 其他 | 2 | process.rs |
| **总计** | **55** | |

### 新增系统调用 (2026-02-14)

| 系统调用 | 调用号 | 功能 | 状态 |
|---------|--------|------|------|
| getuid | 174 | 获取用户ID | ✅ 已实现 |
| geteuid | 175 | 获取有效用户ID | ✅ 已实现 |
| getgid | 176 | 获取组ID | ✅ 已实现 |
| getegid | 177 | 获取有效组ID | ✅ 已实现 |
| exit_group | 94 | 进程组退出 | ✅ 已实现 |

---

## 核心文件清单

| 文件 | 行数 | 描述 |
|------|------|------|
| `src/syscall/mod.rs` | 524 | 系统调用分发和跟踪 |
| `src/syscall/process.rs` | 668 | 进程管理、信号、内存、时间 |
| `src/syscall/fs.rs` | ~800 | 文件系统操作 |
| `src/syscall/thread.rs` | ~150 | 线程管理 |
| `src/syscall/sync.rs` | ~200 | 同步原语 |
| `src/syscall/errno.rs` | ~100 | 错误码定义 |
| `src/task/process.rs` | 384 | 进程控制块(PCB) |
| `src/task/task.rs` | ~300 | 任务控制块(TCB) |

---

## 关键设计特点

### 1. 模块化架构

不同类别的系统调用实现在独立的源文件中：
- `process.rs` - 进程管理和信号
- `thread.rs` - 线程管理
- `fs.rs` - 文件系统
- `sync.rs` - 同步原语

### 2. 错误处理

使用Linux errno约定：
- 返回负数表示错误（-ENOSYS, -EINVAL等）
- errno定义在`errno.rs`中
- 提供`errno()`辅助函数

### 3. 日志和调试

完整的系统调用跟踪机制：
- 环境变量TRACE_PID跟踪特定进程
- 环境变量TRACE_NAME跟踪特定程序
- SYSCALL_TRACE_ALL全局开关
- trace!宏记录系统调用

### 4. 多线程支持

每个进程包含多个线程：
- 线程共享进程地址空间
- 独立的线程栈和上下文
- 支持线程本地存储(TLS)

### 5. 同步机制

提供完整的同步原语：
- 互斥量(Mutex)
- 信号量(Semaphore)
- 条件变量(Condvar)

### 6. 文件系统支持

支持多种文件系统：
- FAT32 - 兼容性好
- EXT4 - Linux标准
- EasyFS - 教学文件系统
- 最近新增块缓存层（64块LRU）

### 7. 内存管理

灵活的内存管理：
- 页表管理
- mmap/munmap支持
- TLS区域支持
- 堆内存动态增长(sbrk)

### 8. 信号处理

完整的信号机制：
- 31个标准信号
- 自定义信号处理器
- 信号掩码和嵌套处理

### 9. 单用户模型

简化的权限系统：
- 所有用户/组ID返回0
- 没有权限检查
- 简化教学和调试

### 10. 类型安全

利用Rust类型系统：
- 强类型参数
- 所有权和生命周期
- 无数据竞争

---

## 与xv6-lab对比

| 特性 | xv6-lab | rCore-lab |
|-----|---------|----------|
| 系统调用数量 | 100+ | 55 |
| 进程管理 | ✓ fork/exec/wait | ✓ fork/exec/waitpid |
| 线程管理 | ✗ | ✓ thread_create/gettid |
| 用户ID | ✓ getuid/geteuid | ✓ 新增支持 |
| 组ID | ✓ getgid/getegid | ✓ 新增支持 |
| 进程组退出 | ✓ exit_group | ✓ 新增支持 |
| clone | ✓ 完整实现 | ✗ 使用thread_create |
| 网络 | ✓ socket/bind/connect | ✗ 未实现 |
| IPC | ✓ System V | ✗ 未实现 |
| 同步原语 | ✗ | ✓ mutex/semaphore/condvar |
| 文件系统 | 自定义 | FAT32/EXT4/EasyFS |
| 块缓存 | 内置 | 最近新增 |
| 架构 | C语言 | Rust语言 |

---

## 使用示例

### 进程创建和等待

```c
// 用户程序示例
int pid = fork();
if (pid == 0) {
    // 子进程
    exec("/bin/sh", argv, envp);
} else {
    // 父进程
    int status;
    waitpid(pid, &status);
}
```

### 线程创建

```c
// 线程函数
void* thread_func(void* arg) {
    // 线程代码
    return NULL;
}

// 创建线程
int tid = thread_create(thread_func, arg);
waittid(tid);
```

### 文件操作

```c
int fd = openat(AT_FDCWD, "/tmp/test.txt", O_RDWR | O_CREATE, 0);
write(fd, "Hello", 5);
close(fd);
```

### 同步原语

```c
// 互斥量
int mutex_id = mutex_create(false);
mutex_lock(mutex_id);
// 临界区
mutex_unlock(mutex_id);

// 信号量
int sem_id = semaphore_create(1);
semaphore_down(sem_id);
// 临界区
semaphore_up(sem_id);
```

---

## 未来扩展方向

### 短期目标

1. **网络系统调用** - socket, bind, connect等
2. **IPC机制** - 消息队列、共享内存
3. **高级文件操作** - sendfile, splice
4. **扩展clone** - 完整的Linux clone实现

### 中期目标

1. **权限系统** - 真正的用户/组管理
2. **性能优化** - 批量系统调用、vDSO
3. **安全增强** - seccomp, capabilities
4. **设备驱动** - 更多设备支持

### 长期目标

1. **容器支持** - namespace, cgroup
2. **实时调度** - SCHED_FIFO, SCHED_RR
3. **NUMA支持** - CPU亲和性
4. **异步I/O** - io_uring

---

## 提交历史

### 2026-02-14 新增系统调用

**Commit**: ff2bfcb
**分支**: zjy-syscall

**变更内容**:
- 添加SYSCALL_GETUID (174)
- 添加SYSCALL_GETEUID (175)
- 添加SYSCALL_GETGID (176)
- 添加SYSCALL_GETEGID (177)
- 添加SYSCALL_EXIT_GROUP (94)

**实现文件**:
- `os/src/syscall/mod.rs` - 常量定义和分发
- `os/src/syscall/process.rs` - 函数实现

**设计说明**:
- 所有用户/组ID返回0（单用户系统）
- exit_group调用exit实现（单进程模型）
- 与xv6-lab保持一致的实现方式
- 提高与Linux应用的兼容性

---

## 参考资料

1. [rCore-Tutorial Book](https://rcore-os.cn/rCore-Tutorial-Book-v3/)
2. [Linux系统调用手册](https://man7.org/linux/man-pages/)
3. [RISC-V特权架构规范](https://riscv.org/technical/specifications/)
4. [Rust异步编程](https://rust-lang.github.io/async-book/)
5. [xv6-lab系统调用文档](./xv6-lab系统调用详细文档.md)

---

## 调试和追踪

### 环境变量配置

```bash
# 追踪特定进程
export TRACE_PID=2

# 追踪特定程序
export TRACE_NAME=busybox

# 重新编译
make clean && make debug
```

### GDB调试

```bash
# 启动调试会话
make debug

# 在GDB中设置断点
(gdb) break syscall
(gdb) continue
```

### 日志级别

系统调用使用不同的日志级别：
- `trace!` - 详细追踪信息
- `info!` - 一般信息
- `warn!` - 警告信息
- `error!` - 错误信息

---

## 常见问题

### Q1: 为什么getuid等总是返回0？

A: rCore-lab是单用户教学系统，没有实现完整的权限系统。所有进程都以"超级用户"身份运行。

### Q2: 为什么没有clone系统调用？

A: rCore-lab使用专门的thread_create替代clone。fork用于创建进程，thread_create用于创建线程。

### Q3: 如何添加新的系统调用？

A:
1. 在`mod.rs`中添加`const SYSCALL_XXX`定义
2. 在对应模块中实现`sys_xxx`函数
3. 在`syscall()`分发函数中添加case分支
4. 编译测试

### Q4: 系统调用如何传递字符串？

A: 使用`translated_str()`函数从用户空间安全地读取字符串，自动处理虚拟地址转换和边界检查。

### Q5: 如何处理系统调用错误？

A: 返回负的errno值，例如：
```rust
return errno(EINVAL);  // 返回-22
```

---

**文档作者**: Claude (Anthropic AI)
**创建日期**: 2026-02-14
**更新日期**: 2026-02-14
**版本**: 1.0
**项目路径**: `/Users/mac/Desktop/project/rcore-lab/os`
**文档位置**: `/Users/mac/Desktop/project/rcore-lab/docs/ZJYDocs/rcore-lab系统调用详细文档.md`

---

*本文档基于rCore-lab源代码分析生成，详细记录了所有系统调用的实现细节。新增的5个系统调用提高了与Linux应用的兼容性。*
