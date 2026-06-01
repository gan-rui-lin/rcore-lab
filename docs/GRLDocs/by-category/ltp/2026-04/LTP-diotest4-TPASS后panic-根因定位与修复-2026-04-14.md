# LTP `diotest4` 失败并触发内核 panic 根因定位与修复（2026-04-14）

## 1. 现象

执行：

```bash
SINGLE_TEST="/glibc/ltp/testcases/bin/diotest4" bash run.sh
```

出现两段异常：

1. 用例中途失败：

- `diotest4    3  TFAIL  :  diotest4.c:267: can't write to file -1`

2. 后续内核崩溃：

- `[kernel] Panicked at arch/src/riscv64/trap/mod.rs:74`
- trap 关键信息显示 `LoadPageFault`，`stval=0xffffffc0c0000000`

## 2. 关键证据

## 2.1 `case 3` 的 `TFAIL` 直接对应 `sys_write -> -EFAULT`

在 `diotest4` 场景下，`sys_write` 对用户缓冲区仅做了
`translated_byte_buffer_checked(..., writable=false)` 校验；
对于 lazy 页（尚未触发缺页分配）会直接返回 `None`，从而回 `EFAULT`。

这会导致 `diotest4` 第 3 项报错。

## 2.2 panic 地址指向 `MEMORY_END` 边界

调试日志中，ext4 写回回调 `dev_bwrite` 的 `buf` 按页递增，最终到：

- `buf=0xffffffc0c0000000`

并在该地址触发 `LoadPageFault`。

`0xffffffc0c0000000` 正好是 RISC-V 直映窗口中 `PA=0xC0000000`（`MEMORY_END`）对应地址，
说明内核路径拿到了“越界物理页映射”并在 memcpy 读取时崩溃。

## 3. 根因分析

本次问题由两类缺陷叠加触发：

1. **功能性缺陷（直接导致 TFAIL）**  
   `sys_write` 未对“用户读缓冲”做 lazy fault-in，导致合法 lazy 页被误判 `EFAULT`。

2. **健壮性缺陷（导致 panic 放大）**  
   页表/翻译路径缺少“用户 PPN 上界”校验；当异常 PTE 出现时，
   会继续被解释为可访问地址，最终在内核 memcpy 触发 page fault。

另外，`handle_demand_fault()` 之前存在“`map_one()` 失败时仍可能返回成功”的窗口，
会放大异常映射的后续影响。

## 4. 修复方案

## 4.1 `sys_write` 增加 readable fault-in 路径

文件：`os/src/syscall/fs.rs`

- 新增 `try_resolve_user_readable()`
- 新增 `translated_user_read_buffer()`
- `sys_write` 改用 `translated_user_read_buffer()` 取用户源缓冲

效果：lazy 页在写系统调用里会先按读权限补页，不再误报 `EFAULT`。

## 4.2 `handle_demand_fault` 增加“补页成功校验”

文件：`os/src/mm/memory_set.rs`

- `map_one()` 后立即检查 `page_table.translate(fault_vpn).is_valid()`
- 若失败直接返回 `false`

效果：避免“补页失败却继续往后走”。

## 4.3 页表层增加用户 `PPN` 上界防护（RISC-V + LoongArch）

文件：

- `arch/src/riscv64/mm/page_table.rs`
- `arch/src/loongarch64/page_table.rs`

改动：

1. `PageTable::map()`：若 `flags` 含 `U`，则要求 `ppn < PhysAddr::from(MEMORY_END).floor()`
2. `translated_byte_buffer_checked()`：用户页翻译时同样检查 `ppn` 上界

效果：即使出现异常 PTE，也会在边界处被拒绝，不再将越界物理页转为可访问 slice。

## 5. 复测结果

修复后执行同一命令：

```bash
SINGLE_TEST="/glibc/ltp/testcases/bin/diotest4" bash run.sh
```

结果：

- `diotest4` 全部 `TPASS`
- 无内核 panic
- QEMU 正常退出

关键日志文件：

- 修复前：`/tmp/diotest4_debug4.log`
- 修复后：`/tmp/diotest4_after_fix.log`

## 6. 结论

这次 `diotest4` 并非单点“heap lazy VMA”问题，而是：

1. `sys_write` 对 lazy 读页处理不完整（功能性缺陷）
2. 页表翻译缺少用户 PPN 防护（健壮性缺陷）

两者叠加导致“先 TFAIL，再 panic”。修复后问题已复现关闭。

## 7. 后续建议

1. 增加回归测试：`lazy user buffer + write` 必须成功（覆盖 `sys_write`）
2. 增加内核断言/统计：记录被拒绝的“用户越界 PPN”次数，便于追上游异常来源
3. 针对类似路径（`readv/writev/sendmsg/recvmsg`）统一复用“fault-in + 上界检查”策略

