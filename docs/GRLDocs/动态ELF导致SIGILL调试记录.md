# 动态 ELF SIGILL 调试记录

本文记录一次针对 rcore-lab 中 IllegalInstruction (SIGILL) 的完整调试过程，包含定位思路、关键日志、推理链条与修复路径。目标是把“为什么会 SIGILL、怎么定位、如何修复”讲清楚。

## 1. 现象与初始症状

在 release 运行 musl/basic 测试时，偶发出现 IllegalInstruction：

- 日志中出现：
  - "trap_handler: IllegalInstruction in application"
  - sepc 指向某个地址（例如 0x11bbb8 或 0x0）
  - stval 为 0 或 ELF 魔数
- 子进程被默认 SIGILL handler 结束
- 但父进程继续运行，看起来测试并非完全崩溃，属于“隐性错误”。

这个现象明显是问题：用户态执行到无效指令或非法地址，说明装载或执行路径有缺陷。

## 2. 初始假设与排查方向

最早的怀疑点有两个：

1) 文件读取异常导致入口代码全 0
- 之前在 release 中观测到 /bin/sh 或 busybox 入口字节为 0。
- 可能是 ext4 read_at 打开路径不一致导致 read_all 读取空数据。

2) 动态链接 ELF 未正确处理
- musl/basic 中部分程序是动态 ELF，包含 PT_INTERP。
- 如果内核未处理解释器，可能直接从 ELF 头部或 0 地址执行。

## 3. 证据链与日志对比

### 3.1 入口字节为 0 的问题

早期日志中出现：
- /bin/sh entry bytes = 00 00 00 ...
- 入口内存字节同样为 0
- 直接触发 SIGILL

这是“文件读不到数据”的强证据。通过在 ext4 VFS 中对 read_at 添加 path 处理（去除前导 / 兼容），入口字节恢复正常。

### 3.2 SIGILL 发生位置不在入口

后续日志显示：
- entry bytes 正常
- 但运行过程中 sepc = 0x11bbb8 等
- 该处字节为 00 00 00 00

说明不是入口失败，而是运行期跳到了空白页或被清零区域。

### 3.3 sepc=0 + ELF 魔数

关键日志（all48.log）：

- sepc = 0x0
- sepc bytes = 7f 45 4c 46 02 01 01 00
- stval = 0x464c457f (ELF magic)

这非常明确：CPU 正在执行 ELF 文件头，说明内核在 exec 动态 ELF 时没有跳转到解释器入口，而是错误地从 0 开始执行。

## 4. 关键定位：PT_INTERP 未处理

通过分析 musl/basic 程序：
- readelf -l 显示 Requesting program interpreter: /lib/ld-linux-riscv64-lp64d.so.1
- 这些程序为动态 ELF

内核原本只支持静态 ELF，缺少 PT_INTERP 支持。

结果：
- 动态程序被当作普通 ELF 执行
- entry 设置为 0x1000 之类，但实际加载路径不完整
- 导致 CPU 进入无效地址或 ELF 头部

## 5. 解决策略与实现

### 5.1 先封堵 PT_INTERP

最先做的是检测 PT_INTERP 并直接 ENOEXEC，避免 SIGILL。这个能止损，但导致动态程序全部失败，测试中会出现 “not found”。

### 5.2 支持解释器执行

1) 在 sys_exec 中解析 PT_INTERP。
2) 若存在解释器路径，尝试 open /lib/ld-linux-riscv64-lp64d.so.1。
3) 若不存在，则回退到 /musl/lib/libc.so。
4) 将 exec_path 替换为解释器，argv 变为 [interp, original]
5) 递归走 exec 路径加载解释器。

这样可以让动态 ELF 通过解释器启动，避免执行 ELF 头部。

### 5.3 硬链接保证解释器存在

由于镜像中没有 /lib/ld-linux-riscv64-lp64d.so.1，必须创建硬链接指向 /musl/lib/libc.so。为此在 ensure_busybox_links 中加入：
- create_dir("/lib")
- ensure_hardlink("/lib/ld-linux-riscv64-lp64d.so.1", "/musl/lib/libc.so")

这样 PT_INTERP 能解析成功。

### 5.4 mmap 基址冲突修复

解释器启动后，第一次调用 sys_mmap 时触发 panic：

- "vpn is mapped before mapping"
- 日志显示 mmap 从 0x40000000 开始
- 解释器本身 PT_LOAD 也位于 0x40000000

原因：mmap_base 固定为 0x4000_0000，与解释器映射重叠。

修复：
- exec 完成后将 mmap_base 设置为 max(DEFAULT_MMAP_BASE, heap_bottom 对齐后)
- 保证 mmap 区域在已加载镜像之后

这消除了重复映射 panic。

## 6. 关键日志与推理链条

关键证据链如下：

1) sepc bytes = 00..00
   - 说明执行到空白页
2) sepc bytes = 7f 45 4c 46
   - 说明执行 ELF 头
3) readelf 显示 PT_INTERP
   - 说明动态 ELF 需要解释器
4) /lib/ld-linux-riscv64-lp64d.so.1 不存在
   - 解释器路径不可解析
5) 引入解释器执行逻辑 + 链接修复
   - 解释器可启动
6) sys_mmap panic
   - mmap_base 与解释器地址冲突
7) 调整 mmap_base
   - panic 消失

每一步都能与日志直接对应，形成完整因果链。

## 7. 当前状态

- SIGILL 由动态 ELF 触发的问题已被解释器执行逻辑覆盖。
- 镜像中硬链接确保 /lib/ld-linux-riscv64-lp64d.so.1 存在。
- mmap_base 与解释器加载区冲突已修正。

剩余风险：
- 动态链接器行为是否完全兼容
- auxv 对解释器是否完整
- 仍可能出现其他动态链接相关错误

## 8. 后续建议

1) 对动态链接器加载流程增加更多调试：打印 interp_entry、AT_BASE、AT_ENTRY。
2) 若出现新的 SIGILL，优先检查 sepc bytes 与 file bytes 是否一致。
3) 若 musl 动态程序仍异常，可考虑补齐 /musl/lib 下的依赖库和符号解析路径。

这次调试证明：SIGILL 并不是“随机异常”，而是动态 ELF 未正确处理的结果。通过逐步定位与验证，可以稳定收敛到真正的根因与修复路径。