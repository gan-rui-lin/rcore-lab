# rCore-Lab macOS 环境配置指南

## 文档信息
- **日期**: 2026-02-12
- **平台**: macOS (Apple Silicon - ARM64)
- **系统版本**: Darwin 24.6.0

## 概述

本文档记录了在 macOS 系统上配置 rCore-Lab 开发环境的完整过程，包括遇到的问题和解决方案。

## 初始问题

### 1. 网络连接问题

执行 `make run` 时遇到以下错误：

```bash
sed: 1: "s/^ch([0-9]+).*\1/p": unterminated substitute in regular expression
/bin/sh: rustup: command not found
make: *** [env] Error 127
```

尝试安装 Rust 时出现 DNS 解析失败：

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
error: Could not resolve host: static.rust-lang.org
```

**原因分析**:
- DNS 配置问题（nameserver 指向 localhost）
- 网络延迟极高（ping 时间 5-7 秒）
- 无法直接访问 Rust 官方服务器

### 2. Makefile 兼容性问题

Makefile 第 34 行的 sed 命令在 macOS BSD sed 中语法错误：
```makefile
CHAPTER ?= $(shell git rev-parse --abbrev-ref HEAD | sed -nE 's/^ch([0-9]+).*$/\1/p')
```

**问题**: macOS BSD sed 需要对 `$` 进行额外转义。

## 解决方案

### 1. 修复 Makefile

将 Makefile 第 34 行修改为：

```makefile
CHAPTER ?= $(shell git rev-parse --abbrev-ref HEAD | sed -nE 's/^ch([0-9]+).*$$/\1/p')
```

**变更**: `$` 改为 `$$` 以适配 macOS sed。

### 2. 通过 Homebrew 安装 Rust

由于网络问题无法使用官方安装脚本，改用 Homebrew：

```bash
# 安装 Rust
brew install rust

# 安装 rustup（Rust 工具链管理器）
brew install rustup
```

**优势**:
- Homebrew 的镜像缓存机制可以绕过网络问题
- 安装过程更稳定
- 自动处理依赖关系

### 3. 配置 Rust 工具链

```bash
# 将 rustup 添加到 PATH
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

# 初始化 stable 工具链
rustup default stable

# 安装项目所需的 nightly 工具链（根据 rust-toolchain.toml）
rustup toolchain install nightly-2024-05-02

# 添加必要的组件
rustup component add rust-src llvm-tools-preview --toolchain nightly-2024-05-02

# 安装 RISC-V 目标
rustup target add riscv64gc-unknown-none-elf --toolchain nightly-2024-05-02
```

### 4. 安装 cargo-binutils

由于 nightly-2024-05-02 版本过旧，使用 stable 工具链安装：

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo +stable install cargo-binutils
```

这将安装以下工具：
- rust-objdump
- rust-objcopy
- rust-nm
- rust-size
- 等其他二进制工具

### 5. 配置环境变量

将以下内容添加到 `~/.zshrc`：

```bash
# rustup 路径
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

# cargo 二进制工具路径
export PATH="$HOME/.cargo/bin:$PATH"
```

应用配置：
```bash
source ~/.zshrc
```

## 环境验证

### 检查已安装工具

```bash
# 检查 rustup
rustup --version
# Output: rustup 1.28.2

# 检查 cargo
cargo --version
# Output: cargo 1.93.0

# 检查工具链
rustup show
# Output:
# Default host: aarch64-apple-darwin
# installed toolchains:
#   stable-aarch64-apple-darwin (default)
#   nightly-2024-05-02-aarch64-apple-darwin (active)
# active toolchain:
#   nightly-2024-05-02-aarch64-apple-darwin
#   overridden by '/Users/mac/Desktop/project/rcore-lab/rust-toolchain.toml'

# 检查 RISC-V 目标
rustup target list | grep riscv64gc-unknown-none-elf
# Output: riscv64gc-unknown-none-elf (installed)

# 检查二进制工具
which rust-objcopy rust-objdump
# Output:
# /Users/mac/.cargo/bin/rust-objcopy
# /Users/mac/.cargo/bin/rust-objdump

# 检查 QEMU
qemu-system-riscv64 --version
# Output: QEMU emulator version 9.x.x
```

### 构建测试

```bash
cd /Users/mac/Desktop/project/rcore-lab/os
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH"
make run
```

**构建结果**:
- ✅ 用户程序编译成功（initcode, sig_simple, sig_simple2, sig_tests）
- ✅ 内核编译成功（约 13.83 秒）
- ✅ 文件系统镜像创建成功
- ✅ QEMU 成功启动 OS

## 完整工具清单

### 已安装的工具
1. **Rust 1.93.0** (via Homebrew)
2. **rustup 1.28.2** (Rust 工具链管理器)
3. **nightly-2024-05-02 工具链** (rCore-Lab 项目要求)
4. **riscv64gc-unknown-none-elf** (RISC-V 目标平台)
5. **cargo-binutils** (Rust 二进制工具集)
6. **rust-src** (Rust 源代码组件)
7. **llvm-tools-preview** (LLVM 工具预览版)
8. **QEMU** (RISC-V 虚拟机，已通过 Homebrew 安装)

### 依赖项（自动安装）
- libssh2
- libgit2
- z3
- llvm
- pkgconf
- openssl@3
- sqlite
- zstd
- python@3.14

## 使用指南

### 编译并运行 OS

```bash
cd /Users/mac/Desktop/project/rcore-lab/os
make run
```

### 退出 QEMU

按键顺序：`Ctrl+A` 然后 `X`

### 清理构建

```bash
make clean
```

### 查看反汇编

```bash
make disasm
```

### 仅构建内核

```bash
make kernel
```

### 构建文件系统镜像

```bash
make fs-img
```

## 常见问题

### Q1: 新终端中找不到 rustup 命令

**解决方案**: 确保已经在 `~/.zshrc` 中添加了 PATH 配置，并执行 `source ~/.zshrc`

### Q2: rust-objcopy: command not found

**解决方案**:
```bash
echo 'export PATH="$HOME/.cargo/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

### Q3: 编译时报错 "rustc 1.80.0-nightly is not supported"

**解决方案**: 使用 stable 工具链安装工具：
```bash
cargo +stable install <package-name>
```

### Q4: 网络问题导致无法下载依赖

**解决方案**:
1. 使用 Homebrew 安装基础工具
2. 配置 cargo 镜像源（可选）
3. 考虑使用 Docker 环境（项目支持）

## macOS 特定注意事项

### BSD sed vs GNU sed
macOS 使用 BSD sed，语法与 GNU sed 略有不同：
- 正则表达式中的 `$` 需要双重转义为 `$$`
- 某些选项可能不兼容

### Apple Silicon (ARM64)
- 确保安装的是 ARM64 版本的工具（aarch64-apple-darwin）
- Homebrew 默认安装到 `/opt/homebrew/`
- Intel Mac 的路径为 `/usr/local/`

### 性能优化
- 首次编译可能较慢（需要编译所有依赖）
- 后续编译会利用 cargo 缓存，速度会快很多
- 考虑使用 `sccache` 加速重复编译

## 项目结构

```
rcore-lab/
├── os/                 # 操作系统内核代码
├── user/              # 用户态程序
├── easy-fs/           # 简易文件系统
├── easy-fs-fuse/      # 文件系统工具
├── bootloader/        # SBI 引导加载程序
├── vendor/            # 第三方依赖（vendored）
└── docs/              # 文档目录
    └── ZJYDocs/       # 个人学习文档
```

## 后续步骤

1. 阅读 [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
2. 按章节完成实验（ch1-ch9）
3. 使用 `git checkout ch$ID` 切换到不同章节
4. 完成后使用测试框架验证：
   ```bash
   cd ci-user && make test CHAPTER=$ID
   ```

## 参考资料

- [rCore-Tutorial-Guide](https://LearningOS.github.io/rCore-Tutorial-Guide/)
- [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
- [Rust 官方文档](https://doc.rust-lang.org/)
- [RISC-V 规范](https://riscv.org/specifications/)

## 更新日志

### 2026-02-12
- 初始环境配置完成
- 修复 Makefile macOS 兼容性问题
- 通过 Homebrew 成功安装所有依赖
- 首次成功构建并运行 OS

---

**作者**: Claude Code
**最后更新**: 2026-02-12
