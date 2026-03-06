# QEMU 版本兼容性问题修复

## 问题描述

运行 OS 时遇到错误：
```
[kernel] Panicked at src/drivers/block/virtio_blk.rs:29 assertion `left == right` failed: Error when reading VirtIOBlk
  left: Unsupported
 right: Ok
```

## 原因分析

- **当前 QEMU 版本**: 10.2.0（太新）
- **工作的版本**: 7.0.0
- **问题**: QEMU 10.x 对 VirtIO 规范实现更严格，某些旧代码不兼容

### 技术细节

QEMU 10.x 在 VirtIO 块设备操作时：
1. Feature negotiation 更严格
2. 某些未正确协商的特性会返回 `Unsupported`
3. 旧版本的 virtio-drivers crate 可能不支持新的协商流程

## 解决方案

### 方案 1：降级 QEMU 到 7.x（推荐）

#### 步骤 1: 卸载当前 QEMU

```bash
brew uninstall qemu
```

#### 步骤 2: 安装 QEMU 7.x

```bash
# 方法 A: 使用 homebrew-core 的旧版本
brew tap homebrew/core
brew install qemu@7
brew link qemu@7

# 方法 B: 从源码安装 QEMU 7.2.0
cd /tmp
wget https://download.qemu.org/qemu-7.2.0.tar.xz
tar xf qemu-7.2.0.tar.xz
cd qemu-7.2.0
./configure --target-list=riscv64-softmmu
make -j$(nproc)
sudo make install
```

#### 步骤 3: 验证版本

```bash
qemu-system-riscv64 --version
# 应显示: QEMU emulator version 7.x.x
```

#### 步骤 4: 测试运行

```bash
cd /Users/mac/Desktop/project/rcore-lab
bash run.sh
```

---

### 方案 2：修改 QEMU 参数（临时方案）

不降级 QEMU，修改启动参数强制使用 VirtIO legacy 模式。

#### 修改 run.sh

编辑 `/Users/mac/Desktop/project/rcore-lab/run.sh`，找到第 171-182 行的 QEMU 启动命令，修改：

```bash
# 原始（第 178 行）：
-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \

# 修改为（禁用某些新特性）：
-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0,disable-modern=on,disable-legacy=off \
```

完整修改后的命令（第 171-182 行）：

```bash
qemu-system-riscv64 -machine virt \
  -kernel kernel-qemu \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file="$IMAGE_FILE",if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0,disable-modern=on,disable-legacy=off \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
    -netdev "$NETDEV_OPTS" \
        $NET_DUMP_OBJ \
    $GDB_FLAGS
```

或者尝试另一种方式：

```bash
# 使用 scsi 而不是 virtio-blk
-device virtio-scsi-device,id=scsi \
-device scsi-hd,drive=x0 \
```

---

### 方案 3：使用 Docker（最简单，推荐新手）

使用包含正确 QEMU 版本的 Docker 镜像。

#### 创建 Dockerfile

```dockerfile
FROM ubuntu:22.04

# 安装依赖
RUN apt-get update && apt-get install -y \
    build-essential \
    curl \
    git \
    qemu-system-riscv64=1:7.0+dfsg-7ubuntu2 \
    && rm -rf /var/lib/apt/lists/*

# 安装 Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

# 配置 Rust 工具链
RUN rustup default stable && \
    rustup toolchain install nightly-2024-05-02 && \
    rustup target add riscv64gc-unknown-none-elf --toolchain nightly-2024-05-02 && \
    rustup component add rust-src llvm-tools-preview --toolchain nightly-2024-05-02 && \
    cargo install cargo-binutils

WORKDIR /workspace
```

#### 构建和运行

```bash
cd /Users/mac/Desktop/project/rcore-lab

# 构建镜像
docker build -t rcore-qemu7 .

# 运行
docker run --rm -it -v $(pwd):/workspace rcore-qemu7 bash run.sh
```

---

### 方案 4：从 Homebrew 旧版本安装（如果可用）

```bash
# 搜索可用的 QEMU 版本
brew search qemu

# 如果有 qemu@7，直接安装
brew uninstall qemu
brew install qemu@7
brew link qemu@7 --force

# 验证
qemu-system-riscv64 --version
```

---

## 推荐方案比较

| 方案 | 优点 | 缺点 | 难度 |
|------|------|------|------|
| 方案 1（降级）| 完全兼容，最稳定 | 需要卸载新版本 | 中等 |
| 方案 2（参数）| 不需要降级 | 可能不完全解决问题 | 简单 |
| 方案 3（Docker）| 环境隔离，最干净 | 需要 Docker，速度稍慢 | 简单 |
| 方案 4（Homebrew）| 最简单 | 可能没有旧版本 | 最简单 |

## 快速决策指南

1. **如果 Homebrew 有 qemu@7** → 使用方案 4
2. **如果没有，且愿意编译** → 使用方案 1
3. **如果想快速测试** → 先试方案 2
4. **如果都不行** → 使用方案 3（Docker）

## 验证修复

运行以下命令验证：

```bash
cd /Users/mac/Desktop/project/rcore-lab

# 检查 QEMU 版本
qemu-system-riscv64 --version

# 运行 OS
bash run.sh
```

成功的输出应该显示：
```
=== rCore initcode ===
=== Running /musl/basic_testcode.sh ===
[正常的测试输出...]
```

而不是 panic。

---

## 其他注意事项

### 如果需要保留新版 QEMU

可以同时安装多个版本：

```bash
# 安装新版（系统默认）
brew install qemu

# 编译旧版到自定义路径
cd /tmp
wget https://download.qemu.org/qemu-7.2.0.tar.xz
tar xf qemu-7.2.0.tar.xz
cd qemu-7.2.0
./configure --prefix=$HOME/qemu-7.2.0 --target-list=riscv64-softmmu
make -j$(nproc)
make install

# 修改 run.sh 使用特定版本
# 在 run.sh 开头添加：
export PATH="$HOME/qemu-7.2.0/bin:$PATH"
```

### QEMU 版本兼容性表

| QEMU 版本 | 兼容性 | 说明 |
|-----------|--------|------|
| 7.0.x | ✅ 完美 | 项目测试版本 |
| 7.1.x - 7.2.x | ✅ 良好 | 应该可以工作 |
| 8.x | ⚠️ 部分 | 可能需要参数调整 |
| 9.x | ⚠️ 部分 | 可能需要参数调整 |
| 10.x | ❌ 不兼容 | 需要降级或修改驱动 |

---

## 长期解决方案

### 更新 VirtIO 驱动

如果想继续使用新版 QEMU，需要更新项目的 virtio-drivers crate：

1. 检查 `os/Cargo.toml` 中的 virtio-drivers 版本
2. 更新到最新版本
3. 修改驱动代码以适配新 API
4. 测试兼容性

这需要更深入的代码修改，不推荐初学者使用。

---

**创建日期**: 2026-02-12
**最后更新**: 2026-02-12
**推荐方案**: 方案 1 或方案 4（降级 QEMU）
