# rcore-lab 双架构快速启动指南

## 一分钟上手

### RISC-V 架构（默认）

```bash
# 1. 安装依赖
rustup target add riscv64gc-unknown-none-elf
rustup component add rust-src llvm-tools-preview
cargo install cargo-binutils

# 2. 编译运行
cd os
make build    # 或 make build ARCH=riscv64
make run
```

### LoongArch 架构

```bash
# 1. 安装依赖
rustup target add loongarch64-unknown-none
rustup component add rust-src llvm-tools-preview

# 2. 编译（无需外部工具链，使用 rust-lld）
cd os
make build ARCH=loongarch64

# 3. 运行（需要 QEMU 7.0+ 支持 LoongArch）
make run ARCH=loongarch64
```

## 编译结果

### RISC-V 内核
- **位置**：`os/target/riscv64gc-unknown-none-elf/release/os`
- **大小**：约 400KB
- **格式**：ELF 64-bit LSB executable, UCB RISC-V

### LoongArch 内核
- **位置**：`os/target/loongarch64-unknown-none/release/os`
- **大小**：约 641KB
- **格式**：ELF 64-bit LSB executable, LoongArch

## 常用命令对照表

| 操作 | RISC-V | LoongArch |
|------|--------|-----------|
| 只编译内核 | `make kernel` | `make kernel ARCH=loongarch64` |
| 编译+运行 | `make run` | `make run ARCH=loongarch64` |
| 清理 | `make clean` | `make clean` |
| 查看汇编 | `make disasm` | `make disasm ARCH=loongarch64` |

## 关键差异

| 特性 | RISC-V | LoongArch |
|------|--------|-----------|
| 链接器 | rust-lld (自动) | rust-lld (配置) |
| 标准库编译 | 预编译 | 从源码编译 (-Zbuild-std) |
| 系统调用 | `ecall` | `syscall 0` |
| 启动方式 | SBI 固件 | DMW 直接映射 |

## 验证安装

```bash
# 检查 Rust 工具链
rustup target list | grep -E 'riscv64gc|loongarch64'

# 应该看到：
# loongarch64-unknown-none (installed)
# riscv64gc-unknown-none-elf (installed)

# 检查 QEMU
qemu-system-riscv64 --version      # RISC-V
qemu-system-loongarch64 --version  # LoongArch (可选)
```

## 故障排除

### LoongArch 编译失败

**症状**：`linking with cc failed`

**解决方案**：确认 `os/.cargo/config.toml` 包含以下配置：
```toml
[target.loongarch64-unknown-none]
linker = "rust-lld"
```

### 找不到目标

**症状**：`error: couldn't find target loongarch64-unknown-none`

**解决方案**：
```bash
rustup target add loongarch64-unknown-none
```

### QEMU 不支持 LoongArch

**解决方案**：
- macOS: 编译 QEMU 源码或使用 Docker
- Linux: 升级 QEMU 到 7.0+

## 详细文档

完整文档请参考 [MULTI_ARCH_GUIDE.md](MULTI_ARCH_GUIDE.md)

## 技术支持

- 提交 Issue：[GitHub Issues](https://github.com/your-repo/rcore-lab/issues)
- 参考实现：[OSKernel2025-rustoswhu](https://github.com/oskernel2025/rustoswhu)
