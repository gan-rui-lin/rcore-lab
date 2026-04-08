# iozone `Bad address` 根因与修复说明（2026-04-08）

## 1. 问题现象

在 LoongArch 跑 iozone 时，读阶段出现：

- `Error freading block 0 600100000`
- `read: Bad address`
- 子进程退出码 `54`（`iozone` 报错退出）

对应日志样式可见你最初给出的片段（`all-la.log` 438-459 行）以及后续最小复现日志。

## 2. 复现与排查策略

按照“先缩小问题面再回归”的方式：

1. 先做你要求的 `-i 6` 小测（仅写路径）  
   入口脚本：[`initcode.rs`](/home/grl/codeRepo/rcore-lab/user/src/bin/initcode.rs:57)
2. 再验证包含读路径的用例（`-i 0 -i 1`）

说明：`-i 6` 本身只覆盖 `fwriter`，不会触发 `fread` 路径，所以它主要用于快速验证“系统整体可跑、写路径正常”，不直接复现此次 `Bad address` 根因。

### 2.1 实际调试过程（怎么调出来的）

这次是按“先证据、再猜想、再改代码”的顺序做的：

1. 从失败日志确认症状和阶段  
   先看 `all-la.log` 出错段，确认是 iozone 读阶段报错而不是随机崩溃：  
   `Error freading block ...` + `read: Bad address` + `exit code=54`。

2. 先做最小可控复现，避免 `-t all` 噪声  
   在 `initcode.rs` 增加 `SINGLE_TEST=tmp` 小脚本，先只跑你指定的 `-i 6`，确保基础链路稳定、迭代快。

3. 用“能复现错误”的最小组合锁定读路径  
   观察到 `-i 6` 仅写路径，不会触发读错误；随后用包含读阶段的组合（`-i 0 -i 1`，例如 `tmp-iozone-4k`）确认 `Bad address` 可稳定复现。

4. 打细日志看 syscall 证据  
   运行 `TRACE_NAME=iozone LOG=SYSCALL ...`，并按关键字过滤：  
   `mmap/read/readv/pread64/ret=-14(EFAULT)/Bad address`。  
   关注点是“用户缓冲区地址在 `0x600...` mmap 区时，read 路径返回 EFAULT”。

5. 回到内核代码反推第一偏差点  
   从 `sys_read -> translated_user_write_buffer` 顺着看，发现旧逻辑只处理 COW，不处理 lazy mmap 尚未建页（无 PTE）场景，因此会把“可补页后继续”的情况提前判成 `EFAULT`。

6. 做最小修复并立即回归  
   在 `fs.rs` 的用户写缓冲区准备逻辑里先尝试 `handle_demand_fault`，再走 COW 和权限复核；同步把 `readv/pread64` 复用同一逻辑，避免 read 家族行为不一致。

7. 双重验证  
   - 快速验证：`SINGLE_TEST=tmp`（`-i 6`）通过  
   - 根因验证：`SINGLE_TEST=tmp-iozone-4k`（含 `-i 1` 读）不再出现 `Bad address/code=54`

## 3. 根因分析

### 3.1 出错点

内核在“把文件数据写入用户缓冲区”时，会检查用户地址是否可写。  
相关路径：

- `sys_read` / `sys_readv` / `sys_pread64`
- 最终依赖 `translated_user_write_buffer(...)` 与页表校验

核心问题在于：  
**原逻辑只尝试处理 COW（写保护）场景，没有处理 lazy mmap 尚未建 PTE 的场景。**

因此当用户缓冲区位于 lazy 映射区域、但页尚未 fault-in 时，会被直接判成 `EFAULT`，用户态就看到 `Bad address`。

### 3.2 为什么 iozone 会踩中

iozone 多进程读写中会频繁使用 mmap/缓冲区，地址段落在 `0x600...`（mmap 区）很常见。  
当读路径首次写入这些“尚未 materialize 的用户页”时，内核未先补页，直接返回 `EFAULT`，于是出现 `Error freading block ... Bad address`。

## 4. 代码改动

### 4.1 修复点 1：写入用户缓冲区前先补 lazy 页

文件：[`os/src/syscall/fs.rs`](/home/grl/codeRepo/rcore-lab/os/src/syscall/fs.rs:668)

在 `try_resolve_user_cow_writable()` 中新增：

1. 若 `page_table.translate(vpn)` 为空，先调用 `handle_demand_fault(va)`
2. 再次读取 PTE 并校验 `U/W` 权限
3. 对只读页继续走 `handle_cow_fault(va)`
4. COW 后再次校验结果必须可写

这让“lazy 未建页”和“COW 只读页”都能在内核 copy 前被正确修复。

### 4.2 修复点 2：`readv/pread64` 统一复用同一缓冲区修复逻辑

原先 `sys_readv` 与 `sys_pread64` 直接调用 `translated_byte_buffer_checked(..., writable=true)`，对 lazy 页不友好。  
现改为复用 `translated_user_write_buffer(...)`：

- [`sys_readv` 调整](/home/grl/codeRepo/rcore-lab/os/src/syscall/fs.rs:2839)
- [`sys_pread64` 调整](/home/grl/codeRepo/rcore-lab/os/src/syscall/fs.rs:4045)

这样 read 系列路径行为一致，避免遗漏。

### 4.3 小测入口（按本次需求）

新增 `SINGLE_TEST=tmp` 快速脚本（只跑 `-i 6`）：

- 脚本定义：[`initcode.rs`](/home/grl/codeRepo/rcore-lab/user/src/bin/initcode.rs:57)
- 选择器入口：[`initcode.rs`](/home/grl/codeRepo/rcore-lab/user/src/bin/initcode.rs:1060)

## 5. 验证结果

### 5.1 你要求的小测（`-i 6`）

命令：

```bash
SINGLE_TEST=tmp LOG=INFO bash run-la.sh -f sdcard-la.img -t all
```

结果：

- `tmp-iozone-quick exit code: 0`
- `fwriters` 正常输出
- 无 `Bad address`

### 5.2 读路径回归（`-i 0 -i 1`）

命令：

```bash
SINGLE_TEST=tmp-iozone-4k LOG=INFO bash run-la.sh -f sdcard-la.img -t all
```

结果：

- `Command line used: ./iozone -t 4 -i 0 -i 1 -r 4k -s 1m`
- `iozone test complete.`
- `status=0x0`
- 无 `Error freading block ...`、无 `Bad address`、无 `code=54`

## 6. 结论

本次 `Bad address` 的根因不是 iozone 本身，而是内核 read 路径对“lazy 用户页”处理不完整：  
**在 copy_to_user 前缺少 demand-fault 补页步骤，导致误报 `EFAULT`。**

修复后：

- `-i 6` 小测可稳定用于快速迭代
- 含读阶段的 iozone 用例也不再出现该 `Bad address` 问题
