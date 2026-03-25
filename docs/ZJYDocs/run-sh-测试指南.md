# `bash run.sh` 测试运行指南

日期：2026/3/20

## 概述

`run.sh` 是 rcore-lab 项目的一站式构建 + 运行脚本。它负责：

1. 调用 `make rv`（或 `make debug`）构建内核和用户态程序
2. 启动 QEMU，加载内核和 sdcard 镜像
3. 内核启动后执行 `initcode`（用户态第一个进程），由 `initcode.rs` 决定跑哪些测试

测试的选择逻辑**不在 `run.sh` 里**，而是通过**编译期环境变量**注入到 `user/src/bin/initcode.rs` 中。理解这个分层是关键。

---

## 基本用法

```bash
# 最简形式：构建 release 内核 + 跑全部测试（无日志）
bash run.sh -f sdcard-rv.img -t all

# 等价写法（-t rv 和 -t all 效果相同）
bash run.sh -f sdcard-rv.img -t rv
```

### 参数说明

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `-t, --type TYPE` | 构建类型：`rv`/`all`（release）或 `debug` | `rv` |
| `-f, --file FILE` | sdcard 镜像文件路径 | `sdcard-rv.img` |
| `-d` | 启用 GDB 调试（QEMU 加 `-s -S`，等待 GDB 连接） | 关闭 |
| `-n, --netforward` | 启用 user 模式网络端口转发（UDP 12345） | 关闭 |
| `--netmode MODE` | 网络模式：`user`/`tap`/`bridge` | `user` |
| `--netdump FILE` | QEMU 网络抓包到指定文件 | 无 |
| `--offline` | 强制离线构建（默认开启） | 开启 |
| `--online` | 允许联网构建（下载 crate 等） | - |

---

## 测试选择：通过环境变量控制

### 核心机制

`initcode.rs` 通过 `option_env!("SINGLE_TEST")` 在**编译期**读取环境变量。这意味着改变测试目标需要**重新编译**用户态程序。

`run.sh` 每次都会触发 `make`，所以只需在命令前加环境变量即可。

### 1. 跑全部测试套件（默认行为）

不设置 `SINGLE_TEST` 时，`initcode` 会遍历 `musl` 和 `glibc` 两个 libc 根目录下的所有测试套件：

```bash
bash run.sh -f sdcard-rv.img -t all
```

全部测试套件列表（定义在 `initcode.rs` 的 `TEST_SUITES`）：
- `basic`、`busybox`、`cyclictest`、`iozone`、`iperf`
- `libcbench`、`libctest`、`lmbench`、`ltp`、`lua`、`netperf`

执行顺序：先跑 `/musl` 下所有套件，再跑 `/glibc` 下所有套件。每个套件通过 `/{root}/{suite}_testcode.sh` 脚本运行（由 busybox sh 解释执行）。

### 2. 跑单个二进制测试

```bash
# 跑 sdcard 中的某个 ELF 二进制（绝对路径）
SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t all

# 跑 glibc 下的测试
SINGLE_TEST=/glibc/basic/fork bash run.sh -f sdcard-rv.img -t all
```

这会直接 `fork + execve` 该二进制，**完全绕过** shell 脚本和 busybox。适合调试单个测试点。

### 3. 跑某个 libc 的所有套件

```bash
# 只跑 musl 下所有套件
SINGLE_TEST=musl bash run.sh -f sdcard-rv.img -t all

# 只跑 glibc 下所有套件
SINGLE_TEST=glibc bash run.sh -f sdcard-rv.img -t all
```

### 4. 跑指定 libc + 指定套件

用 `{libc}-{suite}` 格式：

```bash
# 跑 musl 的 libctest 套件
SINGLE_TEST=musl-libctest bash run.sh -f sdcard-rv.img -t all

# 跑 glibc 的 basic 套件
SINGLE_TEST=glibc-basic bash run.sh -f sdcard-rv.img -t all

# 跑 musl 的 busybox 套件
SINGLE_TEST=musl-busybox bash run.sh -f sdcard-rv.img -t all
```

### 5. 跑指定套件（musl + glibc 都跑）

只写套件名（不带 libc 前缀）：

```bash
# basic 套件，musl 和 glibc 各跑一遍
SINGLE_TEST=basic bash run.sh -f sdcard-rv.img -t all

# libctest 套件
SINGLE_TEST=libctest bash run.sh -f sdcard-rv.img -t all
```

### 6. 跑内嵌的 single-elf 套件

这是硬编码在 `initcode.rs` 中的 `/musl/basic/*` 测试列表，逐个 ELF 直接执行（不经过 shell 脚本）：

```bash
SINGLE_TEST=single-elf bash run.sh -f sdcard-rv.img -t all
```

### 7. 跑嵌入的 pthread 测试

`initcode.rs` 内置了 `pthread_cancel_small` ELF。通过 `RUN_EMBEDDED_PTHREAD` 编译期开关启用：

```bash
RUN_EMBEDDED_PTHREAD=1 bash run.sh -f sdcard-rv.img -t all
```

此模式会写入 ELF 到 `/tmp/pthread_cancel_small` 并执行，然后**直接 shutdown**，不跑其他测试。

### SINGLE_TEST 选择器总结

| `SINGLE_TEST` 值 | 行为 |
|---|---|
| 未设置 | 跑全部套件（musl + glibc 各 11 个） |
| `/musl/basic/getpid` | 直接 execve 该 ELF |
| `all` | 等同于未设置，跑全部 |
| `single-elf` | 跑硬编码的 `/musl/basic/*` 列表 |
| `musl` | 跑 musl 下所有 11 个套件 |
| `glibc` | 跑 glibc 下所有 11 个套件 |
| `musl-libctest` | 跑 `/musl/libctest_testcode.sh` |
| `glibc-basic` | 跑 `/glibc/basic_testcode.sh` |
| `basic` | 跑 `/musl/basic_testcode.sh` + `/glibc/basic_testcode.sh` |
| `其他字符串` | 当作二进制路径尝试 execve |

---

## 日志控制

### LOG 环境变量

```bash
# 无日志（最快，默认 -t rv/all 时）
LOG=OFF bash run.sh -f sdcard-rv.img -t all

# 只看错误
LOG=ERROR bash run.sh -f sdcard-rv.img -t all

# 看警告和错误
LOG=WARN bash run.sh -f sdcard-rv.img -t all

# 看 syscall 级别信息
LOG=SYSCALL bash run.sh -f sdcard-rv.img -t all

# 看 info 级别
LOG=INFO bash run.sh -f sdcard-rv.img -t all

# 看全部日志（非常慢，日志量巨大）
LOG=TRACE bash run.sh -f sdcard-rv.img -t all

# debug 构建默认开启 TRACE
bash run.sh -f sdcard-rv.img -t debug
```

**注意**：`-t debug` 时默认 `LOG=TRACE`，`-t rv/all` 时默认 `LOG=OFF`。可通过显式设置 `LOG` 覆盖。

### 按进程过滤 syscall trace

```bash
# 只 trace pid=1 的系统调用
TRACE_PID=1 LOG=TRACE SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t debug

# 按进程名过滤（匹配 exec 后的 basename）
TRACE_NAME=getpid LOG=TRACE SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t debug
```

`TRACE_PID` 和 `TRACE_NAME` 也是**编译期**注入的（`option_env!`），只有匹配的进程才输出 syscall trace。在大量进程并发时非常有用。

---

## 日志保存与分析

推荐将输出重定向到日志文件：

```bash
# 第 1 轮调试
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all1.log 2>&1

# 第 2 轮（修复后验证）
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all2.log 2>&1

# 只看错误级别（日志量小很多）
LOG=ERROR bash run.sh -f sdcard-rv.img -t all > all3.log 2>&1
```

### 用 ripgrep 分析日志

```bash
# 搜索非法指令异常
rg "IllegalInstruction" all1.log

# 搜索页错误
rg "StorePageFault|InstructionPageFault|LoadPageFault" all1.log

# 搜索内核错误/警告
rg "\[ERROR\]|\[WARN\]" all1.log

# 搜索信号相关
rg "signum = 4|signum = 12|SIGKILL|bad addr" all1.log

# 搜索 panic
rg "Panicked|panic" all1.log

# 搜索 syscall 负返回值
rg "ret=-" all1.log

# 搜索 sepc/stval（trap 地址信息）
rg "sepc|stval" all1.log
```

### 判断测试是否通过

- 出现 `=== All tests completed ===` **不代表**测试通过（rcore 不会轻易 panic）
- 必须确认**没有** `IllegalInstruction`、`StorePageFault`、`SIGKILL`、`Panicked`、`bad addr`、`[ERROR]` 等关键字
- 测试用例应有正常输出（如 `test sbrk`、`unlink success` 等）

---

## GDB 调试

```bash
# 终端 1：启动 QEMU 等待 GDB
LOG=INFO bash run.sh -f sdcard-rv.img -t debug -d > debug.log 2>&1 &

# 终端 2：连接 GDB
riscv64-unknown-elf-gdb os/target/riscv64gc-unknown-none-elf/release/os
(gdb) target remote :1234
(gdb) set architecture riscv:rv64
(gdb) c
```

也可以用 `SINGLE_TEST` 缩小调试范围：

```bash
SINGLE_TEST=/musl/basic/fork LOG=INFO bash run.sh -f sdcard-rv.img -t debug -d > debug.log 2>&1 &
```

---

## 网络相关测试

网络测试套件（`iperf`、`netperf`）需要网络支持：

```bash
# user 模式 + 端口转发
bash run.sh -f sdcard-rv.img -t all -n

# tap 模式（需要先在宿主机创建 tap 设备）
bash run.sh -f sdcard-rv.img -t all --netmode tap --tap-ifname tap0

# bridge 模式
bash run.sh -f sdcard-rv.img -t all --netmode bridge --bridge br0 --tap-ifname tap0

# 抓包分析
bash run.sh -f sdcard-rv.img -t all --netdump net.pcap
```

TAP/Bridge 模式需要在宿主机预先配置：

```bash
sudo ip tuntap add dev tap0 mode tap user $USER
sudo ip link set tap0 up
sudo ip link add br0 type bridge
sudo ip link set br0 up
sudo ip link set tap0 master br0
sudo ip addr add 10.0.2.2/24 dev br0
```

---

## 常用命令速查

```bash
# === 日常开发 ===
# 快速跑全部测试，不要日志
bash run.sh -f sdcard-rv.img -t all

# 只跑 musl basic 套件
SINGLE_TEST=musl-basic bash run.sh -f sdcard-rv.img -t all

# 调试某个具体测试点
SINGLE_TEST=/musl/basic/fork LOG=TRACE bash run.sh -f sdcard-rv.img -t all > debug.log 2>&1

# 带 pid 过滤的精细 trace
TRACE_PID=1 LOG=TRACE SINGLE_TEST=/musl/basic/getpid bash run.sh -f sdcard-rv.img -t debug

# === 调试-测试循环 ===
# 第 N 轮调试（递增编号）
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all1.log 2>&1
# 分析日志
rg "IllegalInstruction|StorePageFault|SIGKILL|Panicked|\[ERROR\]" all1.log
# 修复后验证
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > all2.log 2>&1

# === release vs debug 构建 ===
# release（快，无优化日志）
bash run.sh -f sdcard-rv.img -t rv
# debug（慢，默认 TRACE 日志，含调试符号）
bash run.sh -f sdcard-rv.img -t debug

# === 额外磁盘 ===
# 如果根目录存在 disk.img，会自动作为第二块 virtio-blk 挂载
```

---

## 环境变量速查

| 变量 | 阶段 | 说明 |
|------|------|------|
| `LOG` | 编译期+运行期 | 日志级别：`OFF`/`ERROR`/`WARN`/`INFO`/`SYSCALL`/`DEBUG`/`TRACE` |
| `SINGLE_TEST` | 编译期 | 测试选择器（见上表） |
| `TRACE_PID` | 编译期 | 只对指定 pid 输出 syscall trace |
| `TRACE_NAME` | 编译期 | 只对指定进程名输出 syscall trace |
| `RUN_EMBEDDED_PTHREAD` | 编译期 | 启用内嵌 pthread 测试（设为 `1`） |
| `OFFLINE` | 构建期 | 离线构建（默认 `1`） |

**"编译期"意味着改变这些变量后必须重新编译才会生效。** `run.sh` 每次都会触发 `make`，所以正常流程下无需手动清理。但如果发现环境变量改了但行为没变，尝试 `make clean` 后重新运行。
