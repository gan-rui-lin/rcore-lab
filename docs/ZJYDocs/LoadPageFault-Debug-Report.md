# LoadPageFault 调试报告

**Bug**: busybox 在 0x10ef4 处发生 LoadPageFault，尝试解引用地址 0x0
**提交**: `3e53f6a` - fix: 修复TCB覆盖argc导致的LoadPageFault
**日期**: 2026-02-15

---

## 问题现象

```
[kernel] trap_handler: Exception(LoadPageFault) in application
  bad addr (stval) = 0x0
  bad instruction (sepc) = 0x10ef4
  Registers:
    a5 (x15) = 0x0  ← argv[0] 为 NULL!
```

程序在尝试读取 `argv[0]` 指向的字符串第一个字符时崩溃。

---

## 调试过程

### 1. 反汇编分析

使用 `riscv64-unknown-elf-objdump` 反汇编 busybox：

```asm
10ee8:  0004b783    ld    a5,0(s1)      # a5 = argv[0]
10eec:  02d00713    li    a4,45
10ef0:  def1bc23    sd    a5,-520(gp)  # applet_name = a5
10ef4:  0007c683    lbu   a3,0(a5)     # a3 = *a5 ← PageFault!
```

**发现**: `argv[0]` 是 NULL 指针！

### 2. 栈布局检查

预期的用户栈布局：
```
High Address
+------------------+
| TCB (if needed)  |
+------------------+
| argc             |  ← sp should point here
+------------------+
| argv[0] pointer  |  ← should point to valid string
| argv[1] pointer  |
| NULL             |
+------------------+
| envp[0] pointer  |
| ...              |
+------------------+
| env strings      |
| arg strings      |
+------------------+
Low Address
```

### 3. 代码审查

检查 `process.rs` 中的 `exec()` 函数：

**问题代码** (行 295-329):
```rust
// 1. 写入 argc (line 295-296)
user_sp = (user_sp - word_size) & !0xf;
*translated_refmut(new_token, user_sp as *mut usize) = argc;

// 2. 创建 trap_cx，传入 user_sp (line 298-304)
let mut trap_cx = TrapContext::app_init_context(
    entry_point,
    user_sp,  // ← sp 指向 argc，正确！
    ...
);
trap_cx.x[10] = argc;
trap_cx.x[11] = argv_base;
trap_cx.x[12] = envp_base;

// 3. TCB 初始化，修改 user_sp (line 313-329)
let tcb_size = 16usize;
user_sp = (user_sp - tcb_size) & !0xf;  // ← BUG: 修改了 user_sp!
let tp_value = user_sp;

// 初始化 TCB
*translated_refmut(new_token, tp_value as *mut usize) = 0;
*translated_refmut(new_token, (tp_value + 8) as *mut usize) = tp_value;

// trap_cx.sp 仍然指向旧的 user_sp (argc 位置)
// 但是 TCB 数据已经覆盖了那个位置！
```

### 4. 根本原因

**执行顺序错误**:
1. ✅ 写入 argc 到地址 X
2. ✅ trap_cx.sp = X
3. ❌ 在地址 X 写入 TCB（覆盖了 argc！）
4. ❌ 程序启动时 sp 指向 X，但 X 处是 TCB 数据，`argc = 0`, `argv[0] = 0`

**实际内存布局（Bug）**:
```
High Address
+------------------+
| TCB (16 bytes)   |  ← 写在这里
|  dtv = 0         |  ← 覆盖了原来的 argc!
|  self = tcb_addr |  ← 覆盖了原来的 argv 指针数组!
+------------------+  ← sp 指向这里（错误！）
| envp[0]          |
| ...              |
```

**结果**: busybox 读取 `argc` 得到 0，读取 `argv[0]` 得到 0（NULL），尝试解引用时 PageFault。

---

## 解决方案

### 修复原理

在写入 argc 之前分配 TCB，确保内存布局正确：

```rust
// 1. 先分配 TCB (if needed)
let tp_value = if tls_area.is_none() {
    let tcb_size = 16usize;
    user_sp = (user_sp - tcb_size) & !0xf;
    let tcb_addr = user_sp;

    // 初始化 TCB
    *translated_refmut(new_token, tcb_addr as *mut usize) = 0;
    *translated_refmut(new_token, (tcb_addr + 8) as *mut usize) = tcb_addr;

    Some(tcb_addr)
} else {
    None
};

// 2. 然后写入 argc (在 TCB 下面)
let argc = arg_addrs.len();
user_sp = (user_sp - word_size) & !0xf;
*translated_refmut(new_token, user_sp as *mut usize) = argc;

// 3. 创建 trap_cx，使用正确的 sp
let mut trap_cx = TrapContext::app_init_context(
    entry_point,
    user_sp,  // 指向 argc（TCB 下面）
    ...
);

// 4. 设置 tp 指向 TCB
if let Some(tcb_addr) = tp_value {
    trap_cx.x[4] = tcb_addr;
}
```

### 正确的内存布局

```
High Address
+------------------+
| TCB (16 bytes)   |  ← tp 指向这里
|  dtv = 0         |
|  self = tcb_addr |
+------------------+
| argc             |  ← sp 指向这里（正确！）
+------------------+
| argv[0] pointer  |  ← 指向有效字符串
| argv[1] pointer  |
| NULL             |
+------------------+
| envp[0] pointer  |
| ...              |
+------------------+
| auxv entries     |
+------------------+
| env strings      |
| arg strings      |
+------------------+
Low Address
```

---

## 代码变更

**文件**: `os/src/task/process.rs`
**修改**: 22 insertions(+), 16 deletions(-)

### 关键改动

1. **TCB 分配提前** (新增，line 274-291):
   ```rust
   // Allocate TCB before argc/argv/envp if no PT_TLS
   let tp_value = if tls_area.is_none() {
       let tcb_size = 16usize;
       user_sp = (user_sp - tcb_size) & !0xf;
       let tcb_addr = user_sp;
       // Initialize TCB...
       Some(tcb_addr)
   } else {
       None
   };
   ```

2. **tp 寄存器设置简化** (line 330-335):
   ```rust
   if let Some(ref tls) = tls_area {
       trap_cx.x[4] = tls.tp_value;
   } else if let Some(tcb_addr) = tp_value {
       trap_cx.x[4] = tcb_addr;  // 使用之前分配的 TCB
   }
   ```

---

## 验证

### 编译测试
```bash
$ bash run.sh
构建成功!
```

### 预期结果

程序应该能正常启动，不再出现 LoadPageFault。`argv[0]` 将指向有效的程序名称字符串。

### 测试步骤

1. 清理旧进程：`pkill qemu-system-riscv64`
2. 运行测试：`bash run.sh`
3. 观察输出，确认：
   - ✅ 没有 LoadPageFault 异常
   - ✅ busybox 正常启动
   - ✅ 测试完成："=== All tests completed ==="

---

## 教训总结

### 1. 栈布局的重要性

用户栈的初始化必须严格按照ABI规范：
- 固定结构：TCB → argc → argv → envp → auxv → 字符串
- 不能中途修改 sp
- trap_cx 使用最终的 sp 值

### 2. 调试技巧

- **反汇编**: 确定崩溃指令的具体操作
- **寄存器分析**: 从错误信息推断数据来源
- **代码审查**: 追踪数据流，找出修改点
- **内存布局可视化**: 画图理解问题

### 3. 常见陷阱

**❌ 错误模式**: 先设置 sp，后修改栈内容
```rust
let sp = allocate_stack();
trap_cx.sp = sp;          // 设置 sp
modify_stack_at(sp);      // 修改 sp 处的内容 ← 错误！
```

**✅ 正确模式**: 完成所有栈初始化后再设置 sp
```rust
let mut sp = allocate_stack();
initialize_stack(&mut sp);  // 所有初始化
trap_cx.sp = sp;           // 最后设置 sp
```

### 4. 代码审查要点

检查 sp 设置时，确认：
- [ ] 所有栈数据已经写入
- [ ] sp 指向正确的位置（argc）
- [ ] 之后没有代码修改 sp 或其指向的内存
- [ ] tp 等其他寄存器不影响栈布局

---

## 相关资源

### 参考文档
- [RISC-V psABI Specification](https://github.com/riscv-non-isa/riscv-elf-psabi-doc)
- [System V ABI](https://refspecs.linuxfoundation.org/elf/gabi4+/contents.html)
- [musl libc startup code](https://git.musl-libc.org/cgit/musl/tree/crt)

### 相关提交
- `e47e5c3` - fix: 为没有PT_TLS的程序初始化最小TCB
- `3e53f6a` - fix: 修复TCB覆盖argc导致的LoadPageFault

### 调试工具
- `riscv64-unknown-elf-objdump -d` - 反汇编
- `riscv64-unknown-elf-gdb` - GDB调试
- QEMU monitor 命令
- Kernel log 分析

---

**作者**: rCore开发团队
**审查**: Claude Opus 4.6
**版本**: 1.0
**最后更新**: 2026-02-15
