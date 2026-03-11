# LoongArch SignalAction 结构体布局不匹配修复

**日期**: 2026/3/11

**分类**: 调试记录 / 多架构适配

**关键词**: LoongArch, sigaction, SA_RESTORER, 信号处理, ABI 兼容性

---

## 一、问题现象

在首次运行 LoongArch 版本的 basic-musl 测试套件时，所有测试用例虽然功能上正常通过，但每个测试执行前后都会产生一条 `[ERROR]` 级别日志：

```
[ERROR] [signal] pid=4 signum=17 invalid restorer=0xfffffffc7fffffff, using trampoline
```

整个测试流程（共 30 个 basic 测试）累计产生了 **31 条**此类错误日志。`signum=17` 对应 `SIGCHLD` 信号——每当子进程退出时，父进程（busybox shell，pid=4）都会收到 SIGCHLD，并触发信号处理逻辑。由于内核检测到 `restorer` 地址 `0xfffffffc7fffffff` 超出用户态地址空间上限（`USER_ADDR_MAX`），判定其无效，转而使用内核预埋的 sigreturn trampoline 作为兜底。

功能上没有 crash，但这暴露了一个严重的 ABI 兼容性问题——**内核解析用户态传入的 `sigaction` 结构体时，字段偏移与 LoongArch musl libc 的实际布局不一致**。

## 二、罪魁祸首

**根因：`SignalAction` 结构体在 LoongArch 上多了一个不应存在的 `restorer` 字段，导致 `mask` 被错误解读为 `restorer`。**

具体来说，内核中的 `SignalAction`（`os/src/task/action.rs`）定义为：

```rust
#[repr(C, align(16))]
pub struct SignalAction {
    pub handler: usize,    // offset 0
    pub flags: usize,      // offset 8
    pub restorer: usize,   // offset 16
    pub mask: SignalFlags,  // offset 24
}
```

这个布局对 RISC-V 是正确的，但对 LoongArch 是**错误的**。

## 三、背景知识：Linux `sigaction` 的架构差异

### 3.1 `SA_RESTORER` 机制

在 Linux 信号处理框架中，当用户态信号 handler 执行完毕（`ret` 指令返回）时，需要有一段代码来发起 `rt_sigreturn` 系统调用，将执行流恢复到被信号中断前的位置。这段代码的来源有两种：

1. **`sa_restorer`**（用户态提供）：libc 在调用 `sigaction()` 时，在 `sa_restorer` 字段填入一段预编译的 sigreturn 桩代码的地址。内核将这个地址设为 handler 的返回地址（`RA` 寄存器）。handler `ret` 后跳到这里，执行 `ecall`/`syscall` 发起 `rt_sigreturn`。

2. **内核 trampoline**（内核提供）：如果 `sa_restorer` 为 0 或无效，内核自己在用户地址空间映射一个 trampoline 页，里面包含同样的 sigreturn 指令序列，然后把 `RA` 指向它。

### 3.2 架构间的关键差异

并非所有架构都支持 `SA_RESTORER`。在 Linux 内核源码中：

- **RISC-V**：定义了 `SA_RESTORER = 0x04000000`，`struct kernel_sigaction` 包含 `sa_restorer` 字段。musl libc 的 RISC-V 端口也会在 `sigaction()` 中填充 `sa_restorer`。

- **LoongArch**：使用 `asm-generic/signal.h` 的通用定义，**不定义 `SA_RESTORER`**。因此 `struct kernel_sigaction` 中 **没有 `sa_restorer` 字段**。信号返回完全依赖内核在用户栈或 VDSO 页上放置的 sigreturn trampoline。

这意味着两种架构的 `sigaction` 结构体 ABI 布局不同：

| 字段偏移 | RISC-V | LoongArch |
|---------|--------|-----------|
| offset 0 | `sa_handler` (8B) | `sa_handler` (8B) |
| offset 8 | `sa_flags` (8B) | `sa_flags` (8B) |
| offset 16 | **`sa_restorer` (8B)** | **`sa_mask` (8B)** |
| offset 24 | `sa_mask` (8B) | *(结构体结束)* |

### 3.3 misalignment 的后果

当 LoongArch 的 musl libc 调用 `sigaction(SIGCHLD, &act, NULL)` 时，它按照 LoongArch ABI 写入用户态内存：

```
offset 0:  handler 地址
offset 8:  flags 值
offset 16: mask 值（例如 0xfffffffc7fffffff）
```

但 rcore-lab 内核按 RISC-V 布局读取：

```
offset 0:  handler → 正确
offset 8:  flags   → 正确
offset 16: restorer → 读到了 mask 的值！(0xfffffffc7fffffff)
offset 24: mask    → 读到了越界的垃圾数据或零
```

这就是为什么日志显示 `restorer=0xfffffffc7fffffff`——它实际上是 signal mask 的值。

## 四、分析过程

### 4.1 从日志定位问题

首先注意到异常值 `0xfffffffc7fffffff` 的特征：

```
0xFFFFFFFC7FFFFFFF
= 1111_1111_1111_1111_1111_1111_1111_1100_0111_1111_1111_1111_1111_1111_1111_1111
```

清零的 bit 位为 31、32、33。如果将其解释为 signal mask（bit N 对应 signal N+1），清零的信号为 32、33、34——这恰好是 RT 信号范围的起始区域，musl 内部用于线程取消（`SIGCANCEL`、`SIGTIMER` 等）。这强烈暗示这个值**就是一个 signal mask**，而非地址。

### 4.2 验证布局假设

确认 Linux 内核源码中 LoongArch 的 `sigaction` 定义：

- `arch/loongarch/include/uapi/asm/signal.h` 继承自 `asm-generic/signal.h`
- `include/uapi/asm-generic/signal-defs.h` 中 `SA_RESTORER` 仅在架构定义了该宏时才生效
- LoongArch 没有定义 `SA_RESTORER`，因此 `struct kernel_sigaction` 中无 `sa_restorer` 字段

与 RISC-V 对比，RISC-V 在 `arch/riscv/include/uapi/asm/signal.h` 中明确定义了 `#define SA_RESTORER 0x04000000`。

### 4.3 确认影响范围

虽然只有 SIGCHLD 触发了 ERROR 日志（因为只对 SIGCHLD 和 sig33 添加了详细日志），但实际上**所有通过 `sigaction()` 注册的信号 handler** 都受此 ABI 不匹配的影响：

- `mask` 字段被错误地读为 `restorer`，导致实际存储的 mask 是垃圾值
- 信号屏蔽行为不正确（不过 basic 测试没有覆盖复杂的信号屏蔽场景，所以没有暴露功能错误）

## 五、修复方案

### 5.1 结构体条件编译（`os/src/task/action.rs`）

将 `SignalAction` 按架构拆分为两个版本：

```rust
// RISC-V 版本：包含 restorer 字段
#[cfg(not(target_arch = "loongarch64"))]
#[repr(C, align(16))]
pub struct SignalAction {
    pub handler: usize,
    pub flags: usize,
    pub restorer: usize,
    pub mask: SignalFlags,
}

// LoongArch 版本：无 restorer 字段，mask 紧跟 flags
#[cfg(target_arch = "loongarch64")]
#[repr(C, align(16))]
pub struct SignalAction {
    pub handler: usize,
    pub flags: usize,
    pub mask: SignalFlags,
}
```

同时提供统一的 `restorer()` 方法，在 LoongArch 上始终返回 0：

```rust
impl SignalAction {
    pub fn restorer(&self) -> usize {
        #[cfg(not(target_arch = "loongarch64"))]
        { self.restorer }
        #[cfg(target_arch = "loongarch64")]
        { 0 }
    }
}
```

### 5.2 信号分发逻辑简化（`os/src/task/mod.rs`）

LoongArch 没有 `sa_restorer`，信号返回**始终**通过内核 trampoline。原来的逻辑是先检查 restorer 有效性、无效时 fallback 到 trampoline，修复后简化为：

```rust
// LoongArch: no SA_RESTORER, always use kernel trampoline
#[cfg(target_arch = "loongarch64")]
{
    if let Some(res) = task_inner.res.as_ref() {
        let tramp_base = res.ustack_base().saturating_sub(PAGE_SIZE);
        let tramp_offset = arch::sigtrx::sigreturn_trampoline_offset();
        trap_cx[TrapFrameArgs::RA] = tramp_base + tramp_offset;
    }
}

// RISC-V: use sa_restorer if valid
#[cfg(not(target_arch = "loongarch64"))]
if action.restorer != 0 {
    if action.restorer < USER_ADDR_MAX {
        trap_cx[TrapFrameArgs::RA] = action.restorer;
    }
}
```

### 5.3 日志和 syscall 层适配（`os/src/syscall/process.rs`）

将所有 `action.restorer` 直接字段访问改为 `action.restorer()` 方法调用，确保跨架构编译通过。

## 六、修复验证

修复后重新运行 basic-musl 测试：

```
$ LOG=ERROR bash run-la.sh -t debug > debug02.log
$ grep -c "invalid restorer" debug02.log
0
$ grep "\[ERROR\]" debug02.log
（无输出）
```

- `invalid restorer` 错误：**0 条**（修复前 31 条）
- `[ERROR]` 日志：**0 条**
- 全部 30 个 basic 测试正常通过，输出与修复前一致

同时确认 RISC-V 编译不受影响（`restorer` 字段在 `#[cfg(not(loongarch64))]` 下仍然存在）。

## 七、经验总结

1. **多架构移植时，ABI 结构体布局必须逐一核对**。即使是同一个 POSIX 概念（如 `sigaction`），不同架构的内核 ABI 可能有字段增减。`sa_restorer` 就是一个典型的"大多数架构有，但部分架构没有"的字段。

2. **看似无害的 ERROR 日志可能隐藏严重的 ABI 问题**。本次 `restorer` 的错误值恰好触发了 trampoline 兜底逻辑，没有导致 crash，但 `mask` 字段的值实际上也是错的——只是 basic 测试没有覆盖到复杂信号屏蔽场景。如果后续运行涉及 `sigprocmask` 或自定义信号 handler 的测试，就会出现更难排查的问题。

3. **异常值的数值特征是重要线索**。`0xfffffffc7fffffff` 作为地址显然不合理，但作为 signal mask（清零了 bit 31-33，对应 musl 内部使用的 RT 信号）则完全合理。这种"换个角度解读数值"的思路在结构体 misalignment 类 bug 中非常有效。

4. **条件编译 + 统一方法接口**是处理架构差异的良好模式。`restorer()` 方法让调用方无需关心底层结构体差异，同时 `#[repr(C)]` 保证了与用户态 ABI 的字节级匹配。
