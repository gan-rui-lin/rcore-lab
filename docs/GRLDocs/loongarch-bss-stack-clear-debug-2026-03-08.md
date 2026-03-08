# LoongArch64 启动清 BSS 卡住问题调试记录（2026/03/08）

**日期**：2026/03/08

## 结论先行（罪魁祸首）
根因是 **LoongArch64 早期启动栈被 `clear_bss()` 清零覆盖**。启动代码先把 `sp` 指向 `.bss.stack` 的 `boot_stack_top`，随后在 `rust_main()` 调用 `clear_bss()` 清整段 `.bss`。因为 `.bss.stack` 在 `.bss` 段最前，导致清 BSS 时把正在使用的栈区域清零，栈被破坏，程序表现为在 `clear_bss()` 内部“卡住”，无法继续执行到 `console_init()`/`println!`。

这是一个典型的 **清 BSS 范围包含启动栈** 的问题。修复方法是 **将 `sbss` 放在 `.bss.stack` 之后**，让 `clear_bss()` 只清普通 `.bss`，而不清启动栈。

---

## 现象与复现
### 现象
在 LoongArch64 上启用 GDB 调试，进入 `rust_main()` 后，一步步执行到 `clear_bss()`，再单步或 `finish` 后就“卡住”，无法继续到 `console_init()`。

GDB 输出中表现为一直停在 core 的 `fill`/`spec_fill` 相关路径（没有源码也会显示 `No such file or directory`）。例如：
```
Breakpoint 1, os::arch::loongarch64::entry::clear_bss ()
... fill() ...
^C
Program received signal SIGINT, Interrupt.
0x9000000090000998 in core::slice::specialize::{impl#1}::spec_fill<u8> ...
```

### 复现步骤
1. 使用 LoongArch 调试运行：
   - `bash run-la.sh -t debug -d`
2. GDB 连接：
   - `gdb-loongarch64-unknown-linux-gnu -q -ex "file kernel-la" -ex "target remote :1234"`
3. 设置断点：
   - `b rust_main`
   - `c`
4. 单步/`finish`：
   - `n` 或 `finish`
5. 观察：无法走到 `console_init()`，疑似卡在清 BSS。

---

## 关键证据与分析
### 1. `clear_bss()` 卡住
`rust_main()` 中首先调用：
```
clear_bss();
```
该函数会对 `[sbss, ebss)` 范围做 `fill(0)`。

### 2. `nm` 确认 BSS 范围与栈位置
通过 `nm -n kernel-la | grep -E 'sbss|ebss|boot_stack'` 得到：
```
9000000090004000 B boot_stack_lower_bound
9000000090004000 D sbss
9000000090014000 B boot_stack_top
9000000090014000 B ebss
```
说明：
- `sbss` 和 `boot_stack_lower_bound` 是同一个地址。
- `ebss` 和 `boot_stack_top` 是同一个地址。
- 清 BSS 的范围正好覆盖 `.bss.stack` 这 64KB 的启动栈。

### 3. 启动流程与栈使用
在 `entry.S` 中：
- `sp` 被设置为 `boot_stack_top`。
- 随后跳转到 `rust_main()`。

也就是说：**`clear_bss()` 正在清理当前正在使用的栈内存**。

这解释了为什么执行 `clear_bss()` 后就卡住：
- 栈被清零后，函数返回地址/局部变量被破坏。
- 继续执行时出现不可预期的行为，表现为 GDB 里卡在 fill 循环。

---

## 修复思路与实现
### 目标
让 `clear_bss()` **不要清 `.bss.stack`**，避免破坏启动栈。

### 方案
调整 LoongArch64 的链接脚本：
- `.bss.stack` 仍然保留在 `.bss` 区域最前。
- 但是将 `sbss` 设在 `.bss.stack` 之后。
- 这样 `clear_bss()` 从 `sbss` 开始，只清普通 `.bss`/`.sbss`。

### 改动示意（核心逻辑）
```
.bss : {
    *(.bss.stack)
    sbss = .;
    *(.bss .bss.*)
    *(.sbss .sbss.*)
}
```

---

## 验证方式
1. 重新编译调试版本：
   - `make la MODE=debug`
2. 重新启动 GDB：
   - `bash run-la.sh -t debug -d`
   - `gdb-loongarch64-unknown-linux-gnu -q -ex "file kernel-la" -ex "target remote :1234"`
3. 断点确认：
   - `b rust_main`
   - `c`
   - `finish` 或 `n`
4. 期望现象：
   - `clear_bss()` 能顺利返回。
   - 能走到 `console_init()`，并输出 `println!` 的日志。

如果仍未到 `console_init()`，再进一步检查：
- 串口 MMIO 地址是否正确。
- 早期 CSR 设置是否影响地址映射。

---

## 经验总结
1. **清 BSS 必须避开当前栈**。早期启动栈通常放在 `.bss.stack`，如果 `sbss` 没有排除它，就会自毁。
2. **GDB 看起来像“卡在 fill”不一定是死循环**，更可能是栈被清空导致执行路径混乱。
3. `nm` 是非常直观的工具，用它查 `sbss/ebss/boot_stack` 立刻能确认清零范围是否合理。
4. 在多架构支持场景下，**链接脚本细节非常关键**，尤其是地址空间映射与段布局，任何一个疏忽都会导致早期启动阶段直接卡住。
