# LoongArch64 GDB 调试指南

**日期**: 2026/3/9

---

## 一、环境准备

```bash
# GDB 路径
GDB=/usr/local/gdb-15.1/bin/gdb-loongarch64-unknown-linux-gnu

# 内核 ELF（debug 模式有符号）
ELF=os/target/loongarch64-unknown-none/debug/os
```

## 二、启动调试会话

```bash
# 终端1: QEMU 带 GDB stub（-d 开启 -s -S，停在第一条指令等 GDB）
LOG=INFO bash run-la.sh -t debug -d

# 终端2: 连接
$GDB $ELF
(gdb) target remote :1234
```

## 三、QEMU 异常日志（最强调试手段）

**不需要 GDB**，直接让 QEMU 记录所有异常到文件：

```bash
qemu-system-loongarch64 -kernel kernel-la -m 2G -nographic -smp 1 \
    -drive file=sdcard-la.img,if=none,format=raw,id=x0 \
    -device virtio-blk-pci,drive=x0 -no-reboot \
    -device virtio-net-pci,netdev=net0 -netdev user,id=net0 \
    -rtc base=utc \
    -d int -D /tmp/qemu-int.log &
sleep 8 && pkill -f qemu-system-loongarch64
```

### 分析三板斧

```bash
# 1) 行数：百万行 = 死循环
wc -l /tmp/qemu-int.log

# 2) 异常类型统计：哪种异常最多 = 罪魁祸首
grep -o "exception: [0-9]*" /tmp/qemu-int.log | sort | uniq -c | sort -rn | head

# 3) 尾部：看最后卡在什么状态
tail -10 /tmp/qemu-int.log
```

### LoongArch 异常编号速查

| ecode | 名称 | 含义 | 常见原因 |
|-------|------|------|---------|
| 1 | PIL | Load 页无效 | TLB 有 entry 但 V=0 |
| 2 | PIS | Store 页无效 | TLB 有 entry 但 V=0 |
| 3 | PIF | Fetch 页无效 | 跳到未映射地址执行 |
| 4 | PME | 页修改异常 | D=0 时 store（LoongArch D 位软件管理） |
| 7 | PPI | 页特权违反 | 内核页被用户态访问 |
| 11 | TI | 定时器中断 | 正常，`ticlr` 清除 |
| 12 | SYS | syscall | 正常 |
| 13 | INE | 指令不存在 | 跳到垃圾地址 / 非法编码 |
| 16 | - | 向量指令禁用 | EUEN 未使能 LSX/LASX |
| 63 | TLBR | TLB 重填 | TLB 无 entry，触发 `tlb_fill` |

### 关键字段解读

```
loongarch_cpu_do_interrupt: PC 9000000090031000 ERA 9000000090031054 cause 2
ESTAT 0000000000020000 BADVA ffffffffffc4e000
```

- **PC**: 异常处理入口（= EENTRY 值）
- **ERA**: 触发异常的指令地址（= RISC-V 的 sepc）
- **cause**: 异常编号
- **ESTAT**: 状态寄存器，bits[21:16] = ecode
- **BADVA**: 出错的虚拟地址（= RISC-V 的 stval）

## 四、GDB 常用命令

### 4.1 查找 Rust 符号（name mangling）

```gdb
# 直接 "b task_entry" 会报 not defined，要搜完整路径：
info functions task_entry
info functions context_switch_pt
info functions run_tasks

# 用搜到的完整路径设断点：
b os::arch::loongarch64::trap::task_entry
b os::arch::loongarch64::kcontext::context_switch_pt

# naked 函数可以直接用 #[no_mangle] 名字：
b user_restore
```

### 4.2 断点

```gdb
# 软件断点
b os::arch::loongarch64::trap::task_entry

# 硬件断点（对 naked/asm 函数更可靠）
hbreak *0x90000000900209a0

# 条件断点
b trap_handler if $a0 == 4

# 查看/删除断点
info breakpoints
delete 1
```

### 4.3 执行控制

```gdb
c               # 继续执行
si              # 单步一条汇编指令
si 13           # 步进 13 条（跳过 context_switch 的 13 条 save）
n               # 源码级单步（跳过函数调用）
finish          # 执行到当前函数返回
```

### 4.4 寄存器

```gdb
# 常用寄存器
info registers pc ra sp
printf "pc=0x%lx sp=0x%lx ra=0x%lx\n", $pc, $sp, $ra
printf "a0=0x%lx a1=0x%lx a2=0x%lx\n", $a0, $a1, $a2

# 所有寄存器 + CSR（通过 QEMU monitor）
monitor info registers

# LoongArch 寄存器别名：
#   $r4-$r11 = $a0-$a7（参数/返回值）
#   $r12-$r20 = $t0-$t8（临时）
#   $r23-$r31 = $s0-$s8（callee-saved）
#   $r22 = $fp/$s9
#   $r3 = $sp, $r1 = $ra, $r2 = $tp
```

### 4.5 内存

```gdb
# 查看 KContext 结构体（13 个 8 字节字段）
x/13gx $a1

# KContext 布局：
#   offset 0:  ksp
#   offset 8:  ktp
#   offset 16-96: s9, s0-s8 (10 regs)
#   offset 96: kpc (= ra)

# 反汇编
x/10i $pc             # 当前位置
x/10i $ra             # RA 指向的代码

# 栈内容
x/20gx $sp

# 读取字符串
x/s 0x9000000090050000
```

## 五、实战调试模式

### 模式1: 追踪 context_switch_pt 到 task_entry

```gdb
b os::arch::loongarch64::kcontext::context_switch_pt
c

# 命中后检查参数
printf "from=%p to=%p token=%p\n", $a0, $a1, $a2
x/13gx $a1          # 看 to KContext

# 步进到 ret（13 save + 3 page_table + 13 load = 29 条）
si 29
printf "ra=0x%lx sp=0x%lx\n", $ra, $sp
x/3i $pc             # 应该是 ret

# 执行 ret，看跳到哪
si 1
printf "pc=0x%lx\n", $pc
x/10i $pc            # 应该是 task_entry 的代码
```

### 模式2: 检查 user_restore 入口

```gdb
b user_restore
c

# 命中后检查 trap context 地址
printf "a0 (trap_cx) = 0x%lx\n", $a0
printf "sp (kernel)  = 0x%lx\n", $sp
printf "ra (return)  = 0x%lx\n", $ra

# a0 应该是 trap context 的用户虚拟地址（TRAP_CONTEXT_BASE 附近）
```

### 模式3: 卡死时中断查看状态

```gdb
# 如果内核卡死（GDB 连接后 target 在运行）：
# Ctrl+C 中断
interrupt

# 查看卡在哪
printf "pc=0x%lx\n", $pc
x/5i $pc
bt                   # 回溯调用栈
info threads
```

### 模式4: batch 脚本（非交互式快速检查）

```bash
cat > /tmp/gdb.cmd << 'EOF'
set pagination off
set confirm off
target remote :1234
hbreak *0x90000000900209a0
c
printf "sp=0x%lx ra=0x%lx\n", $sp, $ra
x/13gx $a1
EOF

timeout 15 $GDB -batch -x /tmp/gdb.cmd $ELF
```

> **注意**: batch 模式 `c` 后如果断点没命中，后续命令会报
> "Cannot execute while target is running"。用 `hbreak` 比 `b` 更可靠。

## 六、经验总结

1. **QEMU `-d int` 日志是第一选择**，比 GDB 快 10 倍定位——看异常类型 + BADVA 即可
2. **死循环 = 百万行日志**，`wc -l` + `uniq -c` 秒判断
3. **GDB batch 有竞态**，断点没命中后续命令全废；交互式更稳
4. **CSR 通过 `monitor info registers` 查看**，GDB 本身不认 LoongArch CSR 名字
5. **Rust 符号有 mangling**，先 `info functions xxx` 搜再设断点
6. **naked 函数用 hbreak**，软件断点在 naked 函数上可能偏移到函数体内部
7. **异常 4 (PME) 循环 = D 位没处理**，这是 LoongArch 最常见的坑
8. **异常 16 循环 = EUEN 没开**，编译器优化会生成 LSX 指令
