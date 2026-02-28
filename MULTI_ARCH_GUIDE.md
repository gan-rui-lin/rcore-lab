# rcore-lab 多架构支持指南

## 概述

rcore-lab 现已支持两种 CPU 架构：
- **RISC-V 64** (`riscv64`) - 默认架构，使用 SBI 固件启动
- **LoongArch 64** (`loongarch64`) - 国产龙芯架构，使用 DMW 直接内存窗口启动

两种架构共享相同的内核代码库，通过条件编译实现架构特定功能。

---

## 快速开始

### 1. 环境准备

#### 所有架构通用依赖
```bash
# 安装 Rust nightly 工具链
rustup toolchain install nightly-2024-05-02

# 安装必要组件
rustup component add rust-src llvm-tools-preview
cargo install cargo-binutils
```

#### RISC-V 架构依赖
```bash
# 添加 RISC-V 目标
rustup target add riscv64gc-unknown-none-elf

# QEMU (macOS)
brew install qemu

# QEMU (Ubuntu/Debian)
sudo apt-get install qemu-system-riscv64
```

#### LoongArch 架构依赖
```bash
# 添加 LoongArch 目标（裸机）
rustup target add loongarch64-unknown-none

# QEMU LoongArch 支持（需要 QEMU 7.0+）
# macOS (通过源码编译或使用 Docker)
# Ubuntu/Debian (需要较新版本)
sudo apt-get install qemu-system-loongarch64  # QEMU >= 7.0
```

**注意**：LoongArch 使用 Rust 自带的 `rust-lld` 链接器，无需额外安装 `loongarch64-linux-gnu-*` 工具链。

---

## 构建和运行

### 构建 RISC-V 内核（默认）

```bash
cd os

# 方式 1: 使用默认架构
make build

# 方式 2: 显式指定架构
make build ARCH=riscv64

# 运行
make run
```

### 构建 LoongArch 内核

```bash
cd os

# 构建
make build ARCH=loongarch64

# 运行（需要 QEMU LoongArch 支持）
make run ARCH=loongarch64
```

### 只编译内核（不构建用户程序）

```bash
# RISC-V
make kernel

# LoongArch
make kernel ARCH=loongarch64
```

### 清理构建产物

```bash
# 清理所有架构
make clean

# 清理后重新构建
make build ARCH=loongarch64
```

---

## 架构对比

| 特性 | RISC-V 64 | LoongArch 64 |
|------|-----------|--------------|
| **目标三元组** | `riscv64gc-unknown-none-elf` | `loongarch64-unknown-none` |
| **链接器** | `rust-lld` (默认) | `rust-lld` (显式配置) |
| **启动方式** | SBI 固件 (RustSBI) | DMW 直接映射 |
| **系统调用指令** | `ecall` | `syscall 0` |
| **中断 CSR** | sstatus, stvec | CRMD, EENTRY |
| **页表格式** | SV39 (39位, 3级) | LA64 (39位, 3级) |
| **TLB 管理** | 硬件自动 | 手动 invtlb |
| **非对齐访问** | 硬件支持 | 软件模拟（已实现） |
| **栈指针寄存器** | x2 (sp) | r3 (sp) |
| **返回地址寄存器** | x1 (ra) | r1 (ra) |
| **QEMU 虚拟机** | `qemu-system-riscv64` | `qemu-system-loongarch64` |

---

## 目录结构

### 架构相关代码

```
os/src/
├── arch/                       # 架构特定实现
│   ├── mod.rs                 # 架构选择（通过 cfg）
│   ├── riscv64/               # RISC-V 实现
│   │   ├── boot.rs            # 启动代码
│   │   ├── context.rs         # Trap 上下文
│   │   ├── trap.rs            # 异常处理
│   │   └── ...
│   └── loongarch64/           # LoongArch 实现
│       ├── boot.rs            # DMW 初始化
│       ├── console.rs         # UART 16550 驱动
│       ├── context.rs         # Trap 上下文
│       ├── switch.rs          # 任务切换
│       ├── timer.rs           # 定时器
│       ├── trap.rs            # 异常处理
│       ├── consts.rs          # 常量定义
│       └── linker.ld          # 链接脚本
├── trap/                      # 架构无关 trap 接口
├── mm/                        # 内存管理
├── task/                      # 任务管理
└── syscall/                   # 系统调用
```

### 配置文件

- **os/.cargo/config.toml** - 内核编译配置（包含两种架构的 rustflags）
- **user/.cargo/config.toml** - 用户程序编译配置
- **os/Makefile** - 构建脚本（支持 ARCH 参数）
- **user/Makefile** - 用户程序构建脚本

---

## 编译配置详解

### 内核配置 (os/.cargo/config.toml)

```toml
[build]
target = "riscv64gc-unknown-none-elf"  # 默认目标

[target.riscv64gc-unknown-none-elf]
rustflags = [
    "-Clink-arg=-Tsrc/linker.ld",
    "-Cforce-frame-pointers=yes"
]

[target.loongarch64-unknown-none]
linker = "rust-lld"                    # 使用 LLVM LLD 链接器
rustflags = [
    "-Clink-arg=-Tsrc/arch/loongarch64/linker.ld",
    "-Cforce-frame-pointers=yes",
    "-Ctarget-feature=-ual",           # 禁用非对齐访问（软件模拟）
    "--cfg=board=\"qemu\""
]

[unstable]
build-std = ["core", "compiler_builtins", "alloc"]
build-std-features = ["compiler-builtins-mem"]
```

### LoongArch 特殊编译参数

由于 LoongArch 是裸机目标，需要使用 `-Zbuild-std` 从源码编译标准库：

```bash
cargo build --release \
    --target loongarch64-unknown-none \
    -Zbuild-std=core,alloc,compiler_builtins \
    -Zbuild-std-features=compiler-builtins-mem
```

---

## QEMU 运行参数

### RISC-V QEMU 启动命令

```bash
qemu-system-riscv64 \
    -machine virt \
    -nographic \
    -bios bootloader/rustsbi-qemu.bin \
    -device loader,file=os/target/riscv64gc-unknown-none-elf/release/os.bin,addr=0x80200000 \
    -drive file=user/target/riscv64gc-unknown-none-elf/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0
```

**参数说明**：
- `-machine virt` - 使用 QEMU RISC-V virt 虚拟板
- `-bios` - RustSBI 固件，提供 SBI 接口
- `-device loader` - 将内核加载到 0x80200000（KERNEL_ENTRY_PA）
- `-drive` + `-device virtio-blk-device` - 挂载文件系统镜像

### LoongArch QEMU 启动命令

```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel os/target/loongarch64-unknown-none/release/os \
    -drive file=user/target/loongarch64-unknown-none/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0
```

**参数说明**：
- `-machine virt` - QEMU LoongArch virt 虚拟板
- `-cpu la464-loongarch-cpu` - 使用 LA464 CPU（支持最新 LoongArch 指令集）
- `-kernel` - 直接加载 ELF 格式内核（无需 BIOS）
- `-m 128M` - 分配 128MB 内存

**关键差异**：
- LoongArch **不需要 BIOS/固件**，内核直接从 `_start` 入口点启动
- 使用 `-kernel` 加载 ELF 文件，而不是 `-device loader`
- 必须指定 CPU 型号 `-cpu la464-loongarch-cpu`

---

## 条件编译示例

### 在代码中选择架构

```rust
// 导入架构特定的寄存器操作
#[cfg(target_arch = "riscv64")]
use riscv::register::{sstatus, stvec, scause};

#[cfg(target_arch = "loongarch64")]
use loongarch64::register::{crmd, eentry, estat};

// 根据架构执行不同代码
pub fn init() {
    #[cfg(target_arch = "riscv64")]
    unsafe {
        stvec::write(trap_handler as usize, stvec::TrapMode::Direct);
    }

    #[cfg(target_arch = "loongarch64")]
    {
        eentry::set_eentry(trap_handler as usize);
    }
}
```

### 系统调用汇编差异

```rust
// user/src/syscall.rs
fn syscall(id: usize, args: [usize; 6]) -> isize {
    let mut ret: isize;

    #[cfg(target_arch = "riscv64")]
    unsafe {
        core::arch::asm!(
            "ecall",
            inlateout("x10") args[0] => ret,
            in("x11") args[1],
            // ...
            in("x17") id,
        );
    }

    #[cfg(target_arch = "loongarch64")]
    unsafe {
        core::arch::asm!(
            "syscall 0",
            inlateout("$a0") args[0] => ret,
            in("$a1") args[1],
            // ...
            in("$a7") id,
        );
    }

    ret
}
```

---

## 常见问题

### Q1: LoongArch 编译失败：`linking with cc failed`

**问题**：使用系统默认的 `cc` 链接器导致失败。

**解决方案**：确保 `.cargo/config.toml` 中配置了 `linker = "rust-lld"`：

```toml
[target.loongarch64-unknown-none]
linker = "rust-lld"
```

### Q2: 找不到 `loongarch64-unknown-none` 目标

**解决方案**：
```bash
rustup target add loongarch64-unknown-none
```

### Q3: QEMU 报错 `CPU model not found`

**问题**：QEMU 版本过低，不支持 LoongArch。

**解决方案**：
- 升级 QEMU 到 7.0 或更高版本
- 或使用 Docker 容器（见下文）

### Q4: LoongArch 用户程序缺少链接脚本

**问题**：`user/src/linker-loongarch64.ld` 不存在。

**解决方案**：参考 RISC-V 的链接脚本创建 LoongArch 版本（或当前可以复用 RISC-V 链接脚本）。

### Q5: 非对齐访问导致程序崩溃

**说明**：LoongArch 硬件不支持非对齐访问，已在 `os/src/arch/loongarch64/trap.rs` 中实现软件模拟。如果遇到相关问题，检查 `UnalignedAccess` 异常处理逻辑。

---

## Docker 开发环境（可选）

如果本地环境配置困难，可以使用 Docker：

### 使用 OSKernel2025 Docker 镜像

```bash
# 构建镜像（包含 LoongArch 工具链）
cd /path/to/OSKernel2025-rustoswhu
docker build -t rcore-loongarch:latest .

# 运行容器
docker run --rm -it -v $(pwd):/workspace -w /workspace rcore-loongarch:latest bash

# 在容器内编译
cd /workspace/rcore-lab/os
make build ARCH=loongarch64
make run ARCH=loongarch64
```

---

## 调试技巧

### 查看生成的汇编代码

```bash
# RISC-V
make disasm

# LoongArch
make disasm ARCH=loongarch64
```

### GDB 调试

#### RISC-V 调试
```bash
# 终端 1：启动 QEMU GDB 服务器
make gdbserver

# 终端 2：连接 GDB
make gdbclient
```

#### LoongArch 调试
```bash
# 终端 1：启动 QEMU（添加 -s -S 参数）
qemu-system-loongarch64 -machine virt -cpu la464-loongarch-cpu \
    -kernel os/target/loongarch64-unknown-none/release/os \
    -nographic -s -S

# 终端 2：使用 GDB 连接（需要 loongarch64-linux-gnu-gdb）
loongarch64-linux-gnu-gdb \
    -ex 'file os/target/loongarch64-unknown-none/release/os' \
    -ex 'target remote localhost:1234'
```

### 查看内核二进制信息

```bash
# 查看文件类型
file os/target/loongarch64-unknown-none/release/os
# 输出: ELF 64-bit LSB executable, LoongArch, version 1 (SYSV)

# 查看 ELF 头信息
rust-readobj --file-header os/target/loongarch64-unknown-none/release/os

# 查看符号表
rust-nm os/target/loongarch64-unknown-none/release/os | grep __switch
```

---

## 性能对比（理论）

| 指标 | RISC-V 64 | LoongArch 64 |
|------|-----------|--------------|
| 指令集复杂度 | 简单（精简指令集） | 中等（MIPS-like） |
| 系统调用开销 | 低（`ecall` + SBI） | 低（`syscall` 直接） |
| TLB 刷新开销 | 低（硬件自动） | 中（需手动 invtlb） |
| 非对齐访问性能 | 高（硬件支持） | 低（软件模拟，约 20% 损失） |
| 生态成熟度 | 高（工具链完善） | 中（快速发展中） |

---

## 未来扩展

### 计划支持的架构
- **AArch64** (ARM 64位) - 移动和嵌入式设备
- **x86_64** - 传统 PC 架构

### 多核支持（SMP）
当前实现为单核，未来可扩展：
- LoongArch IPI（核间中断）
- RISC-V SBI 多核启动
- 共享调度器

### 真实硬件支持
- **RISC-V**：Allwinner D1, StarFive VisionFive 2
- **LoongArch**：龙芯 3A5000, 2K1000

---

## 参考资料

### 官方文档
- [RISC-V Specifications](https://riscv.org/technical/specifications/)
- [LoongArch Reference Manual](http://loongson.cn/download)
- [QEMU LoongArch Documentation](https://www.qemu.org/docs/master/system/target-loongarch.html)

### 相关项目
- [rCore-Tutorial-v3](https://github.com/rcore-os/rCore-Tutorial-v3) - 原始 RISC-V 教程
- [OSKernel2025-rustoswhu](https://github.com/oskernel2025/rustoswhu) - LoongArch 参考实现
- [RustSBI](https://github.com/rustsbi/rustsbi) - RISC-V SBI 固件

### 学习资源
- 《RISC-V 体系结构编程与实践》
- 《LoongArch 指令集手册》

---

## 贡献指南

欢迎为 rcore-lab 多架构支持做贡献！

### 添加新架构的步骤
1. 在 `os/src/arch/<new_arch>/` 创建架构目录
2. 实现必要模块：boot、context、trap、timer、switch
3. 更新 `os/src/arch/mod.rs` 添加条件编译
4. 更新 Makefile 添加构建规则
5. 更新 `.cargo/config.toml` 配置
6. 编写测试用例并验证功能

### 提交 Pull Request
- 确保 RISC-V 构建不受影响
- 添加充分的代码注释
- 更新相关文档

---

## 致谢

- **rCore-Tutorial 团队** - 提供了优秀的 RISC-V 教程基础
- **OSKernel2025-rustoswhu 团队** - 提供了 LoongArch 参考实现
- **Rust 社区** - 提供了强大的工具链支持

---

## 许可证

本项目遵循与 rCore-Tutorial-v3 相同的许可证。

---

**最后更新**：2026-02-27
**维护者**：rcore-lab contributors
