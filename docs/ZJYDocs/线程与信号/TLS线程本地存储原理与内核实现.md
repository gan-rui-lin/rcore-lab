# TLS（线程本地存储）原理与内核实现

> 日期：2026/3/11
>
> 关键词：TLS、TP 寄存器、PT_TLS、pthread、CLONE_SETTLS、set_tid_address

---

## 一、问题的起源：为什么线程需要"自己的"变量？

在多线程程序中，所有线程共享同一个地址空间——代码段、数据段、堆都是同一份。这意味着一个全局变量 `int errno`，如果线程 A 调用 `read()` 失败把 `errno` 设成 `EBADF`，线程 B 紧接着去读 `errno`，就会读到线程 A 的错误码。这显然是不可接受的。

最直接的解决方案是把 `errno` 放在栈上——但 `errno` 是 C 标准库的全局符号，无数已有代码都在用它，不可能改成局部变量。

于是就产生了 **TLS（Thread-Local Storage，线程本地存储）** 的概念：**每个线程拥有独立副本的全局变量**。语法上它仍然像全局变量一样访问，但底层每个线程看到的是自己的那份拷贝。在 C/C++ 中用 `__thread` 或 `_Thread_local`（C11）/ `thread_local`（C++11）声明：

```c
__thread int errno;          // 每个线程各有一份 errno
__thread void *thread_data;  // 每个线程的私有指针
```

编译器和链接器会把所有 `__thread` 变量汇集到 ELF 文件的一个特殊段——**PT_TLS** 段中。这个段记录了：

- TLS 模板数据的起始地址和大小（已初始化的 `.tdata`）
- BSS 部分的大小（未初始化的 `.tbss`）
- 对齐要求

PT_TLS 段本身 **不会被映射成普通的可读写内存**（它不是 PT_LOAD），它只是一个 **模板**——每当创建一个新线程时，运行时需要分配一块内存，把模板内容拷贝进去，作为该线程的 TLS 副本。

---

## 二、TP 寄存器：线程如何找到自己的 TLS

每个线程有自己的 TLS 副本，那线程执行代码时怎么知道"我的 `errno` 在哪"？答案是通过一个 **专用的 CPU 寄存器** 来指向当前线程的 TLS 区域。不同架构使用不同的寄存器：

| 架构 | TLS 寄存器 | 说明 |
|------|-----------|------|
| RISC-V | `tp`（x4） | Thread Pointer，ABI 规定专用于 TLS |
| x86_64 | `fs` 段基址 | 通过 `arch_prctl(ARCH_SET_FS, addr)` 或 `wrfsbase` 设置 |
| AArch64 | `tpidr_el0` | 用户态线程指针寄存器 |
| LoongArch64 | `$tp`（r2） | 与 RISC-V 类似的专用寄存器 |

以 RISC-V 为例。当编译器遇到对 `__thread int errno` 的访问时，生成的指令大致如下：

```asm
# 访问 __thread int errno
# 假设 errno 在 TLS 块中偏移 -16 的位置
lw  a0, -16(tp)    # 从 tp 寄存器指向的地址偏移 -16 处加载
```

这就是 `tp` 寄存器的核心作用：**它是一个基地址指针，编译器通过 `tp + offset` 来寻址当前线程的 TLS 变量**。每个线程的 `tp` 值不同，所以同样的偏移量会访问到不同的内存地址，从而实现"同一个变量名、不同线程不同值"。

`tp` 寄存器在 RISC-V ABI 中是 **保留寄存器**，编译器和操作系统约定它只能用于 TLS。普通代码不会修改它，它只在以下时刻被设置：

1. 进程启动时（exec）
2. 新线程创建时（clone + CLONE_SETTLS）

---

## 三、exec 时内核做了什么：PT_TLS 段地址写入 TP

当内核执行 `execve` 加载一个新的 ELF 可执行文件时，需要解析 ELF 的 program headers。在 rustoswhu 的实现中（`memory_set.rs:map_elf`）：

```rust
for i in 0..ph_count {
    let ph = elf.program_header(i).unwrap();
    if ph.get_type().unwrap() == xmas_elf::program::Type::Tls {
        tls_addr = ph.virtual_addr() + offset as u64;
    }
    // ... 处理 PT_LOAD 段 ...
}
```

找到 PT_TLS 段后，取其虚拟地址。然后在 `exec` 函数末尾：

```rust
trap_cx[TrapFrameArgs::TLS] = tls_addr as usize;
```

这行代码把 PT_TLS 段的地址写入 trap frame 中对应 `tp` 寄存器的字段。当进程从内核态返回用户态时，trap frame 中的值会被恢复到 CPU 寄存器中，于是用户态代码就能通过 `tp` 寄存器访问 TLS 数据了。

### 为什么这样做能工作？

对于 **静态链接的单线程程序**，这个简化实现是正确的。原因如下：

1. 静态链接时，所有 TLS 变量的偏移在链接期就确定了
2. 单线程只有一份 TLS 副本，就是 ELF 文件中 PT_TLS 段映射出来的那块内存
3. `tp` 指向这个段的起始地址，`tp + offset` 就能正确访问每个 TLS 变量

但对于 **多线程** 或 **动态链接** 的程序，事情要复杂得多——这就引出了 libc 运行时的角色。

---

## 四、libc 运行时：TLS 的真正管理者

实际上，内核在 exec 时设置的 `tp` 值只是一个 **初始值**。真正的 TLS 管理工作由用户态的 C 运行时（musl libc / glibc）完成。以 musl libc 为例，在程序启动的 `__init_tls` 函数中，发生了以下事情：

### 4.1 主线程的 TLS 初始化

```
1. 解析 auxv 中的 AT_PHDR、AT_PHNUM，找到 PT_TLS 段
2. 分配一块内存 = sizeof(struct pthread) + TLS 模板大小 + 对齐填充
3. 将 PT_TLS 段的 .tdata 内容拷贝到新分配的内存中
4. 将 .tbss 部分清零
5. 在这块内存的特定位置放置 struct pthread（线程控制块）
6. 调用 __set_thread_area() 或直接设置 TP 寄存器，指向这块内存
```

关键点：**musl 会覆盖内核在 exec 时设置的 TP 值**。musl 分配了自己的 TLS 区域，并把 `struct pthread`（线程控制块）嵌入其中。最终 `tp` 指向的不仅仅是 TLS 变量，还包含了线程的元数据（TID、取消状态、信号掩码等）。

在 RISC-V 上，musl 的内存布局大致为：

```
低地址                                                 高地址
┌──────────────┬───────────┬──────────────────────────┐
│  TLS 数据区  │  padding  │  struct pthread (TCB)    │
│ (.tdata+.tbss)│          │                          │
└──────────────┴───────────┴──────────────────────────┘
                                    ↑
                                    tp 寄存器指向这里
```

这也是为什么前面说 "编译器用 `tp + 负偏移` 来访问 TLS 变量"——TLS 数据在 `tp` 指向位置的 **低地址** 方向。

### 4.2 为什么内核仍然需要设置 PT_TLS 地址？

既然 musl 会覆盖 `tp`，内核还有必要设置它吗？**有必要**，原因有二：

1. **在 musl 的 `__init_tls` 执行之前**，如果有代码访问了 TLS 变量（比如某些 constructor 或 libc 内部初始化代码），`tp` 必须指向一个合法地址，否则会产生段错误。内核把 `tp` 设为 PT_TLS 段地址至少保证了早期 TLS 访问不会崩溃。

2. **对于不使用 musl/glibc 的极简程序**（比如直接用汇编写的或用 `nostdlib` 编译的），内核设的 `tp` 就是唯一的 TLS 基址。

---

## 五、多线程与 CLONE_SETTLS：线程创建时的 TLS

当用户态调用 `pthread_create` 创建新线程时，musl libc 内部的流程大致如下：

```
1. mmap 分配新线程的栈空间
2. 在栈顶（或额外分配的区域）创建新的 TLS 区域：
   a. 拷贝 PT_TLS 模板数据
   b. 初始化 struct pthread
   c. 设置 TID、dtv（Dynamic Thread Vector）等
3. 调用 clone 系统调用：
   clone(CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND |
         CLONE_THREAD | CLONE_SETTLS | CLONE_CHILD_SETTID |
         CLONE_CHILD_CLEARTID,
         new_stack, ptid, tls_addr, ctid)
```

关键标志位：

- **`CLONE_SETTLS`**（0x80000）：告诉内核"请把第4个参数（tls_addr）写入新线程的 TP 寄存器"
- **`CLONE_CHILD_SETTID`**：告诉内核"在新线程的地址空间中，向 ctid 指针处写入新线程的 TID"
- **`CLONE_CHILD_CLEARTID`**：告诉内核"当这个线程退出时，向 ctid 指针处写入 0，并对该地址做 futex_wake"

在 rustoswhu 的内核实现中，`CLONE_SETTLS` 的处理（`process.rs:187-193`）：

```rust
if flags.contains(CloneFlags::SETTLS) {
    trap_cx[TrapFrameArgs::TLS] = _tls as usize;
}
```

就是把 musl 准备好的 TLS 区域地址写入新线程的 trap frame。当新线程被调度执行时，`tp` 寄存器会被设为这个值，于是新线程就拥有了自己独立的 TLS 副本。

**这就是 TLS 与线程创建的核心联动**：用户态 libc 负责分配和初始化 TLS 内存，内核负责在上下文切换时把正确的 TP 值装入寄存器。

---

## 六、set_tid_address：线程退出的通知机制

`set_tid_address` 系统调用（编号 96）看似简单，实则是 pthread 线程生命周期管理的关键一环。

### 6.1 调用时机

musl libc 在两个地方调用 `set_tid_address`：

1. **主线程初始化时**（`__init_tls` 中）：
   ```c
   __syscall(SYS_set_tid_address, &__thread_list_lock);
   ```
   这个调用的主要目的是获取当前线程的 TID（返回值），同时注册 `clear_child_tid` 地址。

2. **通过 clone 的 `CLONE_CHILD_CLEARTID` 标志**——效果等同于 `set_tid_address`，但是在线程创建时就一步到位。

### 6.2 它到底做了什么？

Linux 内核中，`set_tid_address(tidptr)` 做两件事：
1. 将 `tidptr` 保存为当前线程的 `clear_child_tid`
2. 返回当前线程的 TID

### 6.3 clear_child_tid 的作用：pthread_join 的底层支撑

当一个线程退出时，内核检查它是否设置了 `clear_child_tid`。如果设置了：

```
1. 向 clear_child_tid 地址写入 0
2. 对该地址执行 futex(FUTEX_WAKE, 1)
```

这个机制是 `pthread_join` 的底层实现基础。当线程 A 调用 `pthread_join(thread_B)` 时：

```
线程 A:                              线程 B:
                                     (正在执行...)
pthread_join(B):
  读取 B->tid（非零，B 还活着）
  futex_wait(&B->tid, B的TID值)
  （阻塞等待...）
                                     线程 B 退出:
                                       内核: *clear_child_tid = 0
                                       内核: futex_wake(clear_child_tid, 1)
  （被唤醒，B->tid 现在是 0）
  return 0  // join 成功
```

`clear_child_tid` 指向的地址通常就是 `struct pthread` 中的 `tid` 字段。musl 在 `pthread_create` 时把 `ctid` 参数设为 `&new_thread->tid`，这样：

- `CLONE_CHILD_SETTID` 让内核在线程启动时向 `&tid` 写入 TID
- `CLONE_CHILD_CLEARTID` 让内核在线程退出时向 `&tid` 写入 0 并 futex_wake

整个流程无需用户态额外的锁或信号，纯粹靠内核的原子写入 + futex 唤醒完成线程退出通知。

在 rustoswhu 的实现中（`task/mod.rs:146-156`）：

```rust
if inner.tidaddress.clear_child_tid.is_some() {
    let addr = inner.tidaddress.clear_child_tid.unwrap();
    *safe_translated_refmut(..., addr as *mut i32).unwrap() = 0;  // 写 0
    let paddr = inner.memory_set.lock().page_table
        .translate(VirtAddr::from(addr)).unwrap().0;
    futex_wake(FutexKey::new(paddr, pid), 1);    // 线程共享 futex
    futex_wake(FutexKey::new(paddr, 0), 1);      // 进程共享 futex
}
```

---

## 七、完整生命周期：从 exec 到线程退出

把上面所有知识串起来，一个多线程程序的 TLS 生命周期如下：

```
1. execve("./program")
   ├── 内核解析 ELF，找到 PT_TLS 段
   ├── tp = PT_TLS 虚拟地址（初始值）
   └── 跳转到 _start

2. _start → __libc_start_main → __init_tls
   ├── 分配主线程的 TLS 区域（含 struct pthread）
   ├── 拷贝 PT_TLS 模板到新区域
   ├── tp = 新分配的 TLS 区域地址（覆盖内核设的值）
   ├── set_tid_address(&self->tid) → 注册 clear_child_tid
   └── 继续执行 main()

3. pthread_create(thread_func)
   ├── musl: mmap 新栈 + 新 TLS 区域
   ├── musl: 拷贝 PT_TLS 模板，初始化 struct pthread
   ├── clone(CLONE_SETTLS | CLONE_CHILD_SETTID | CLONE_CHILD_CLEARTID,
   │         new_stack, &parent_tid, new_tls, &new_thread->tid)
   │   ├── 内核: 新线程 tp = new_tls
   │   ├── 内核: *(&new_thread->tid) = new_tid  [CHILD_SETTID]
   │   └── 内核: 记录 clear_child_tid = &new_thread->tid  [CHILD_CLEARTID]
   └── 新线程开始执行 thread_func，tp 指向自己的 TLS

4. 新线程退出
   ├── 内核: *(clear_child_tid) = 0
   ├── 内核: futex_wake(clear_child_tid, 1)
   └── 等待中的 pthread_join 被唤醒

5. 主线程 pthread_join 返回
   └── 程序继续执行
```

---

## 八、对我们 rcore-lab 的启示

理解了 TLS 的完整机制后，对内核实现有以下关键认识：

1. **exec 时设置 TP 是必要的最小动作**。即使 musl 会覆盖它，内核仍需提供一个合法的初始值。如果 PT_TLS 段不存在（程序没有 TLS 变量），`tls_addr` 为 0，`tp` 就是 0——这对不用 TLS 的程序无影响，因为 musl 初始化时会自行设置。

2. **CLONE_SETTLS 是多线程的命脉**。没有它，新线程的 `tp` 会继承父线程的值，两个线程的 TLS 指向同一块内存，`errno` 等变量就会互相覆盖——这正是 TLS 要解决的问题。

3. **set_tid_address + futex 构成了 pthread_join 的底层机制**。如果 `clear_child_tid` 处理不正确（比如忘记写 0 或忘记 futex_wake），`pthread_join` 就会永远阻塞。

4. **TP 寄存器在内核态和用户态之间的保存/恢复至关重要**。每次 trap 进入内核时，`tp` 的值必须被保存到 trap frame 中；返回用户态时必须恢复。如果这一步有遗漏，用户态的 TLS 访问就会崩溃。
