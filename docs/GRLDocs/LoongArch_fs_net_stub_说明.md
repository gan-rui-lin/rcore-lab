# LoongArch64 构建中引入 fs_stub 和 net_stub 的动机说明

日期：2026/03/08

## 一、问题背景

当前 rcore-lab 的内核主体代码（`os/src`）在历史上是以 RISC-V 为单一架构开发的。文件系统与网络子系统（`fs/`、`net/`）以及部分系统调用逻辑（`syscall/`）默认与 RISC-V 平台的板级与驱动环境深度耦合。因此当我们开始让 `loongarch64` 参与编译时，会出现大量“模块未定义/类型缺失”的编译错误。

具体表现如下：

1. **模块 gating 问题**
   - `os/src/main.rs` 中 `fs`、`net` 等模块以前只对 `riscv64` 打开。
   - `task/`、`syscall/` 等核心模块却没有按架构加 `#[cfg]`，默认依赖 `crate::fs::*` 和 `crate::net::*`。
   - 当目标架构切换到 `loongarch64` 时，这些模块依赖的 `fs`、`net` 被编译条件排除，直接造成 unresolved import。

2. **子系统还未完成移植**
   - LoongArch64 目前阶段优先解决“能编译/能启动/能跑到基本路径”，而完整 fs/net 移植属于后续较大工作。
   - 真正让 `fs`、`net` 在 LoongArch64 上可用，意味着：
     - 页表与地址转换逻辑要支持 LoongArch64 内存布局。
     - 块设备驱动与网卡驱动要有 LA 平台对应实现与中断处理。
     - VFS / ext4 / easy-fs 的底层块设备 I/O 与缓存层要适配 LA 平台。
   - 这些不可能在“先保证 LA 编译通过”的阶段一次性完成。

3. **核心模块耦合较强**
   - `syscall/fs.rs` 依赖 `fs` 层导出的 `OpenFlags`、`Stat`、`PollEvents`、`DevNull`、`DevZero` 等类型。
   - `task/process.rs` 依赖 `fs` 层的 `Stdin/Stdout/File`，用于初始化进程的 fd 表。
   - `syscall/mod.rs` 中直接调用 `crate::net::syscall::*`。
   - 如果没有这些符号，即使 LoongArch64 的 trap/timer/mm 都已通过，编译仍无法继续。

因此，需要一个“临时但合法的替身”，让编译器满意、让内核逻辑能继续往前推进。

---

## 二、为什么使用 fs_stub 和 net_stub

### 2.1 目的：让编译器继续向前

`fs_stub.rs` 和 `net_stub.rs` 的本质是“**最小可编译替身**”。它们提供与真实模块相同的**类型名**与**函数签名**，让上层模块在 LoongArch64 构建时能顺利通过编译。它们并不提供真实功能，仅返回 `None` 或 `-ENOSYS`。

这样做的好处是：

- 不需要在大量代码中引入 `#[cfg(target_arch = "riscv64")]` 的条件分支。
- 不破坏现有 RISC-V 代码路径。
- 可以先把 LoongArch64 的 trap、timer、mm 等关键基础设施打通，再逐步替换 stub。

### 2.2 代价与风险

这种方式带来的代价是：

- LoongArch64 构建下，文件系统相关系统调用会返回 `-ENOSYS` 或空结果。
- 网络系统调用同样不可用。
- 运行时如果用户程序依赖 FS/NET，会失败。

但在“先能编译/能启动”的阶段，这种妥协是合理且常用的工程策略。

---

## 三、fs_stub.rs 与 net_stub.rs 的具体作用

### 3.1 fs_stub.rs

提供了 `syscall/fs.rs` 与 `task/process.rs` 所需的最小 API：

- `File` trait（只保留最基础接口）
- `Stdin`, `Stdout` 类型
- `OpenFlags`, `Stat`, `StatMode`, `PollEvents` 结构（以空实现/最小实现方式）
- `open_file()` 入口（返回 `None`）

这样 `syscall/fs.rs` 和 `task/process.rs` 中的引用不会报错。

### 3.2 net_stub.rs

`syscall/mod.rs` 中有大量网络 syscall 的分发，直接调用 `crate::net::syscall::sys_*`。

`net_stub.rs` 的作用是提供一个 `net::syscall` 模块，使所有网络 syscall 对应的函数存在，统一返回 `-ENOSYS`。

这保证编译通过，同时也明确表示“当前 LoongArch64 不支持网络功能”。

---

## 四、为什么不是在 syscall/task 中加入大量 cfg

另一种方案是：

- 在 `syscall` 中用 `#[cfg(target_arch = "riscv64")]` 包住所有 fs/net syscall 分支。
- 在 `task` 中用 `#[cfg]` 处理 fd 初始化。

这种做法会带来两个问题：

1. **侵入性极高**
   - 涉及文件非常多，改动面大，容易引入新 bug。
   - 需要对所有 syscall/流程逐一切分，对进度影响巨大。

2. **难以维护**
   - 每新增一个 syscall 或 FS/NET 依赖，都要添加 cfg 分支。
   - 最终代码可读性下降，架构差异散落在各模块中。

与之相比，**使用 stub 是更集中、更可控的临时策略**。

---

## 五、后续演进路线

fs_stub/net_stub 是临时措施，后续应逐步替换：

1. **先保证 LoongArch64 内核能运行基本 init**
   - 保证 trap/timer/mm/task 机制稳定。
   - 最小化用户态依赖（例如只运行最简单的测试）。

2. **逐步替换 stub**
   - 当 LoongArch64 的块设备驱动可用，逐步启用 `fs` 模块。
   - 当 LoongArch64 的网络驱动可用，逐步启用 `net` 模块。

3. **最终目标**
   - `fs_stub.rs`/`net_stub.rs` 不再使用
   - `fs`/`net` 正式成为 LoongArch64 的可用子系统

---

## 六、小结

总结来说，`fs_stub.rs` 与 `net_stub.rs` 的引入是为了：

- 解决 LoongArch64 编译时的“模块缺失”问题。
- 避免在 syscall/task 中加入大量 `cfg`，保持代码整洁。
- 在 LoongArch64 基础设施尚未完善时，提供一个“可编译的临时替身”。

它们并不是最终方案，而是一个**阶段性工程策略**，目的是确保移植工作可以持续推进，而不是在 fs/net 未完成时卡住编译。

如果你希望以后彻底移除 stub，就需要逐步补齐 LoongArch64 的文件系统与网络支持，再把模块 gating 恢复为真实实现。
