# LoongArch64 QEMU 运行问题

## 问题描述

LoongArch64 内核编译成功，但 QEMU 运行时 virtio 块设备无法挂载。

## 错误信息历史

### 尝试 1: virtio-blk-device with bus
```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel target/loongarch64-unknown-none/release/os \
    -drive file=../user/target/loongarch64-unknown-none/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0
```

**错误**: `Bus 'virtio-mmio-bus.0' not found`

### 尝试 2: virtio-blk-device without bus
```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel target/loongarch64-unknown-none/release/os \
    -drive file=../user/target/loongarch64-unknown-none/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-device,drive=x0
```

**错误**: `A 'virtio-bus' bus was found but is full`

### 尝试 3: virtio-blk-pci (当前)
```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel target/loongarch64-unknown-none/release/os \
    -drive file=../user/target/loongarch64-unknown-none/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-pci,drive=x0
```

**错误**: （待测试）

## 环境信息

- **QEMU 版本**: 10.2.0
- **系统**: macOS (Darwin 24.6.0)
- **架构**: aarch64 (Apple Silicon)
- **Rust 工具链**: nightly-2024-05-02

## 可能原因

1. **MMIO vs PCI 总线**: LoongArch64 virt 机器可能使用 PCI 总线而非 MMIO
2. **设备树配置**: 可能需要提供设备树 blob
3. **QEMU 版本问题**: LoongArch64 支持可能需要特定 QEMU 版本或编译选项
4. **内核驱动**: 内核的 virtio 驱动初始化可能只支持 MMIO，未启用 PCI

## 待尝试方案

### 方案 A: 使用 virtio-blk-pci
已在 `os/Makefile` 中实现，待测试结果。

### 方案 B: 禁用 virtio，使用简单的内存文件系统
```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel target/loongarch64-unknown-none/release/os
```
需要修改内核初始化代码以支持无 virtio 模式。

### 方案 C: 检查 QEMU 支持的设备
```bash
qemu-system-loongarch64 -machine virt -device help
```

### 方案 D: 使用不同的机器类型
```bash
qemu-system-loongarch64 -machine help
```
查看是否有其他可用的机器类型。

### 方案 E: 添加 PCI 总线参数
```bash
qemu-system-loongarch64 \
    -machine virt \
    -cpu la464-loongarch-cpu \
    -m 128M \
    -nographic \
    -kernel target/loongarch64-unknown-none/release/os \
    -device pcie-root-port,id=pcie.1 \
    -drive file=../user/target/loongarch64-unknown-none/release/fs.img,if=none,format=raw,id=x0 \
    -device virtio-blk-pci,drive=x0,bus=pcie.1
```

## 内核侧修改

如果问题出在内核驱动，需要检查：

1. **virtio 驱动初始化**: `os/src/drivers/` 或 `vendor/virtio-drivers-old/`
2. **PCI 支持**: LoongArch64 架构是否启用了 PCI 总线支持
3. **设备树解析**: 是否正确解析 QEMU 提供的设备信息

## 当前状态

- ✅ 代码重构完成（riscv64 + loongarch64）
- ✅ riscv64 编译通过并可运行
- ✅ loongarch64 编译通过
- ⚠️  loongarch64 QEMU 运行被阻塞

## 负责人

- zjy

## 更新日期

2026-03-04
