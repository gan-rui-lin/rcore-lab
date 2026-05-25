# rCore Lab

![Rust](https://img.shields.io/badge/Rust-nightly--2024--08--01-orange)
![Arch](https://img.shields.io/badge/Arch-RISC--V%20%7C%20LoongArch64-blue)

`rcore-lab` 是一个基于 rCore 教学内核继续扩展的实验操作系统项目。项目主要面向 RISC-V64 与 LoongArch64 平台，在 QEMU 上运行，重点探索 Linux 兼容系统调用、动态链接程序支持、文件系统、网络、信号、线程与 LTP/Libc 测试适配。

## 特性

- 支持 RISC-V64 与 LoongArch64 双架构构建。
- 支持 musl/glibc 用户程序运行与动态链接相关适配。
- 实现并扩展进程、线程、信号、futex、文件系统、网络等 Linux 风格接口。
- 支持 ext4/FAT/easy-fs 相关实验路径与 VirtIO 块设备。
- 支持 VirtIO 网络设备，提供 user/tap/bridge 等 QEMU 网络运行模式。
- 内置面向 basic、busybox、libctest、lmbench、LTP、iperf、netperf 等套件的 initcode 启动逻辑。
- 提供脚本化运行、离线构建、日志抓取和测试结果分析工具。

## 目录结构

```text
.
├── arch/               # 架构相关代码：RISC-V64、LoongArch64、页表、陷入、上下文切换
├── os/                 # 内核主体：任务、内存、文件系统、网络、系统调用、驱动
├── user/               # 用户态程序与 initcode，负责启动测试套件
├── easy-fs/            # rCore 教学文件系统
├── easy-fs-fuse/       # easy-fs 镜像制作工具
├── vendor/             # 本地依赖与适配过的第三方库
├── scripts/            # LTP/日志分析脚本
├── docs/               # 开发规范、调试记录、评测经验与专题文档
├── run.sh              # RISC-V QEMU 构建与运行入口
├── run-la.sh           # LoongArch64 QEMU 构建与运行入口
└── Makefile            # 顶层构建入口
```

## 环境要求

- Rust nightly，仓库使用 `rust-toolchain.toml` 固定到 `nightly-2024-08-01`
- `rust-src`、`llvm-tools-preview`、`cargo-binutils`
- `qemu-system-riscv64`
- `qemu-system-loongarch64`，用于 LoongArch64 运行
- `python3`、`make`、`xz`


## 快速开始

### RISC-V64

```bash
# 编译 RISC-V 内核
make rv

# 使用 sdcard-rv.img 启动 QEMU
bash run.sh -f sdcard-rv.img -t rv
```

### LoongArch64

```bash
# 编译 LoongArch64 内核
make la

# 使用 sdcard-la.img 启动 QEMU
bash run-la.sh -f sdcard-la.img -t la --no-data-disk
```

如果镜像不存在但存在对应的 `.xz` 压缩包，运行脚本会尝试自动解压。

## 运行指定测试

用户态入口在 `user/src/bin/initcode.rs`。可以通过编译期环境变量 `SINGLE_TEST` 控制运行的测试范围：

```bash
# 运行全部测试入口
SINGLE_TEST=all LOG=OFF bash run.sh -f sdcard-rv.img -t rv

# 只运行 musl 下的测试集合
SINGLE_TEST=musl LOG=ERROR bash run.sh -f sdcard-rv.img -t rv

# 只运行 glibc netperf
SINGLE_TEST=glibc-netperf LOG=ERROR bash run-la.sh -f sdcard-la.img -t la --no-data-disk

# 从某个 LTP case 开始继续跑
SINGLE_TEST=all LTP_START_FROM=waitpid10 LOG=OFF bash run.sh -f sdcard-rv.img -t rv
```

常用日志级别：

```bash
LOG=OFF      # 关闭大部分日志，适合评测
LOG=ERROR    # 只看错误
LOG=WARN     # 查看异常、信号、页故障等问题
LOG=SYSCALL  # 跟踪系统调用
LOG=TRACE    # 全量调试日志
```

## 常用命令

```bash
# 构建两个架构
make all

# debug 模式构建 RISC-V
make debug

# 清理构建产物
make clean

# 进入 Docker 开发环境
make docker

# 格式化主要 Rust 子工程
make fmt
```

## 调试与分析

常见调试方式是保存 QEMU 输出，然后用 `rg` 或脚本分析：

```bash
LOG=WARN SINGLE_TEST=all bash run.sh -f sdcard-rv.img -t rv > rv.log 2>&1

rg "Panicked|SIG|PageFault|IllegalInstruction|ret=-" rv.log
python3 scripts/ltp/analyze_rv_ltp_log.py rv.log
```

## 项目状态

项目处于活跃实验和评测适配阶段。代码中会保留一些针对具体测试点的兼容逻辑、性能优化和调试脚本。适合用于学习 Rust OS、理解 rCore 内核演进、调试 Linux 兼容接口，以及复现实验性系统调用与 libc 测试问题。

## License

本项目使用 [GPL-3.0](LICENSE) 许可证。
