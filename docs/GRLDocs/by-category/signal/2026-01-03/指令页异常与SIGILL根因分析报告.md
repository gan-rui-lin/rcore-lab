# 指令页异常与 SIGILL 根因分析报告

## 一、问题背景与现象概述

在 musl/basic 测试流程中，最初出现两类致命异常：

1. **IllegalInstruction（SIGILL / signum 4）**：日志中可见用户态执行到 `sepc` 时，内存中的指令字节全为 0，但 ELF 文件对应位置的字节正确。
2. **InstructionPageFault（sepc=0/stval=0）**：在 `clone` 测试中，子进程进入用户态后出现取指异常，`sepc=0`、`stval=0`，对应 PTE 均为 unmapped。

这两个问题会导致测试流程中断，因此先采用临时权宜之计保证可运行，再回头定位根因并移除权宜修复。

## 二、阶段性权宜之计及其动机

当时的 SIGILL 表现为：
- `sepc` 处内存指令字节为 0；
- ELF 文件对应地址的字节正常；
- 现象呈现“偶现”。

这说明 **装载时正确、运行时被覆盖或失效**。在根因不明且测试阻塞的情况下，采用临时修复：

- 在 `IllegalInstruction` Trap 中判定 `sepc` 内存字节全 0 且文件字节有效；
- 直接从 ELF 中回填该页，继续执行。

该策略仅为临时自愈，并非根因修复。

## 三、关键调试过程（含关键命令）

### 1. 定位异常点的日志核验

为确认异常是否仍存在，优先从日志中定位关键异常字符串：

```bash
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all66.log
rg "IllegalInstruction|StorePageFault|InstructionPageFault" all66.log
```

确认日志中存在 `IllegalInstruction` 与 `StorePageFault`。

### 2. 反汇编定位 `sepc` 对应指令

针对 `StorePageFault` 的 `sepc=0x105364`，使用 objdump 定位代码：

```bash
llvm-objdump -d --no-show-raw-insn \
  --start-address=0x105300 --stop-address=0x1053a0 \
  /home/grl/codeRepo/rcore-lab/busybox/musl/busybox
```

定位到 `__syscall_ret` 中：
- `0x105364: sw s0, 0(a0)`
- `a0` 指向 `errno` 地址。

进一步反汇编 `__errno_location`：

```bash
llvm-objdump -d --no-show-raw-insn \
  --start-address=0x104fe0 --stop-address=0x105080 \
  /home/grl/codeRepo/rcore-lab/busybox/musl/busybox
```

确认 `__errno_location` 通过 `tp-0xa4` 计算 `errno` 指针。

### 3. 检查 Trap 保存/恢复机制

查看 trap entry/exit 汇编：

```bash
nl -ba os/src/trap/trap.S | sed -n '1,140p'
```

关键发现：**原实现不保存/恢复 `tp`（x4）**。
这会导致用户态 `tp` 在系统调用或中断后被内核覆盖，导致 `errno` 指针计算出错。

### 4. 验证 fork/clone 子进程上下文是否复制

出现 `InstructionPageFault (sepc=0)` 的日志片段：
- [all69.log](../../all69.log#L1137-L1156)

检查 `sys_fork`：

```bash
nl -ba os/src/syscall/process.rs | sed -n '148,185p'
```

发现 fork 逻辑只修改 `a0`，没有复制父进程 TrapContext。

此外，日志中 syscall 220 被标识为 `clone`，其参数包含 child stack (`a1`)：
```
[syscall] pid=7 name=ld-linux-riscv64-lp64d.so.1 num=220 args=[0x11,0x400b0470,...]
```
但子进程没有使用 `a1` 设置 `sp`，导致子进程返回用户态时 `sepc` 与栈上下文异常，直接从 0 地址取指。

### 5. 修复后验证

关键修复后，重新运行测试：

```bash
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all70.log
rg "InstructionPageFault|StorePageFault|IllegalInstruction" all70.log
```

并确认 clone 测试正常

## 四、根因分析与最终修复

### 1. SIGILL 根因（为什么不再需要权宜之计）

根因：**Trap 入口未保存/恢复 `tp`（x4），破坏用户态 TLS**。

- musl 的 `errno` 通过 `tp-0xa4` 访问。
- 当 `tp` 被内核覆盖后，`errno` 地址指向非法或未映射区域。
- 写入 `errno` 时触发 `StorePageFault`，进而可能引发指令页被破坏或间接污染，出现 `SIGILL`。

修复：在 trap 保存/恢复流程中加入 `tp` 寄存器：

- 保存 `x4` 于 `TrapContext`
- 恢复 `x4` 回用户态

修复位置：
- [os/src/trap/trap.S](../../os/src/trap/trap.S#L14-L100)

修复后，`tp` 在系统调用与中断之间保持一致，`errno` 访问恢复正常，**SIGILL 不再出现，因此无需权宜之计**。

### 2. InstructionPageFault（sepc=0）的根因

根因：**clone/fork 子进程 TrapContext 未复制 + child stack 未设置**。

具体表现：
- syscall 220 实际为 `clone`。
- `clone` 传入了子栈地址 `a1`，但子进程没有使用。
- `sys_fork` 未复制父 `TrapContext`，导致子进程 `sepc` 未初始化，最终从 `0x0` 取指，触发 `InstructionPageFault`。

修复：
1. 在 `sys_fork` 中复制父 TrapContext 到子进程。
2. 若 `a1`（child stack）非 0，则使用它作为子进程 `sp`。

修复位置：
- [os/src/syscall/process.rs](../../os/src/syscall/process.rs#L148-L167)

修复后，clone 测试正常结束，日志显示：
- [all70.log](../../all70.log#L1136-L1144)

## 五、结论与后续建议

1. **根因已定位并修复**：
   - `tp` 未保存 → SIGILL/StorePageFault。
   - `clone` 子栈未设置 + trap context 未继承 → InstructionPageFault。

2. **权宜之计已移除**，系统可以稳定通过测试。

3. **建议保留的调试手段**：
   - `rg "IllegalInstruction|InstructionPageFault|StorePageFault"` 作为日志快速体检。
   - `llvm-objdump` 定位 `sepc` 对应指令。
   - `nl -ba` 结合行号定位关键代码路径。
