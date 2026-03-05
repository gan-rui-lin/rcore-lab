# macOS 下通过 Docker 操作 ext4 镜像指南

日期：2026/3/6

## 背景

macOS 原生不支持 ext4 文件系统。`hdiutil attach` 只能挂载 HFS+/APFS/FAT 等 macOS 原生格式，对 ext4 会报"无可装载的文件系统"。而 rCore 的 `sdcard-rv.img` 是 ext4 格式，需要查看/修改其中的文件（比如确认 `entry-dynamic.exe` 是否存在、检查 PT_INTERP 等）。

解决方案是利用 Docker 容器内的 Linux 内核来挂载 ext4。Docker Desktop for Mac（或 colima）底层跑的是一个 Linux VM，天然支持 ext4。

## 原理

```
macOS (不支持 ext4)
  └── colima / Docker Desktop (Linux VM)
       └── Docker 容器 (ubuntu:22.04)
            └── mount -o loop sdcard.img /mnt  ← Linux 内核处理 ext4
```

关键点：
- Docker 的 `-v` 参数把宿主机的 `.img` 文件映射到容器内
- 容器内用 `mount -o loop` 把镜像文件当块设备挂载
- `--privileged` 赋予容器挂载权限（普通容器没有 `CAP_SYS_ADMIN`）
- loop device 由 Linux 内核自动分配（`/dev/loop0` 等）

## 前置准备

### 1. 安装 colima（轻量级 Docker runtime）

```bash
brew install colima docker
```

> colima 比 Docker Desktop 轻量得多，不需要 GUI，适合命令行工作流。

### 2. 启动 colima

```bash
colima start
```

启动后会自动配置 Docker socket，`docker` 命令即可使用。

### 3. 构建工具镜像

项目根目录已有 `Dockerfile.ext4`：

```dockerfile
FROM ubuntu:22.04
RUN apt-get update && apt-get install -y \
    e2fsprogs \    # ext4 工具集（e2fsck, dumpe2fs, debugfs 等）
    xz-utils \     # 解压 .xz
    vim less \     # 查看文件
    tree \         # 目录树
    file \         # 文件类型检测
    kmod           # 内核模块工具
RUN mkdir -p /mnt/sdcard
WORKDIR /workspace
```

构建：

```bash
docker build -f Dockerfile.ext4 -t rcore-ext4:latest .
```

## 常用操作

### 查看镜像信息

```bash
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img" \
  rcore-ext4:latest \
  bash -c "file /workspace/sdcard.img && echo '---' && dumpe2fs /workspace/sdcard.img 2>/dev/null | head -20"
```

输出示例：
```
/workspace/sdcard.img: Linux rev 1.0 ext4 filesystem data, UUID=362039d9-...
---
Filesystem volume name:   <none>
Filesystem magic number:  0xEF53
Inode count:              262144
Block count:              1048576
Free blocks:              235780
...
```

### 修复文件系统（重要！）

QEMU 运行后镜像通常处于 dirty 状态（`needs journal recovery`），必须先修复才能挂载：

```bash
# 注意：这里不能用 :ro，因为 fsck 需要写入
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img" \
  rcore-ext4:latest \
  bash -c "e2fsck -y /workspace/sdcard.img"
```

`-y` 自动回答 yes。典型输出：
```
/workspace/sdcard.img: recovering journal
/workspace/sdcard.img: ***** FILE SYSTEM WAS MODIFIED *****
/workspace/sdcard.img: 6397/262144 files, 778146/1048576 blocks
```

### 列出目录

```bash
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img:ro" \
  rcore-ext4:latest \
  bash -c "mount -o loop,ro /workspace/sdcard.img /mnt/sdcard && \
           ls -la /mnt/sdcard/musl/ && \
           umount /mnt/sdcard"
```

### 查找文件

```bash
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img:ro" \
  rcore-ext4:latest \
  bash -c "mount -o loop,ro /workspace/sdcard.img /mnt/sdcard && \
           find /mnt/sdcard -name 'entry-dynamic*' && \
           umount /mnt/sdcard"
```

### 检查 ELF 文件信息（readelf）

需要安装 binutils：

```bash
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img:ro" \
  rcore-ext4:latest \
  bash -c "apt-get update -qq && apt-get install -y -qq binutils > /dev/null 2>&1 && \
           mount -o loop,ro /workspace/sdcard.img /mnt/sdcard && \
           readelf -l /mnt/sdcard/musl/entry-dynamic.exe | grep -A2 INTERP && \
           file /mnt/sdcard/musl/entry-dynamic.exe && \
           umount /mnt/sdcard"
```

这条命令揭示了 dynamic 测试全部 FAIL 的根因：
```
interpreter /lib/ld-musl-riscv64-sf.so.1
```
PT_INTERP 指向 `/lib/ld-musl-riscv64-sf.so.1`，但 sdcard 上没有这个路径的文件。

### 从镜像提取文件到宿主机

```bash
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img:ro" \
  -v "$(pwd):/workspace/host" \
  rcore-ext4:latest \
  bash -c "mount -o loop,ro /workspace/sdcard.img /mnt/sdcard && \
           cp /mnt/sdcard/musl/entry-dynamic.exe /workspace/host/ && \
           umount /mnt/sdcard"
```

提取后可以在 macOS 上用交叉工具链分析：
```bash
riscv64-unknown-elf-readelf -l entry-dynamic.exe
riscv64-unknown-elf-objdump -d entry-dynamic.exe | head -50
```

### 写入文件到镜像

```bash
# 注意：不能用 :ro
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img" \
  -v "$(pwd):/workspace/host" \
  rcore-ext4:latest \
  bash -c "mount -o loop /workspace/sdcard.img /mnt/sdcard && \
           cp /workspace/host/some-file /mnt/sdcard/musl/ && \
           sync && \
           umount /mnt/sdcard"
```

### 交互式 shell

最灵活的方式，直接进入挂载好的环境：

```bash
docker run -it --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img" \
  -v "$(pwd):/workspace/host" \
  rcore-ext4:latest \
  bash -c "mount -o loop /workspace/sdcard.img /mnt/sdcard && \
           echo '✓ Mounted at /mnt/sdcard' && \
           cd /mnt/sdcard && \
           exec bash"
```

进入后可以随意 `ls`、`cat`、`cp`、`find`。退出时记得：
```bash
cd / && umount /mnt/sdcard && exit
```

## 封装脚本：ext4-tools.sh

项目根目录的 `ext4-tools.sh` 封装了上述操作：

```bash
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh list /musl        # 列目录
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh find "*.so"        # 查找
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh cat /musl/run-dynamic.sh  # 查看内容
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh tree /musl 2       # 目录树
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh info               # 文件系统信息
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh shell              # 交互 shell
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh check              # fsck 检查
```

## 关键参数解释

| 参数 | 作用 |
|------|------|
| `--rm` | 容器退出后自动删除，不留垃圾 |
| `--privileged` | 赋予所有 Linux capabilities，允许 `mount` |
| `-v host:container` | 绑定挂载宿主机文件到容器 |
| `:ro` | 只读挂载，防止意外修改镜像 |
| `-it` | 分配 TTY + stdin，用于交互式 shell |
| `mount -o loop` | loop device 挂载，把普通文件当块设备 |
| `mount -o loop,ro` | 只读 loop 挂载 |

## 注意事项

1. **QEMU 运行后必须 fsck**：QEMU 直接操作 raw 镜像，退出时不保证 journal 干净。不 fsck 直接挂载会报 `Structure needs cleaning`。

2. **不要同时挂载和运行 QEMU**：两个进程同时写同一个 raw 镜像会导致数据损坏。

3. **`:ro` 很重要**：只需要查看时务必加 `:ro`，既保护数据又避免 fsck 需求。

4. **`sync` 很重要**：写入后在 `umount` 前调用 `sync`，确保数据刷到镜像文件。

## 实际案例：诊断 entry-dynamic.exe FAIL

完整的诊断流程：

```bash
# 1. 先修复镜像
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh check

# 2. 确认文件存在
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh find "entry-dynamic*"
# 输出: /mnt/sdcard/musl/entry-dynamic.exe  ← 文件存在！

# 3. 检查 PT_INTERP
docker run --rm --privileged \
  -v "$(pwd)/sdcard-rv.img:/workspace/sdcard.img:ro" \
  rcore-ext4:latest \
  bash -c "apt-get update -qq && apt-get install -y -qq binutils >/dev/null 2>&1 && \
           mount -o loop,ro /workspace/sdcard.img /mnt/sdcard && \
           file /mnt/sdcard/musl/entry-dynamic.exe && \
           umount /mnt/sdcard"
# 输出: interpreter /lib/ld-musl-riscv64-sf.so.1  ← 关键线索！

# 4. 确认 loader 不在预期路径
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh find "ld-musl*"
# 输出: (空)  ← 没有这个文件！

# 5. 找到实际 loader 位置
IMAGE_FILE=sdcard-rv.img ./ext4-tools.sh find "libc.so*"
# 输出: /mnt/sdcard/musl/lib/libc.so  ← musl loader 实际在这里
```

结论：内核需要在启动时创建 `/lib/ld-musl-riscv64-sf.so.1` → `/musl/lib/libc.so` 的硬链接。修复后 110 个 dynamic 测试全部通过。
