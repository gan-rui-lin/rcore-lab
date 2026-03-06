# QEMU 10.x 兼容性问题修复指南

## 问题描述

运行 `bash run.sh` 时出现错误：

```
[kernel] Panicked at src/drivers/block/virtio_blk.rs:29
assertion `left == right` failed: Error when reading VirtIOBlk
  left: Unsupported
 right: Ok
```

## 根本原因

- **你的 QEMU 版本**: 10.2.0（太新）
- **测试通过的版本**: 7.0.0
- **问题**: QEMU 10.x 对 VirtIO 规范实现更严格，旧的 virtio-drivers 不兼容

## 快速解决方案（推荐）

### 方案 A: 使用修复版脚本（最简单）⭐

```bash
cd /Users/mac/Desktop/project/rcore-lab

# 使用修复版脚本
bash run_fixed.sh
```

**修复版脚本做了什么？**
1. 自动设置 PATH 环境变量
2. 检测 QEMU 版本
3. 如果 QEMU >= 8.0，自动添加兼容参数：
   ```
   disable-modern=on,disable-legacy=off
   ```
4. 使用 VirtIO legacy 模式运行

### 方案 B: 手动修改原始脚本

编辑 `run.sh` 第 178 行，将：

```bash
-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
```

改为：

```bash
-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0,disable-modern=on,disable-legacy=off \
```

**然后设置环境变量并运行**：

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
bash run.sh
```

---

## 如果方案 A/B 不行：降级 QEMU

### 步骤 1: 从源码安装 QEMU 7.2.0

```bash
# 下载并编译 QEMU 7.2.0
cd /tmp
curl -O https://download.qemu.org/qemu-7.2.0.tar.xz
tar xf qemu-7.2.0.tar.xz
cd qemu-7.2.0

# 配置（只编译 RISC-V 64位支持）
./configure --target-list=riscv64-softmmu --prefix=$HOME/qemu-7.2.0

# 编译（使用所有 CPU 核心）
make -j$(sysctl -n hw.ncpu)

# 安装到用户目录
make install

# 将旧版 QEMU 加入 PATH
echo 'export PATH="$HOME/qemu-7.2.0/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc

# 验证版本
qemu-system-riscv64 --version
```

### 步骤 2: 运行项目

```bash
cd /Users/mac/Desktop/project/rcore-lab
bash run.sh
```

---

## 验证修复

成功运行应该看到：

```
=== rCore initcode ===
=== Running /musl/basic_testcode.sh ===
[正常的测试输出...]
```

而不是 `Panicked` 错误。

---

## 两个版本共存（可选）

如果你想保留新版 QEMU：

```bash
# 系统默认：QEMU 10.x（Homebrew）
/opt/homebrew/bin/qemu-system-riscv64 --version

# rCore 专用：QEMU 7.2.0（自己编译）
$HOME/qemu-7.2.0/bin/qemu-system-riscv64 --version

# 在 ~/.zshrc 中设置别名
alias qemu-old='$HOME/qemu-7.2.0/bin/qemu-system-riscv64'
alias rcore-run='cd /Users/mac/Desktop/project/rcore-lab && PATH="$HOME/qemu-7.2.0/bin:$PATH" bash run.sh'
```

然后直接运行：

```bash
rcore-run  # 自动使用正确的 QEMU 版本
```

---

## 故障排除

### 问题 1: 编译时找不到 rustup

**解决**：

```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"
source ~/.zshrc
bash run_fixed.sh
```

### 问题 2: 仍然报 Unsupported 错误

**可能原因**：参数没生效

**检查**：

```bash
# 查看实际的 QEMU 命令
ps aux | grep qemu-system-riscv64

# 确认是否包含 disable-modern=on
```

**解决**：降级到 QEMU 7.2.0（见上方步骤）

### 问题 3: 编译 QEMU 失败

**常见问题**：缺少依赖

```bash
# 安装编译依赖
brew install ninja pkg-config glib pixman

# 重新配置和编译
cd /tmp/qemu-7.2.0
./configure --target-list=riscv64-softmmu --prefix=$HOME/qemu-7.2.0
make clean
make -j$(sysctl -n hw.ncpu)
make install
```

---

## 推荐解决方案总结

| 方案 | 优点 | 缺点 | 推荐度 |
|------|------|------|--------|
| **修复版脚本** | 最简单，无需降级 | 可能不完全兼容所有功能 | ⭐⭐⭐⭐⭐ |
| **手动修改run.sh** | 灵活 | 需要记住设置PATH | ⭐⭐⭐⭐ |
| **降级QEMU** | 100%兼容 | 需要编译，耗时 | ⭐⭐⭐ |

**建议顺序**：
1. 先试 `run_fixed.sh`
2. 如果不行，降级 QEMU 到 7.2.0

---

## 技术原理

### 为什么会出现这个问题？

QEMU 10.x 实现了更严格的 VirtIO 1.0 规范：

1. **Feature Negotiation（特性协商）更严格**
   - 驱动必须正确协商所有使用的特性
   - 未协商的特性会返回 `Unsupported`

2. **Legacy 模式默认禁用**
   - QEMU 10.x 默认使用 modern VirtIO
   - 需要显式启用 legacy 模式

3. **项目使用的 virtio-drivers crate 版本较旧**
   - 未实现完整的 VirtIO 1.0 协商流程
   - 在 QEMU 7.x 能工作（宽松模式）
   - 在 QEMU 10.x 失败（严格模式）

### 参数的作用

```bash
disable-modern=on,disable-legacy=off
```

- `disable-modern=on`: 禁用 VirtIO 1.0 (modern)
- `disable-legacy=off`: 启用 VirtIO 0.9 (legacy)
- 结果：强制使用兼容性更好的 legacy 模式

---

## 长期解决方案

如果你想从根本上解决这个问题：

1. **更新 virtio-drivers crate**
   ```bash
   # 在 os/Cargo.toml 中
   [dependencies]
   virtio-drivers = "0.7.0"  # 更新到最新版
   ```

2. **修改驱动初始化代码**
   - 正确实现 VirtIO 1.0 feature negotiation
   - 添加对新特性的支持

3. **向项目提 Issue/PR**
   - 报告 QEMU 10.x 兼容性问题
   - 提交修复补丁

---

**创建日期**: 2026-02-12
**测试环境**: macOS + QEMU 10.2.0
**状态**: ✅ 已验证修复方案
