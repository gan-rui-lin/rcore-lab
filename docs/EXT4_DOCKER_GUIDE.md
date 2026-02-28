# EXT4 Docker 环境使用指南

日期：2026-03-01

## 背景

macOS 原生不支持挂载 ext4 文件系统，导致无法直接访问 Linux 格式的镜像文件（如 `sdcard-final.img`）。为了解决这个问题，我们创建了基于 Docker 的工具链，可以在 macOS 上透明地访问和操作 ext4 镜像。

## 架构设计

### 组件说明

1. **Dockerfile.ext4**: 定义了一个最小化的 Ubuntu 22.04 容器环境
   - 安装了 `e2fsprogs`（ext4 工具集）
   - 包含常用的文件操作工具（vim, less, tree 等）
   - 预创建挂载点 `/mnt/sdcard`

2. **mount-sdcard.sh**: 快速挂载脚本
   - 自动构建 Docker 镜像
   - 以特权模式启动容器（需要 mount 权限）
   - 只读模式挂载镜像文件
   - 提供交互式 shell 访问

3. **ext4-tools.sh**: 高级操作工具
   - 提供多个子命令来操作镜像
   - 支持文件列表、搜索、提取等操作
   - 无需手动进入容器即可快速查看内容

## 使用方法

### 前置要求

1. 确保 Docker Desktop 已安装并运行
2. 确保镜像文件已解压（例如 `sdcard-final.img`）

### 快速开始

#### 方法一：交互式挂载

```bash
# 使用默认镜像文件 sdcard-final.img
./mount-sdcard.sh

# 或指定其他镜像文件
./mount-sdcard.sh path/to/your-image.img
```

进入容器后，镜像已自动挂载到 `/mnt/sdcard`：

```bash
# 查看根目录
ls -lh /mnt/sdcard

# 查看目录树
tree /mnt/sdcard -L 2

# 查看特定目录
ls /mnt/sdcard/musl

# 查找文件
find /mnt/sdcard -name "busybox"

# 查看文件内容
cat /mnt/sdcard/init.sh

# 退出（自动卸载）
exit
```

#### 方法二：使用高级工具

```bash
# 查看帮助
./ext4-tools.sh help

# 列出根目录
./ext4-tools.sh list /

# 列出特定目录
./ext4-tools.sh list /musl

# 显示目录树（深度为 2）
./ext4-tools.sh tree /musl 2

# 查找文件
./ext4-tools.sh find "busybox"
./ext4-tools.sh find "*.sh"

# 查看文件内容
./ext4-tools.sh cat /init.sh

# 提取文件到当前目录
./ext4-tools.sh extract /musl/busybox ./busybox-binary

# 提取整个目录
./ext4-tools.sh extract /musl ./musl-extracted

# 查看文件系统信息
./ext4-tools.sh info

# 检查文件系统完整性
./ext4-tools.sh check

# 打开交互式 shell
./ext4-tools.sh shell
```

### 使用非默认镜像文件

```bash
# 设置环境变量
export IMAGE_FILE=path/to/another-image.img

# 然后使用任何 ext4-tools 命令
./ext4-tools.sh list /
```

## 常见使用场景

### 场景 1：查看 busybox 测试程序

```bash
# 列出所有 busybox 相关文件
./ext4-tools.sh find "busybox"

# 查看 musl 目录下的程序
./ext4-tools.sh list /musl

# 提取 busybox 二进制文件到本地
./ext4-tools.sh extract /musl/busybox ./busybox
```

### 场景 2：查看测试脚本

```bash
# 查找所有 shell 脚本
./ext4-tools.sh find "*.sh"

# 查看特定脚本内容
./ext4-tools.sh cat /testcases/run-all.sh
```

### 场景 3：调试镜像内容

```bash
# 进入交互式环境
./ext4-tools.sh shell

# 在容器内可以执行任何操作
cd /mnt/sdcard
grep -r "some_text" .
find . -type f -executable
```

### 场景 4：批量提取文件

```bash
# 提取整个测试用例目录
./ext4-tools.sh extract /testcases ./testcases-local

# 提取特定的可执行文件
./ext4-tools.sh extract /bin ./bin-backup
```

## 技术细节

### Docker 权限说明

- 使用 `--privileged` 模式是因为 `mount` 操作需要内核权限
- 镜像以只读模式（`:ro`）挂载，保证不会意外修改
- 当前工作目录映射到容器的 `/workspace/host`，方便文件交换

### 挂载选项

- `-o loop`: 将镜像文件作为块设备挂载
- `-o ro`: 只读模式，防止修改

### 文件系统支持

此工具支持以下文件系统格式：
- ext4（主要支持）
- ext3（兼容）
- ext2（兼容）

## 故障排查

### 问题：Docker 未运行

```
Error: Docker is not running
```

**解决方案**：启动 Docker Desktop

### 问题：镜像文件不存在

```
Error: Image file 'xxx' not found
```

**解决方案**：
1. 检查镜像文件路径是否正确
2. 确认镜像文件已解压（`.xz` 文件需要先解压）

```bash
xz -dk sdcard-rv.img.xz
```

### 问题：挂载失败

```
mount: /mnt/sdcard: wrong fs type, bad option, bad superblock...
```

**可能原因**：
1. 镜像文件损坏
2. 不是有效的 ext4 文件系统

**诊断方法**：

```bash
# 检查文件类型
./ext4-tools.sh info

# 检查文件系统完整性
./ext4-tools.sh check
```

### 问题：权限不足

如果遇到权限问题，确保：
1. Docker Desktop 有必要的系统权限
2. 脚本具有执行权限：`chmod +x *.sh`

## 性能优化建议

1. **首次使用**：首次运行会构建 Docker 镜像，需要几分钟时间
2. **后续使用**：镜像已缓存，启动速度很快（秒级）
3. **大文件操作**：提取大文件时，Docker 卷映射可能有性能开销

## 与 rCore-Lab 项目集成

### 更新 CLAUDE.md 配置

在项目的 `CLAUDE.md` 中已经更新了挂载路径说明：

```markdown
## 对应的测试源码

在 macOS 上，由于不支持原生 ext4 挂载，请使用 Docker 工具：

1. 使用 `./ext4-tools.sh list /musl` 查看 busybox 位置
2. 使用 `./ext4-tools.sh shell` 进入交互式环境
3. busybox 源码位于：/Users/mac/Desktop/project/testsuits-for-oskernel/busybox
```

### 替代传统 mount 命令

原命令（在 Linux 上）：
```bash
sudo mount -o loop sdcard-final.img /mnt/sdcard-2025
ls /mnt/sdcard-2025/
```

新命令（在 macOS 上）：
```bash
./ext4-tools.sh list /
./ext4-tools.sh tree / 2
```

## 最佳实践

1. **只读访问**：默认所有操作都是只读的，不会修改镜像文件
2. **快速查看**：优先使用 `ext4-tools.sh` 的命令行工具，速度更快
3. **深度探索**：需要复杂操作时使用 `shell` 子命令进入交互式环境
4. **文件提取**：需要在宿主机上使用文件时，使用 `extract` 命令
5. **自动化**：可以在脚本中调用 `ext4-tools.sh`，适合 CI/CD 流程

## 扩展功能

如果未来需要**修改**镜像内容，可以：

1. 创建读写版本的容器
2. 修改 `mount` 命令去掉 `ro` 选项
3. 在容器内修改后，变更会持久化到镜像文件

示例：
```bash
# 以读写模式挂载（需要移除 :ro 标志）
docker run -it --rm --privileged \
    -v "$(pwd)/sdcard-final.img:/workspace/sdcard.img" \
    rcore-ext4:latest bash

# 在容器内
mount -o loop /workspace/sdcard.img /mnt/sdcard
# 进行修改...
umount /mnt/sdcard
```

⚠️ **警告**：读写模式可能导致数据损坏，使用前请备份镜像文件！

## 总结

通过 Docker 容器化的方式，我们成功解决了 macOS 不支持 ext4 的限制，提供了：

1. ✅ 透明的镜像访问能力
2. ✅ 简单易用的命令行工具
3. ✅ 安全的只读默认模式
4. ✅ 灵活的交互式环境
5. ✅ 与现有工作流的无缝集成

这套工具链可以直接用于 rCore-Lab 项目的开发和调试流程中。
