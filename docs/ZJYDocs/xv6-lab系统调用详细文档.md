# xv6-lab 操作系统系统调用详细文档

## 项目概述

**项目路径**: `/Users/mac/Desktop/project/xv6-lab`
**架构**: RISC-V 64位
**系统调用总数**: 100+
**兼容性**: Linux ABI 兼容

xv6-lab 是一个功能完整的类Unix操作系统，基于经典的xv6教学操作系统扩展而来，实现了完整的Linux兼容系统调用接口。

---

## 系统调用架构

### 调用流程

```
用户程序 → 系统调用包装函数 (ulib.c)
    ↓
syscall宏 (syscall.h) → ecall指令
    ↓
内核陷阱处理 (trampoline.S)
    ↓
syscall_handler() (syscall.c) → 查找分发表
    ↓
具体实现函数 (sysproc.c/sysfile.c等)
    ↓
返回值通过a0寄存器返回用户程序
```

### 参数传递约定

- **系统调用号**: a7寄存器
- **参数1-6**: a0-a5寄存器
- **返回值**: a0寄存器
- **错误码**: 负数表示错误（Linux errno约定）

---

## 一、进程管理系统调用

### 1. fork - 创建子进程

**系统调用名**: fork
**系统调用函数**: `sys_clone`
**系统调用号**: 220 (SYS_clone)
**入参**:
- flags: 17 (SIGCHLD)
- stack: 0
- ptid: 0
- tls: 0
- ctid: 0

**功能**: 创建当前进程的副本。子进程获得父进程的地址空间、打开的文件描述符、信号处理器等的完整拷贝。
**返回值**:
- 父进程: 子进程的PID
- 子进程: 0
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_clone()`

---

### 2. exit - 进程退出

**系统调用名**: exit
**系统调用函数**: `sys_exit`
**系统调用号**: 93 (SYS_exit)
**入参**:
- status (int): 退出状态码

**功能**: 终止当前进程的执行，释放进程资源，唤醒等待的父进程。
**返回值**: 无返回（进程已终止）

**实现位置**: `src/syscall/sysproc.c:sys_exit()`

---

### 3. wait - 等待子进程

**系统调用名**: wait
**系统调用函数**: `sys_wait`
**系统调用号**: 260 (SYS_wait4)
**入参**:
- status (int*): 存储子进程退出状态的指针

**功能**: 阻塞等待任意子进程退出，并获取其退出状态。
**返回值**:
- 成功: 子进程的PID
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_wait()`

---

### 4. clone - Linux clone系统调用

**系统调用名**: clone
**系统调用函数**: `sys_clone`
**系统调用号**: 220 (SYS_clone)
**入参**:
- flags (unsigned long): 控制标志
  - CLONE_VM (0x100): 共享虚拟内存
  - CLONE_FS (0x200): 共享文件系统信息
  - CLONE_FILES (0x400): 共享打开的文件
  - CLONE_SIGHAND (0x800): 共享信号处理器
  - CLONE_SETTLS (0x80000): 设置TLS
  - CLONE_PARENT_SETTID (0x100000): 在父进程中写入TID
  - CLONE_CHILD_SETTID (0x1000000): 在子进程中写入TID
  - CLONE_CHILD_CLEARTID (0x200000): 子进程退出时清除TID
- stack (void*): 子进程栈指针
- ptid (int*): 父进程TID存储位置
- tls (void*): TLS指针
- ctid (int*): 子进程TID存储位置

**功能**: 创建新进程或线程，提供细粒度控制。支持进程、线程创建，TLS设置等。
**返回值**:
- 父进程: 子进程/线程的TID
- 子进程: 0
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_clone()`

---

### 5. execve - 执行程序

**系统调用名**: execve
**系统调用函数**: `sys_execve`
**系统调用号**: 221 (SYS_execve)
**入参**:
- path (const char*): 可执行文件路径
- argv (char* const[]): 参数数组
- envp (char* const[]): 环境变量数组

**功能**: 用新程序替换当前进程的地址空间。成功则不返回，失败返回-1。
**返回值**:
- 成功: 不返回
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_execve()`

---

### 6. getpid - 获取进程ID

**系统调用名**: getpid
**系统调用函数**: `sys_getpid`
**系统调用号**: 172 (SYS_getpid)
**入参**: 无

**功能**: 返回当前进程的进程ID。
**返回值**: 当前进程的PID

**实现位置**: `src/syscall/sysproc.c:sys_getpid()`

---

### 7. getppid - 获取父进程ID

**系统调用名**: getppid
**系统调用函数**: `sys_getppid`
**系统调用号**: 173 (SYS_getppid)
**入参**: 无

**功能**: 返回当前进程的父进程ID。
**返回值**: 父进程的PID

**实现位置**: `src/syscall/sysproc.c:sys_getppid()`

---

### 8. gettid - 获取线程ID

**系统调用名**: gettid
**系统调用函数**: `sys_gettid`
**系统调用号**: 178 (SYS_gettid)
**入参**: 无

**功能**: 返回当前线程的线程ID。
**返回值**: 当前线程的TID

**实现位置**: `src/syscall/sysproc.c:sys_gettid()`

---

### 9. exit_group - 进程组退出

**系统调用名**: exit_group
**系统调用函数**: `sys_exit_group`
**系统调用号**: 94 (SYS_exit_group)
**入参**:
- status (int): 退出状态码

**功能**: 终止进程组中的所有线程。
**返回值**: 无返回（进程已终止）

**实现位置**: `src/syscall/sysproc.c:sys_exit_group()`

---

### 10. set_tid_address - 设置清除子TID地址

**系统调用名**: set_tid_address
**系统调用函数**: `sys_set_tid_address`
**系统调用号**: 96 (SYS_set_tid_address)
**入参**:
- tidptr (int*): 线程ID存储地址

**功能**: 设置清除子TID的地址，用于支持CLONE_CHILD_CLEARTID。当线程退出时，内核会在此地址写入0并唤醒futex等待者。
**返回值**: 当前线程的TID

**实现位置**: `src/syscall/sysproc.c:sys_set_tid_address()`

---

### 11. sbrk - 扩展堆内存

**系统调用名**: sbrk
**系统调用函数**: `sys_sbrk`
**系统调用号**: 1002 (SYS_xv6_sbrk, xv6兼容)
**入参**:
- n (int): 增长的字节数（可为负）

**功能**: 扩展或收缩进程的堆内存。
**返回值**:
- 成功: 旧堆顶地址
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_sbrk()`

---

### 12. sleep - 进程睡眠

**系统调用名**: sleep
**系统调用函数**: `sys_sleep`
**系统调用号**: 1003 (SYS_xv6_sleep, xv6兼容)
**入参**:
- n (int): 睡眠的时钟滴答数

**功能**: 使当前进程睡眠指定的时钟滴答数。
**返回值**: 0

**实现位置**: `src/syscall/sysproc.c:sys_sleep()`

---

### 13. uptime - 获取系统运行时间

**系统调用名**: uptime
**系统调用函数**: `sys_uptime`
**系统调用号**: 1004 (SYS_xv6_uptime, xv6兼容)
**入参**: 无

**功能**: 返回系统启动以来的时钟滴答数。
**返回值**: 时钟滴答数

**实现位置**: `src/syscall/systime.c:sys_uptime()`

---

### 14-17. 获取用户/组ID

**系统调用名**: getuid / geteuid / getgid / getegid
**系统调用函数**: `sys_getuid` / `sys_geteuid` / `sys_getgid` / `sys_getegid`
**系统调用号**: 174 / 175 / 176 / 177
**入参**: 无

**功能**: 返回用户ID和组ID（xv6-lab为单用户系统，均返回0）
**返回值**: 0

**实现位置**: `src/syscall/sysproc.c`

---

## 二、文件I/O系统调用

### 18. read - 读取文件

**系统调用名**: read
**系统调用函数**: `sys_read`
**系统调用号**: 63 (SYS_read)
**入参**:
- fd (int): 文件描述符
- buf (void*): 读取缓冲区
- count (size_t): 要读取的字节数

**功能**: 从文件描述符读取数据到缓冲区。
**返回值**:
- 成功: 实际读取的字节数
- EOF: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_read()`

---

### 19. write - 写入文件

**系统调用名**: write
**系统调用函数**: `sys_write`
**系统调用号**: 64 (SYS_write)
**入参**:
- fd (int): 文件描述符
- buf (const void*): 写入数据缓冲区
- count (size_t): 要写入的字节数

**功能**: 将缓冲区数据写入文件描述符。
**返回值**:
- 成功: 实际写入的字节数
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_write()`

---

### 20. writev - 向量写入

**系统调用名**: writev
**系统调用函数**: `sys_writev`
**系统调用号**: 66 (SYS_writev)
**入参**:
- fd (int): 文件描述符
- iov (const struct iovec*): iovec数组
- iovcnt (int): 数组元素个数

**功能**: 从多个缓冲区写入数据到文件描述符。
**返回值**:
- 成功: 实际写入的字节数
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_writev()`

---

### 21. open - 打开文件

**系统调用名**: open
**系统调用函数**: `sys_open`
**系统调用号**: 1024 (SYS_xv6_open, xv6兼容)
**入参**:
- path (const char*): 文件路径
- flags (int): 打开标志
  - O_RDONLY (0): 只读
  - O_WRONLY (1): 只写
  - O_RDWR (2): 读写
  - O_CREATE (0x200): 不存在则创建
  - O_TRUNC (0x400): 截断文件
  - O_DIRECTORY (0x10000): 必须是目录

**功能**: 打开或创建文件。
**返回值**:
- 成功: 文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_open()`

---

### 22. openat - 相对于目录FD打开文件

**系统调用名**: openat
**系统调用函数**: `sys_openat`
**系统调用号**: 56 (SYS_openat)
**入参**:
- dirfd (int): 目录文件描述符（AT_FDCWD=-100表示当前目录）
- path (const char*): 文件路径
- flags (int): 打开标志
- mode (mode_t): 创建模式

**功能**: 相对于目录文件描述符打开文件。支持Linux flags转换。
**返回值**:
- 成功: 文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_openat()`

**Linux flags转换**:
- Linux O_CREAT (0x40) → 内核 O_CREATE (0x200)
- Linux O_TRUNC (0x200) → 内核 O_TRUNC (0x400)
- Linux O_DIRECTORY (0x200000) → 内核 O_DIRECTORY (0x10000)

---

### 23. close - 关闭文件

**系统调用名**: close
**系统调用函数**: `sys_close`
**系统调用号**: 57 (SYS_close)
**入参**:
- fd (int): 文件描述符

**功能**: 关闭文件描述符，释放相关资源。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_close()`

---

### 24. dup - 复制文件描述符

**系统调用名**: dup
**系统调用函数**: `sys_dup`
**系统调用号**: 23 (SYS_dup)
**入参**:
- oldfd (int): 要复制的文件描述符

**功能**: 复制文件描述符，新旧描述符指向同一文件。
**返回值**:
- 成功: 新文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_dup()`

---

### 25. dup3 - 复制文件描述符到指定位置

**系统调用名**: dup3
**系统调用函数**: `sys_dup3`
**系统调用号**: 24 (SYS_dup3)
**入参**:
- oldfd (int): 要复制的文件描述符
- newfd (int): 目标文件描述符
- flags (int): 标志（支持O_CLOEXEC）

**功能**: 复制文件描述符到指定位置。
**返回值**:
- 成功: 新文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_dup3()`

---

### 26. lseek - 设置文件偏移

**系统调用名**: lseek
**系统调用函数**: `sys_lseek`
**系统调用号**: 62 (SYS_lseek)
**入参**:
- fd (int): 文件描述符
- offset (off_t): 偏移量
- whence (int): 基准位置
  - SEEK_SET (0): 文件开头
  - SEEK_CUR (1): 当前位置
  - SEEK_END (2): 文件末尾

**功能**: 设置文件读写位置。
**返回值**:
- 成功: 新的文件偏移
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_lseek()`

---

### 27. fstat - 获取文件状态

**系统调用名**: fstat
**系统调用函数**: `sys_fstat`
**系统调用号**: 80 (SYS_fstat)
**入参**:
- fd (int): 文件描述符
- st (struct stat*): stat结构体指针

**功能**: 获取文件的元数据（大小、类型、inode等）。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_fstat()`

---

### 28. fstatat - 相对于目录FD获取文件状态

**系统调用名**: fstatat
**系统调用函数**: `sys_fstatat`
**系统调用号**: 79 (SYS_newfstatat)
**入参**:
- dirfd (int): 目录文件描述符
- path (const char*): 文件路径
- st (struct stat*): stat结构体指针
- flags (int): 标志

**功能**: 相对于目录FD获取文件状态。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_fstatat()`

---

### 29. ftruncate - 截断文件

**系统调用名**: ftruncate
**系统调用函数**: `sys_ftruncate`
**系统调用号**: 46 (SYS_ftruncate)
**入参**:
- fd (int): 文件描述符
- length (off_t): 新文件大小

**功能**: 将文件截断到指定长度。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_ftruncate()`

---

### 30. fcntl - 文件控制

**系统调用名**: fcntl
**系统调用函数**: `sys_fcntl`
**系统调用号**: 25 (SYS_fcntl)
**入参**:
- fd (int): 文件描述符
- cmd (int): 控制命令
  - F_GETFL (3): 获取文件状态标志
  - F_SETFL (4): 设置文件状态标志
  - F_GETFD (1): 获取文件描述符标志
  - F_SETFD (2): 设置文件描述符标志
  - F_DUPFD (0): 复制文件描述符
- arg (unsigned long): 命令参数

**功能**: 对文件描述符进行各种控制操作。
**返回值**: 根据cmd不同而不同

**实现位置**: `src/syscall/sysfile.c:sys_fcntl()`

---

### 31. ioctl - I/O控制

**系统调用名**: ioctl
**系统调用函数**: `sys_ioctl`
**系统调用号**: 29 (SYS_ioctl)
**入参**:
- fd (int): 文件描述符
- request (unsigned long): 控制请求
- arg (unsigned long): 请求参数

**功能**: 对设备进行I/O控制操作。
**返回值**: 根据request不同而不同

**实现位置**: `src/syscall/sysfile.c:sys_ioctl()`

---

## 三、目录和文件系统操作

### 32. chdir - 改变当前工作目录

**系统调用名**: chdir
**系统调用函数**: `sys_chdir`
**系统调用号**: 49 (SYS_chdir)
**入参**:
- path (const char*): 目标目录路径

**功能**: 改变当前进程的工作目录。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_chdir()`

---

### 33. getcwd - 获取当前工作目录

**系统调用名**: getcwd
**系统调用函数**: `sys_getcwd`
**系统调用号**: 17 (SYS_getcwd)
**入参**:
- buf (char*): 缓冲区
- size (size_t): 缓冲区大小

**功能**: 获取当前工作目录的完整路径。
**返回值**:
- 成功: buf指针
- 失败: NULL

**实现位置**: `src/syscall/sysfile.c:sys_getcwd()`

---

### 34. mkdir - 创建目录

**系统调用名**: mkdir
**系统调用函数**: `sys_mkdirat`
**系统调用号**: 1025 (SYS_xv6_mkdir, xv6兼容)
**入参**:
- path (const char*): 目录路径

**功能**: 创建新目录。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_mkdirat()`

---

### 35. mkdirat - 相对于目录FD创建目录

**系统调用名**: mkdirat
**系统调用函数**: `sys_mkdirat`
**系统调用号**: 34 (SYS_mkdirat)
**入参**:
- dirfd (int): 目录文件描述符
- path (const char*): 目录路径
- mode (mode_t): 权限模式

**功能**: 相对于目录FD创建新目录。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_mkdirat()`

---

### 36. getdents64 - 读取目录条目

**系统调用名**: getdents64
**系统调用函数**: `sys_getdents64`
**系统调用号**: 61 (SYS_getdents64)
**入参**:
- fd (int): 目录文件描述符
- dirp (void*): 目录条目缓冲区
- count (unsigned int): 缓冲区大小

**功能**: 读取目录中的文件条目。
**返回值**:
- 成功: 读取的字节数
- EOF: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_getdents64()`

---

### 37. unlink - 删除文件

**系统调用名**: unlink
**系统调用函数**: `sys_unlinkat`
**系统调用号**: 1026 (SYS_xv6_unlink, xv6兼容)
**入参**:
- path (const char*): 文件路径

**功能**: 删除文件或目录。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_unlinkat()`

---

### 38. unlinkat - 相对于目录FD删除文件

**系统调用名**: unlinkat
**系统调用函数**: `sys_unlinkat`
**系统调用号**: 35 (SYS_unlinkat)
**入参**:
- dirfd (int): 目录文件描述符
- path (const char*): 文件路径
- flags (int): 标志（AT_REMOVEDIR表示删除目录）

**功能**: 相对于目录FD删除文件或目录。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_unlinkat()`

---

### 39. link - 创建硬链接

**系统调用名**: link
**系统调用函数**: `sys_linkat`
**系统调用号**: 1027 (SYS_xv6_link, xv6兼容)
**入参**:
- oldpath (const char*): 现有文件路径
- newpath (const char*): 新链接路径

**功能**: 创建文件的硬链接。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_linkat()`

---

### 40. mknod - 创建特殊文件

**系统调用名**: mknod
**系统调用函数**: `sys_mknod`
**系统调用号**: 1028 (SYS_xv6_mknod, xv6兼容)
**入参**:
- path (const char*): 文件路径
- major (short): 主设备号
- minor (short): 次设备号

**功能**: 创建设备特殊文件。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_mknod()`

---

## 四、管道和特殊文件

### 41. pipe - 创建管道

**系统调用名**: pipe
**系统调用函数**: `sys_pipe2`
**系统调用号**: 1029 (SYS_xv6_pipe, xv6兼容)
**入参**:
- fds (int[2]): 存储管道读写端文件描述符的数组

**功能**: 创建单向数据通道，fds[0]为读端，fds[1]为写端。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_pipe2()`

---

### 42. pipe2 - 创建管道（带标志）

**系统调用名**: pipe2
**系统调用函数**: `sys_pipe2`
**系统调用号**: 59 (SYS_pipe2)
**入参**:
- fds (int[2]): 存储管道文件描述符的数组
- flags (int): 标志（O_CLOEXEC, O_NONBLOCK等）

**功能**: 创建管道，支持额外标志。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_pipe2()`

---

### 43. eventfd2 - 创建事件通知文件描述符

**系统调用名**: eventfd2
**系统调用函数**: `sys_eventfd2`
**系统调用号**: 19 (SYS_eventfd2)
**入参**:
- initval (unsigned int): 初始计数值
- flags (int): 标志
  - EFD_CLOEXEC (0x2000000): exec时关闭
  - EFD_NONBLOCK (0x4000): 非阻塞模式
  - EFD_SEMAPHORE (0x1): 信号量模式

**功能**: 创建用于事件通知的文件描述符。
**返回值**:
- 成功: eventfd文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_eventfd2()`

---

### 44. sendfile - 文件间数据传输

**系统调用名**: sendfile
**系统调用函数**: `sys_sendfile`
**系统调用号**: 71 (SYS_sendfile)
**入参**:
- out_fd (int): 输出文件描述符
- in_fd (int): 输入文件描述符
- offset (off_t*): 输入文件偏移指针
- count (size_t): 要传输的字节数

**功能**: 在两个文件描述符之间高效传输数据。
**返回值**:
- 成功: 传输的字节数
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_sendfile()`

---

### 45-46. 符号链接操作

**系统调用名**: symlink / symlinkat
**系统调用函数**: `sys_symlink` / `sys_symlinkat`
**系统调用号**: 1030 (SYS_xv6_symlink, xv6兼容) / 36 (SYS_symlinkat)
**入参**:
- target (const char*): 目标路径
- linkpath (const char*): 链接路径

**功能**: 创建符号链接。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysfile.c:sys_symlink()`

---

## 五、网络系统调用

### 47. socket - 创建套接字

**系统调用名**: socket
**系统调用函数**: `sys_socket`
**系统调用号**: 198 (SYS_socket)
**入参**:
- domain (int): 协议族（AF_INET=2）
- type (int): 套接字类型（SOCK_STREAM=1, SOCK_DGRAM=2）
- protocol (int): 协议（通常为0）

**功能**: 创建网络通信端点。
**返回值**:
- 成功: 套接字文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_socket()`

---

### 48. bind - 绑定套接字地址

**系统调用名**: bind
**系统调用函数**: `sys_bind`
**系统调用号**: 200 (SYS_bind)
**入参**:
- sockfd (int): 套接字文件描述符
- addr (const struct sockaddr*): 地址结构
- addrlen (socklen_t): 地址结构长度

**功能**: 将套接字绑定到指定地址和端口。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_bind()`

---

### 49. connect - 连接到远程地址

**系统调用名**: connect
**系统调用函数**: `sys_connect`
**系统调用号**: 203 (SYS_connect)
**入参**:
- sockfd (int): 套接字文件描述符
- addr (const struct sockaddr*): 远程地址结构
- addrlen (socklen_t): 地址结构长度

**功能**: 在套接字上发起连接到指定地址。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_connect()`

---

### 50. listen - 监听连接

**系统调用名**: listen
**系统调用函数**: `sys_listen`
**系统调用号**: 201 (SYS_listen)
**入参**:
- sockfd (int): 套接字文件描述符
- backlog (int): 连接队列最大长度

**功能**: 将套接字标记为被动监听连接请求。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_listen()`

---

### 51. accept - 接受连接

**系统调用名**: accept
**系统调用函数**: `sys_accept`
**系统调用号**: 202 (SYS_accept)
**入参**:
- sockfd (int): 监听套接字文件描述符
- addr (struct sockaddr*): 客户端地址结构（输出）
- addrlen (socklen_t*): 地址结构长度（输入/输出）

**功能**: 接受监听套接字上的连接请求。
**返回值**:
- 成功: 新连接的套接字文件描述符
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_accept()`

---

### 52. sendto - 发送数据到指定地址

**系统调用名**: sendto
**系统调用函数**: `sys_sendto`
**系统调用号**: 206 (SYS_sendto)
**入参**:
- sockfd (int): 套接字文件描述符
- buf (const void*): 发送数据缓冲区
- len (size_t): 数据长度
- flags (int): 发送标志
- dest_addr (const struct sockaddr*): 目标地址
- addrlen (socklen_t): 地址结构长度

**功能**: 通过套接字发送数据到指定地址（主要用于UDP）。
**返回值**:
- 成功: 发送的字节数
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_sendto()`

---

### 53. recvfrom - 从指定地址接收数据

**系统调用名**: recvfrom
**系统调用函数**: `sys_recvfrom`
**系统调用号**: 207 (SYS_recvfrom)
**入参**:
- sockfd (int): 套接字文件描述符
- buf (void*): 接收数据缓冲区
- len (size_t): 缓冲区大小
- flags (int): 接收标志
- src_addr (struct sockaddr*): 源地址（输出）
- addrlen (socklen_t*): 地址结构长度（输入/输出）

**功能**: 从套接字接收数据并获取源地址。
**返回值**:
- 成功: 接收的字节数
- 失败: -1

**实现位置**: `src/syscall/sysnet.c:sys_recvfrom()`

**网络栈**: 基于 open-npstack 网络协议栈实现

---

## 六、System V IPC系统调用

### 54-57. 消息队列操作

**系统调用名**: msgget / msgsnd / msgrcv / msgctl
**系统调用函数**: `sys_msgget` / `sys_msgsnd` / `sys_msgrcv` / `sys_msgctl`
**系统调用号**: 186 / 187 / 188 / 189

**msgget - 创建或访问消息队列**
**入参**:
- key (key_t): 消息队列键值
- msgflg (int): 创建标志（IPC_CREAT等）

**功能**: 创建新的消息队列或访问现有队列。
**返回值**:
- 成功: 消息队列标识符
- 失败: -1

**msgsnd - 发送消息**
**入参**:
- msqid (int): 消息队列标识符
- msgp (const void*): 消息指针
- msgsz (size_t): 消息大小
- msgflg (int): 发送标志

**功能**: 向消息队列发送消息。
**返回值**:
- 成功: 0
- 失败: -1

**msgrcv - 接收消息**
**入参**:
- msqid (int): 消息队列标识符
- msgp (void*): 消息缓冲区
- msgsz (size_t): 缓冲区大小
- msgtyp (long): 消息类型
- msgflg (int): 接收标志

**功能**: 从消息队列接收消息。
**返回值**:
- 成功: 接收的字节数
- 失败: -1

**msgctl - 消息队列控制**
**入参**:
- msqid (int): 消息队列标识符
- cmd (int): 控制命令（IPC_STAT, IPC_RMID等）
- buf (struct msqid_ds*): 数据结构

**功能**: 对消息队列执行控制操作。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysmsg.c`

---

### 58-61. 共享内存操作

**系统调用名**: shmget / shmat / shmdt / shmctl
**系统调用函数**: `sys_shmget` / `sys_shmat` / `sys_shmdt` / `sys_shmctl`
**系统调用号**: 194 / 196 / 197 / 195

**shmget - 获取共享内存段**
**入参**:
- key (key_t): 共享内存键值
- size (size_t): 内存段大小
- shmflg (int): 创建标志

**功能**: 创建或访问共享内存段。
**返回值**:
- 成功: 共享内存标识符
- 失败: -1

**shmat - 连接共享内存**
**入参**:
- shmid (int): 共享内存标识符
- shmaddr (const void*): 连接地址（通常为NULL）
- shmflg (int): 连接标志

**功能**: 将共享内存段连接到进程地址空间。
**返回值**:
- 成功: 共享内存起始地址
- 失败: (void*)-1

**shmdt - 分离共享内存**
**入参**:
- shmaddr (const void*): 共享内存地址

**功能**: 将共享内存段从进程地址空间分离。
**返回值**:
- 成功: 0
- 失败: -1

**shmctl - 共享内存控制**
**入参**:
- shmid (int): 共享内存标识符
- cmd (int): 控制命令
- buf (struct shmid_ds*): 数据结构

**功能**: 对共享内存段执行控制操作。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: 在 syscall.c 中注册，具体实现在其他源文件

---

## 七、信号处理系统调用

### 62. kill - 发送信号

**系统调用名**: kill
**系统调用函数**: `sys_kill`
**系统调用号**: 129 (SYS_kill)
**入参**:
- pid (pid_t): 目标进程ID
- signum (int): 信号编号

**功能**: 向指定进程发送信号。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_kill()`

---

### 63. rt_sigaction - 注册信号处理器

**系统调用名**: rt_sigaction
**系统调用函数**: `sys_rt_sigaction`
**系统调用号**: 134 (SYS_rt_sigaction)
**入参**:
- signum (int): 信号编号
- act (const struct sigaction*): 新的信号处理动作
- oldact (struct sigaction*): 旧的信号处理动作（输出）
- sigsetsize (size_t): 信号集大小

**功能**: 设置信号处理函数。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_rt_sigaction()`

---

### 64. rt_sigprocmask - 设置信号掩码

**系统调用名**: rt_sigprocmask
**系统调用函数**: `sys_rt_sigprocmask`
**系统调用号**: 135 (SYS_rt_sigprocmask)
**入参**:
- how (int): 操作方式
  - SIG_BLOCK (0): 添加信号到掩码
  - SIG_UNBLOCK (1): 从掩码移除信号
  - SIG_SETMASK (2): 设置新掩码
- set (const sigset_t*): 新信号集
- oldset (sigset_t*): 旧信号集（输出）
- sigsetsize (size_t): 信号集大小

**功能**: 修改进程的信号屏蔽字。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_rt_sigprocmask()`

---

### 65. rt_sigtimedwait - 等待信号

**系统调用名**: rt_sigtimedwait
**系统调用函数**: `sys_rt_sigtimedwait`
**系统调用号**: 137 (SYS_rt_sigtimedwait)
**入参**:
- set (const sigset_t*): 等待的信号集
- info (siginfo_t*): 信号信息（输出）
- timeout (const struct timespec*): 超时时间
- sigsetsize (size_t): 信号集大小

**功能**: 等待信号集中的信号到达。
**返回值**:
- 成功: 信号编号
- 超时: -EAGAIN
- 失败: -1

**实现位置**: `src/syscall/sysproc.c:sys_rt_sigtimedwait()`

---

### 66. rt_sigreturn - 从信号处理器返回

**系统调用名**: rt_sigreturn
**系统调用函数**: `sys_rt_sigreturn`
**系统调用号**: 139 (SYS_rt_sigreturn)
**入参**: 无（从栈帧恢复上下文）

**功能**: 从信号处理器返回，恢复被中断的执行上下文。
**返回值**: 不返回（恢复原执行流）

**实现位置**: `src/syscall/sysproc.c:sys_rt_sigreturn()`

---

## 八、时间相关系统调用

### 67. gettimeofday - 获取当前时间

**系统调用名**: gettimeofday
**系统调用函数**: `sys_gettimeofday`
**系统调用号**: 169 (SYS_gettimeofday)
**入参**:
- tv (struct timeval*): 时间结构（输出）
- tz (struct timezone*): 时区（已废弃，应为NULL）

**功能**: 获取当前系统时间（秒和微秒）。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/systime.c:sys_gettimeofday()`

---

### 68. clock_gettime - 获取时钟时间

**系统调用名**: clock_gettime
**系统调用函数**: `sys_clock_gettime`
**系统调用号**: 113 (SYS_clock_gettime)
**入参**:
- clockid (clockid_t): 时钟ID
  - CLOCK_REALTIME (0): 系统实时时钟
  - CLOCK_MONOTONIC (1): 单调递增时钟
- tp (struct timespec*): 时间结构（输出）

**功能**: 获取指定时钟的当前时间。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: `src/syscall/systime.c:sys_clock_gettime()`

---

### 69. times - 获取进程时间

**系统调用名**: times
**系统调用函数**: `sys_times`
**系统调用号**: 153 (SYS_times)
**入参**:
- tms (struct tms*): 时间结构（输出）
  - tms_utime: 用户态CPU时间
  - tms_stime: 内核态CPU时间
  - tms_cutime: 子进程用户态时间
  - tms_cstime: 子进程内核态时间

**功能**: 获取进程和子进程的CPU时间。
**返回值**:
- 成功: 系统启动以来的时钟滴答数
- 失败: -1

**实现位置**: `src/syscall/systime.c:sys_times()`

---

### 70. nanosleep - 纳秒级睡眠

**系统调用名**: nanosleep
**系统调用函数**: `sys_nanosleep`
**系统调用号**: 101 (SYS_nanosleep)
**入参**:
- req (const struct timespec*): 请求睡眠时间
- rem (struct timespec*): 剩余时间（如被信号中断）

**功能**: 使进程睡眠指定的纳秒数。
**返回值**:
- 成功: 0
- 被中断: -1 (EINTR)

**实现位置**: 在 syscall.c 中注册

---

## 九、内存管理系统调用

### 71. brk - 设置堆顶

**系统调用名**: brk
**系统调用函数**: `sys_brk`
**系统调用号**: 214 (SYS_brk)
**入参**:
- addr (void*): 新的堆顶地址

**功能**: 设置进程数据段的结束地址。
**返回值**:
- 成功: 新的堆顶地址
- 失败: 当前堆顶地址

**实现位置**: 在 syscall.c 中注册

---

### 72. mmap - 内存映射

**系统调用名**: mmap
**系统调用函数**: `sys_mmap`
**系统调用号**: 222 (SYS_mmap)
**入参**:
- addr (void*): 映射起始地址（建议）
- length (size_t): 映射长度
- prot (int): 保护标志（PROT_READ, PROT_WRITE等）
- flags (int): 映射标志（MAP_SHARED, MAP_PRIVATE等）
- fd (int): 文件描述符（文件映射）
- offset (off_t): 文件偏移

**功能**: 在进程地址空间创建内存映射。
**返回值**:
- 成功: 映射起始地址
- 失败: MAP_FAILED

**实现位置**: 在 syscall.c 中注册

---

### 73. munmap - 解除内存映射

**系统调用名**: munmap
**系统调用函数**: `sys_munmap`
**系统调用号**: 215 (SYS_munmap)
**入参**:
- addr (void*): 映射起始地址
- length (size_t): 映射长度

**功能**: 解除进程地址空间的内存映射。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: 在 syscall.c 中注册

---

### 74. mprotect - 修改内存保护

**系统调用名**: mprotect
**系统调用函数**: `sys_mprotect`
**系统调用号**: 226 (SYS_mprotect)
**入参**:
- addr (void*): 内存区域起始地址
- len (size_t): 区域长度
- prot (int): 新的保护标志

**功能**: 修改内存区域的访问保护属性。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: 在 syscall.c 中注册

---

## 十、其他系统调用

### 75. uname - 获取系统信息

**系统调用名**: uname
**系统调用函数**: `sys_uname`
**系统调用号**: 160 (SYS_uname)
**入参**:
- buf (struct utsname*): 系统信息结构

**功能**: 获取系统名称和版本信息。
**返回值**:
- 成功: 0
- 失败: -1

**实现位置**: 在 syscall.c 中注册

---

### 76. reboot - 重启系统

**系统调用名**: reboot
**系统调用函数**: `sys_reboot`
**系统调用号**: 142 (SYS_reboot)
**入参**:
- magic1 (int): 魔数1
- magic2 (int): 魔数2
- cmd (int): 重启命令
- arg (void*): 命令参数

**功能**: 重启或关闭系统。
**返回值**:
- 成功: 不返回
- 失败: -1

**实现位置**: 在 syscall.c 中注册

---

## 系统调用统计

### 按类别统计

| 类别 | 数量 | 主要文件 |
|------|------|---------|
| 进程管理 | 17 | sysproc.c |
| 文件I/O | 25 | sysfile.c |
| 网络 | 7 | sysnet.c |
| IPC | 8 | sysmsg.c 及其他 |
| 信号 | 5 | sysproc.c |
| 时间 | 4 | systime.c |
| 内存管理 | 4 | 在 syscall.c 中注册 |
| 其他 | 6+ | 各文件 |
| **总计** | **100+** | |

### 按系统调用号范围统计

- **1-99**: Linux标准系统调用（fork, exit, read, write等）
- **100-299**: Linux扩展系统调用（网络、IPC、时间等）
- **1000-1099**: xv6兼容系统调用（fork, sleep, pipe等）

---

## 核心文件清单

| 文件 | 行数 | 描述 |
|------|------|------|
| `src/syscall/syscall.h` | 373 | 系统调用定义和用户态宏 |
| `src/syscall/syscall.c` | 792 | 系统调用分发和处理中枢 |
| `src/syscall/sysproc.c` | 933 | 进程管理、信号处理 |
| `src/syscall/sysfile.c` | 1500+ | 文件I/O和文件系统操作 |
| `src/syscall/sysnet.c` | 448 | 网络系统调用 |
| `src/syscall/sysmsg.c` | 104 | System V消息队列 |
| `src/syscall/systime.c` | 110 | 时间相关系统调用 |
| `user/user.h` | 123 | 用户态系统调用声明 |
| `user/ulib.c` | 338 | 用户态系统调用包装 |

---

## 关键设计特点

### 1. Linux ABI兼容性

xv6-lab采用标准Linux系统调用号和调用约定，使得许多Linux用户程序可以直接运行。

### 2. 参数传递机制

采用RISC-V调用约定：
- a7寄存器传递系统调用号
- a0-a5寄存器传递参数
- a0寄存器返回结果

### 3. 错误处理

遵循Linux错误处理约定：
- 返回值为负表示错误（-errno）
- 返回值为0或正数表示成功

### 4. 模块化架构

不同类别的系统调用实现在独立的源文件中，便于维护和扩展。

### 5. 日志和调试

完整的系统调用跟踪机制，支持：
- 每个系统调用的进入和退出日志
- 参数和返回值记录
- 可配置的跟踪级别

### 6. 安全性

- 使用`copyin`/`copyout`函数安全地访问用户空间数据
- 参数验证和边界检查
- 地址空间隔离

### 7. 网络栈集成

基于open-npstack实现完整的TCP/IP协议栈，支持socket编程。

### 8. IPC机制

实现System V IPC，包括消息队列和共享内存，支持进程间通信。

### 9. 信号机制

完整的POSIX信号支持，包括信号处理器注册、信号掩码、信号等待等。

### 10. 时间管理

基于RISC-V mtime寄存器实现高精度时间管理（12.5MHz时基）。

---

## 与rCore-lab对比

| 特性 | xv6-lab | rCore-lab |
|------|---------|-----------|
| 架构 | RISC-V 64位 | RISC-V 64位/32位 |
| 系统调用数量 | 100+ | 80+ |
| 网络支持 | open-npstack | 部分支持 |
| IPC | System V完整 | 基础IPC |
| 信号 | POSIX信号 | 部分信号 |
| TLS支持 | CLONE_SETTLS | PT_TLS段 |
| 文件系统 | 自定义 | FAT32/EXT4/easy-fs |
| 块缓存 | 内置 | 最近新增 |

---

## 未来扩展方向

1. **更多网络系统调用**: setsockopt, getsockopt, shutdown等
2. **高级IPC**: Unix域套接字, pipe2扩展
3. **性能优化**: 批量系统调用, vDSO支持
4. **安全增强**: seccomp, capabilities
5. **调度策略**: 实时调度, CPU亲和性
6. **文件系统扩展**: 更多文件系统类型支持
7. **设备驱动**: 更丰富的设备支持

---

## 参考资料

1. [xv6-lab项目仓库](file:///Users/mac/Desktop/project/xv6-lab)
2. [Linux系统调用手册](https://man7.org/linux/man-pages/)
3. [RISC-V特权架构规范](https://riscv.org/technical/specifications/)
4. [System V IPC规范](https://pubs.opengroup.org/onlinepubs/9699919799/)
5. [POSIX信号规范](https://pubs.opengroup.org/onlinepubs/9699919799/)

---

**文档作者**: Claude (Anthropic AI)
**创建日期**: 2026-02-14
**版本**: 1.0
**项目路径**: `/Users/mac/Desktop/project/xv6-lab`
**文档位置**: `/Users/mac/Desktop/project/rcore-lab/docs/ZJYDocs/xv6-lab系统调用详细文档.md`

---

*本文档基于xv6-lab源代码分析生成，涵盖了所有主要系统调用的详细信息。如有疑问或需要补充，请参考源代码。*
