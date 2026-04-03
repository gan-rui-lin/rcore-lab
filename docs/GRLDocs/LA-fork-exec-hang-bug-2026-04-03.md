# LoongArch Fork+Exec Hang Bug 调试记录

**日期**: 2026/4/3  
**问题**: LoongArch 上在 fork 的子进程中调用 execve 系统调用会卡住  
**根本原因**: Fork 后子进程传递 execve 参数时触发内存访问问题

## 最终诊断

通过详细的逐步调试，我们发现：

### 实验1：基本 syscall 可以工作
```rust
// 在 fork 后的子进程中：
chdir("/\0");      // ✅ 成功
write(1, msg);     // ✅ 成功  
getpid();          // ✅ 成功
```
**结论**：syscall 机制本身正常工作

### 实验2：syscall 指令本身没问题
```rust
// 裸 syscall，不传递任何参数
unsafe {
    core::arch::asm!(
        "syscall 0",
        inlateout("$a0") 0usize => test_ret,
        in("$a1") 0usize,
        in("$a2") 0usize,
        in("$a7") 221usize, // SYSCALL_EXEC
    );
}
```
**结果**：✅ 成功返回（返回 EFAULT 错误）  
**结论**：syscall 指令和 EXEC 系统调用号都没问题

### 实验3：传递参数时卡住
```rust
execve("/musl/busybox", &argv, &envp);
```
**结果**：❌ 永久卡住在 `syscall 0` 指令上  
**结论**：**问题出在访问参数指针时**

### 实验4：syscall 之前可以访问参数
```rust
let path_str = unsafe { core::str::from_utf8_unchecked(&busybox) };  // ✅ 成功
let pid = getpid();  // ✅ 成功
execve(path_str, &argv, &envp);  // ❌ 卡住
```
**结论**：参数在用户态可以访问，但在 syscall 时传递给内核会出问题

## 根本原因分析

**最可能的原因**：LoongArch fork 实现在复制子进程地址空间时有 bug：

1. **栈或堆内存映射不完整**
   - Fork 可能没有正确复制所有页表项
   - 某些页面的权限位可能不正确（如 User 位缺失）
   - 当 `syscall 0` 指令需要访问这些参数指针时，硬件/QEMU 触发了内部错误

2. **TLB 或缓存一致性问题**
   - Fork 后子进程的 TLB 可能没有正确刷新
   - 访问参数时可能命中了错误的 TLB 条目
   - LoongArch 的 TLB 管理可能需要特殊处理

3. **CSR 寄存器配置问题**
   - LoongArch 的页表基址寄存器（PGDL/PGDH）可能在 fork 后没有正确设置
   - 访问用户态内存时查页表失败，导致卡死

## 为什么其他 syscall 不受影响？

- `chdir`、`write`、`getpid` 等 syscall 传递的参数较少或较简单
- `execve` 需要传递复杂的指针数组（argv、envp），可能触发了更深层次的内存访问
- 可能某些特定地址范围的访问会触发 bug

## 对比 RISC-V

RISC-V 架构上相同的代码工作正常，说明：
- 问题是 LoongArch 特有的
- 可能是架构实现的差异（TLB 管理、页表格式等）
- 也可能是 QEMU LoongArch 模拟器的 bug

## 临时解决方案

在 LoongArch 上跳过所有需要 fork+exec 的操作：

```rust
#[cfg(target_arch = "loongarch64")]
{
    // WORKAROUND: Skip mkdir due to fork+exec hang bug
    force_link("/code/lmbench_src/bin/build/lmbench_all", "/musl/lmbench_all");
}

#[cfg(target_arch = "riscv64")]
{
    let _ = run_busybox_mkdir_p("/musl", busybox_path, "/code/lmbench_src/bin/build");
    force_link("/code/lmbench_src/bin/build/lmbench_all", "/musl/lmbench_all");
}
```

这样可以：
- 避免卡死问题
- 允许 musl/glibc 测试正常运行（init 进程可以直接 exec）
- 但无法测试 fork+exec 相关功能（如 shell、system()）

## 修复方向

### 1. 检查 LoongArch fork 实现

文件：`os/src/task/mod.rs`，`ProcessControlBlock::fork()`

需要检查：
- 页表复制是否完整（包括所有用户页）
- 页表项的权限位是否正确复制
- TLB 是否正确刷新
- 栈页面是否正确映射

### 2. 检查 LoongArch 上下文切换

文件：`arch/src/loongarch64/trap.rs`，`arch/src/loongarch64/entry.S`

需要检查：
- PGDL/PGDH 寄存器是否在切换时更新
- 用户栈指针（$sp）是否正确恢复
- PRMD/ERA 等 CSR 是否正确设置

### 3. 对比 RISC-V 实现

- 对比 `os/src/task/mod.rs` 中 fork 的实现
- 查找 RISC-V 特有的初始化代码
- 移植到 LoongArch

### 4. 添加详细的 TLB/页表日志

在 fork 和 exec 时打印：
- 页表基址（PGDL/PGDH）
- 各个用户页的映射情况
- TLB 刷新操作

## 诊断日志关键片段

```
=== rCore initcode ===
[LA] Testing fork+exec with musl...
[child] after fork               # fork 成功
[child] after chdir              # chdir syscall 成功
[child] after cstring            # 内存访问成功
[child] calling execve...
[child] testing raw syscall...   # 空参数 syscall 成功
[child] raw syscall returned     # 返回了！
# 但传递真实参数时永久卡住
```

## 影响范围

这个 bug 影响所有需要 fork+exec 的场景：
- ❌ Shell 脚本执行
- ❌ busybox 的任何子命令调用
- ❌ system() 或 popen()
- ❌ 进程管理工具
- ❌ LTP 测试套件
- ✅ init 进程直接 exec（不经过 fork）

**优先级：高** - 这是一个严重的架构特定 bug

## 参考信息

- 内核构建：`make la`
- 测试命令：`bash run-la.sh -t all`
- QEMU 版本：qemu-system-loongarch64
- 架构：LoongArch 64-bit
- 对比架构：RISC-V 64-bit（正常工作）

## 相关文件

**用户态**：
- `user/src/bin/initcode.rs`: 测试入口
- `user/src/syscall.rs`: syscall/syscall6 实现

**内核**：
- `os/src/task/mod.rs`: fork 实现
- `os/src/syscall/process.rs`: sys_exec 实现
- `arch/src/loongarch64/trap.rs`: trap handler
- `arch/src/loongarch64/trap.S`: 汇编 trap 入口
- `arch/src/loongarch64/entry.S`: 上下文切换
- `arch/src/loongarch64/context.rs`: TrapFrame 定义
