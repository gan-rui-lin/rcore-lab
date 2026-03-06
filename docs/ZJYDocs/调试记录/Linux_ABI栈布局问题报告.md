# Linux ABI栈布局Bug调试报告

**Bug**: busybox在0x10ef4处持续发生LoadPageFault，argv[0]=NULL
**提交**: `ac9fc68` - fix: 修正Linux ABI栈布局，完全解决LoadPageFault
**日期**: 2026-02-16

---

## 问题演进时间线

### 第一阶段：OOM问题 (04ee655)

**现象**:
```
[TRACE] kernel:pid[2] sys_mmap
[TRACE] [syscall] pid=2 name=busybox num=222 args=[0x0,0x0,0x3,0x22,0xffffffffffffffff,0x0] ret=-22
sh: out of memory
```

**分析**:
- busybox调用`mmap(0, 0, PROT_READ|PROT_WRITE, MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`
- len=0导致内核返回EINVAL (-22)
- 根因：强制TCB初始化干扰了musl libc的malloc

**修复**:
- 移除process.rs中对无PT_TLS程序的强制TCB分配
- 添加完整auxv支持(12项)，包括AT_RANDOM

**结果**:
- OOM问题解决 ✅
- 但暴露了新的LoadPageFault问题 ❌

---

### 第二阶段：LoadPageFault问题 (ac9fc68)

**现象**:
```
[kernel] trap_handler: Exception(LoadPageFault) in application
  bad addr (stval) = 0x0
  bad instruction (sepc) = 0x10ef4
  Registers:
    a5 (x15) = 0x0  ← argv[0]为NULL!
```

## 调试过程详解

### 1. 初步分析：寄存器状态

查看完整寄存器状态：
```
sp (x2) = 0x168e20
a0 (x10) = 0x165204  ← argc应该在这里？
a1 (x11) = 0x168e78  ← argv应该在这里？
a2 (x12) = 0x168e98  ← envp应该在这里？
a5 (x15) = 0x0       ← argv[0] = NULL!
```

**关键观察**：根据RISC-V调用约定，main()的参数是：
- a0 = argc
- a1 = argv
- a2 = envp

但busybox读取argv[0]时得到了NULL。

### 2. 添加调试输出：查看实际栈内容

在process.rs中添加调试输出：
```rust
info!("[kernel] exec: sp={:#x}, argc={}, argv_base={:#x}, envp_base={:#x}",
    user_sp, argc, argv_base, envp_base);
info!("[kernel] exec: argv[0]={:#x}, argv[1]={:#x}",
    if argc > 0 { arg_addrs[0] } else { 0 },
    if argc > 1 { arg_addrs[1] } else { 0 });
info!("[kernel] exec: *sp (argc) = {}, *(argv_base) = {:#x}",
    *translated_ref(new_token, user_sp as *const usize),
    *translated_ref(new_token, argv_base as *const usize));
```

**输出结果**：
```
[ INFO] [kernel] exec: sp=0x168e70, argc=3, argv_base=0x168e80, envp_base=0x168ea0
[ INFO] [kernel] exec: argv[0]=0x168fca, argv[1]=0x168fc5
[ INFO] [kernel] exec: *sp (argc) = 3, *(argv_base) = 0x168fca
```

**关键发现**：
- 内核正确写入了argc=3
- 内核正确写入了argv数组：argv_base=0x168e80, argv[0]=0x168fca
- **但是**：a1寄存器=0x168e78，而argv_base=0x168e80！
- **差8字节** = 1个指针大小

### 3. 检查trap_cx的寄存器设置

查看process.rs中的寄存器初始化：
```rust
trap_cx.x[10] = argc;        // a0 = argc
trap_cx.x[11] = argv_base;   // a1 = argv数组地址
trap_cx.x[12] = envp_base;   // a2 = envp数组地址
```

**此时的理解**（错误）：
- a1应该指向argv数组
- argv数组内容：[argv[0], argv[1], ..., NULL]
- 程序通过`char *arg0 = ((char**)a1)[0]`获取argv[0]

### 4. 反汇编分析：busybox到底在做什么？

参考之前的LoadPageFault-Debug-Report.md：
```asm
10ee8:  0004b783    ld    a5,0(s1)      # a5 = argv[0]
10eec:  02d00713    li    a4,45
10ef0:  def1bc23    sd    a5,-520(gp)  # applet_name = a5
10ef4:  0007c683    lbu   a3,0(a5)     # a3 = *a5 ← PageFault!
```

**关键代码**：`ld a5, 0(s1)` - 从s1指向的地址加载argv[0]

这说明busybox认为s1寄存器指向的是argv数组的**起始地址**，并通过解引用获取argv[0]。

### 5. 查看实际内存布局

根据调试输出：
```
sp = 0x168e70
argc at sp = 0x168e70, value = 3
argv_base = 0x168e80  ← 我创建的argv数组地址
```

**我的错误实现的内存布局**：
```
0x168e70: [argc=3]
0x168e78: [???]  ← a1实际指向这里！
0x168e80: [argv数组] → [0x168fca][0x168fc5][0x168fb7][NULL]
                        ↑argv[0]  ↑argv[1]  ↑argv[2]
```

**问题**：
- 我将argv_base设置为0x168e80
- 但Linux ABI期望argv紧跟在argc后面！
- **应该是sp+8就是argv[0]，而不是指向argv数组的指针！**

### 6. 顿悟时刻：查阅Linux ABI规范

此时我意识到可能对Linux ABI栈布局有根本性误解。查阅System V ABI文档：

**标准Linux进程栈布局** (从低地址到高地址)：
```
position            content                     size (bytes) + comment
  ------------------------------------------------------------------------
  stack pointer ->  [ argc = number of args ]     8
                    [ argv[0] (pointer) ]         8   (program name)
                    [ argv[1] (pointer) ]         8
                    [ argv[..] (pointer) ]        8 * x
                    [ argv[n - 1] (pointer) ]     8
                    [ argv[n] (pointer) ]         8   (= NULL)

                    [ envp[0] (pointer) ]         8
                    [ envp[1] (pointer) ]         8
                    [ envp[..] (pointer) ]        8
                    [ envp[term] (pointer) ]      8   (= NULL)

                    [ auxv[0] (Elf64_auxv_t) ]    16
                    [ auxv[1] (Elf64_auxv_t) ]    16
                    [ auxv[..] (Elf64_auxv_t) ]   16
                    [ auxv[term] (Elf64_auxv_t) ] 16  (= AT_NULL vector)

                    [ padding ]                   0 - 16

                    [ argument ASCIIZ strings ]   >= 0
                    [ environment ASCIIZ str. ]   >= 0

  (0xffffffffffffffff) [ end marker ]                     8   (= NULL)
```

**关键理解**：
- **argv不是一个单独的数组！**
- **argv指针直接紧跟在argc后面写在栈上！**
- a1寄存器应该指向`sp+8`，也就是argv[0]的位置
- 程序通过`char *arg0 = *((char**)sp+1)`获取argv[0]

### 7. 对比我的错误实现

**我的错误实现**：
```rust
// 错误：创建了单独的argv数组
user_sp -= (arg_addrs.len() + 1) * word_size;
let argv_base = user_sp;  // argv数组的地址
for (i, addr) in arg_addrs.iter().enumerate() {
    *translated_refmut(new_token, (argv_base + i * word_size) as *mut usize) = *addr;
}
// 然后单独写argc
user_sp -= word_size;
*translated_refmut(new_token, user_sp as *mut usize) = argc;
```

**内存布局**（错误）：
```
sp → [argc]
     [gap]  ← a1指向这里，但这里什么都没有！
     [argv数组的起始] → [argv[0]][argv[1]][...]
```

**正确的实现**：
```rust
// 正确：argc后面直接跟argv指针
user_sp -= word_size;  // argc空间
user_sp -= (argc + 1) * word_size;  // argv指针空间
user_sp -= (envc + 1) * word_size;  // envp指针空间
user_sp &= !0xf;  // 对齐

let mut current_sp = user_sp;
*translated_refmut(new_token, current_sp as *mut usize) = argc;
current_sp += word_size;

// argv指针直接写在这里！
let argv_base = current_sp;  // argv_base = sp+8
for addr in arg_addrs.iter() {
    *translated_refmut(new_token, current_sp as *mut usize) = *addr;
    current_sp += word_size;
}
```

**内存布局**（正确）：
```
sp → [argc=3]
sp+8 → [argv[0]=0x168fca]  ← a1应该指向这里！
sp+16 → [argv[1]=0x168fc5]
sp+24 → [argv[2]=0x168fb7]
sp+32 → [NULL]
sp+40 → [envp[0]=...]
```

### 8. 验证修复

修复后的调试输出：
```
[ INFO] [kernel] exec: sp=0x168e60, argc=3, argv_base=0x168e68, envp_base=0x168e88
[ INFO] [kernel] exec: argv[0]=0x168fb7, argv[1]=0x168fb2
```

**检查**：
- sp = 0x168e60
- argv_base = 0x168e68 = sp + 8 ✅ 正确！
- envp_base = 0x168e88 = sp + 8 + 4*8 = sp + 40 ✅ 正确！(3个argv + 1个NULL)

**测试结果**：
```
=== /musl/basic_testcode.sh completed (status=0x0) ===
=== All tests completed ===
```

✅ **无LoadPageFault，busybox正常运行！**

---

## 根本原因总结

### 概念混淆

**我的错误理解**：
- 认为argv是一个独立的数组结构
- main函数接收的是：`int main(int argc, char **argv)`
- 所以argv是一个指向指针数组的指针
- 需要先分配argv数组，然后把数组地址传给a1

**正确的理解**：
- **argv不是一个独立分配的数组！**
- **argv IS the stack itself!**
- main函数的`char **argv`本质上就是`sp+8`
- 不需要"argv数组的地址"，sp+8就是argv！

### C语言角度的解释

在C程序看来：
```c
int main(int argc, char **argv) {
    char *arg0 = argv[0];  // 等价于 *(argv + 0)
    // argv本身就是一个指针，指向栈上argc后面的位置
}
```

在汇编/ABI角度：
```asm
# a1 = sp + 8（指向栈上argc后面的第一个指针）
# argv[0] 的地址 = a1 + 0 = sp + 8
# argv[0] 的值 = *(sp + 8)
```

### 内存布局对比

**错误实现**（多了一层间接）：
```
栈上:
sp → argc
     (gap)
     argv_array_start → argv[0]指针
                        argv[1]指针
                        NULL

寄存器: a1 = argv_array_start的地址
访问: argv[0] = *(*a1 + 0)  ← 需要两次解引用
```

**正确实现**（直接访问）：
```
栈上:
sp → argc
sp+8 → argv[0]指针
sp+16 → argv[1]指针
sp+24 → NULL

寄存器: a1 = sp + 8
访问: argv[0] = *(a1 + 0)  ← 只需一次解引用
```

---

## 为什么会犯这个错误？

### 1. C语言抽象的误导

在用户空间C代码中，我们习惯这样想：
```c
char *argv[] = {"prog", "arg1", NULL};
char **argv_ptr = argv;
```

这让人觉得argv是一个"数组的指针"，需要先有数组，再传指针。

### 2. 忽略了ABI的直接性

Linux ABI的设计非常直接：
- 栈就是数据结构本身
- 不需要额外的间接层
- sp+8就是argv，不是"指向argv的指针"

### 3. 之前的TCB覆盖问题的干扰

在修复TCB覆盖argc的问题时，我过度关注"不要覆盖argv数组"，反而没有质疑"为什么需要一个单独的argv数组"。

---

## 关键调试线索回顾

按重要性排序：

### 🔴 最关键线索：a1与argv_base的8字节差异
```
调试输出: argv_base=0x168e80
实际寄存器: a1=0x168e78
差值: 0x168e80 - 0x168e78 = 8字节 = 1个指针
```
→ 说明我多分配了一层结构

### 🔴 PageFault地址：stval=0x0
```
a5 (x15) = 0x0
lbu a3, 0(a5)  ← 访问NULL
```
→ argv[0]是NULL，说明argv数组位置不对

### 🟡 反汇编代码
```asm
ld a5, 0(s1)  # s1应该指向argv数组
```
→ busybox期望s1是argv的起始位置

### 🟡 Linux ABI文档
查阅System V ABI规范后发现标准布局是直接的
→ 没有独立的argv数组！

---

## 经验教训

### 1. 理解ABI要回到最底层

不要用高级语言的抽象来理解底层ABI。ABI规范非常直接，没有"额外的灵活性"。

### 2. 数值差异是关键线索

当看到a1=0x168e78但argv_base=0x168e80时，8字节的差异就应该立即引起警觉。

### 3. 调试输出要精确

添加详细的内存布局调试输出（sp, argc位置, argv_base, 实际内容）是找到问题的关键。

### 4. 参考文档很重要

当遇到"应该能工作但就是不工作"的情况时，回到官方ABI文档是必要的。

---

## 参考资料

1. **System V Application Binary Interface**
   - https://refspecs.linuxbase.org/elf/x86_64-abi-0.99.pdf
   - Section 3.4: Process Initialization

2. **RISC-V ELF psABI Specification**
   - https://github.com/riscv-non-isa/riscv-elf-psabi-doc
   - Program Loading and Dynamic Linking

3. **Linux内核文档**
   - https://www.kernel.org/doc/html/latest/
   - binfmt_elf.c 中的create_elf_tables()函数

4. **musl libc源码**
   - https://git.musl-libc.org/cgit/musl/
   - src/env/__libc_start_main.c

---

## 相关提交

- `3e53f6a` - fix: 修复TCB覆盖argc导致的LoadPageFault（被后续提交替代）
- `04ee655` - fix: 移除强制TCB初始化，修复busybox OOM问题
- `ac9fc68` - fix: 修正Linux ABI栈布局，完全解决LoadPageFault ✅
- `2c009d9` - docs: 更新CHANGELOG记录Linux ABI栈布局修复

---

**作者**: Claude Opus 4.6 (Anthropic AI)
**日期**: 2026-02-16
**版本**: 1.0
