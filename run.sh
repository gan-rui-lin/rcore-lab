#!/bin/bash

# Set PATH to prioritize rustup nightly-2024-05-02 toolchain
export PATH="$HOME/.rustup/toolchains/nightly-2024-05-02-aarch64-apple-darwin/bin:$HOME/.rustup/toolchains/nightly-2024-05-02-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin:$PATH"

# 默认配置
BUILD_TYPE="rv"
IMAGE_FILE="sdcard-rv.img"
GDB_DEBUG="0"
GDB_FLAGS=""
LOG="${LOG-}"
NET_FORWARD="0"
NET_DUMP_FILE=""
NET_DUMP_OBJ=""
NET_MODE="user"
TAP_IFNAME="tap0"
BRIDGE_NAME="br0"
OFFLINE_BUILD="${OFFLINE-1}"

# 显示用法信息
usage() {
    echo "用法: $0 [选项]"
    echo "选项:"
    echo "  -t, --type TYPE    构建类型 (rv/all/debug), 默认: $BUILD_TYPE (all 等同于 rv)"
    echo "  -f, --file FILE    镜像文件名, 默认: $IMAGE_FILE"
    echo "  -d                 启用 GDB 调试 (为 QEMU 添加 -s -S)"
    echo "  -n, --netforward   启用 user net hostfwd (UDP 12345)"
    echo "  --netdump FILE     启用 QEMU 抓包到 FILE (filter-dump)"
    echo "  --netmode MODE     网络模式 (user/tap), 默认: $NET_MODE"
    echo "  --tap-ifname NAME  tap 模式网卡名, 默认: $TAP_IFNAME"
    echo "  --bridge NAME      bridge 模式桥接名(需预先创建), 默认: $BRIDGE_NAME"
    echo "  --offline          强制离线构建 (默认开启)"
    echo "  --online           允许联网构建"
    echo "  -h, --help         显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0 -t debug -f sdcard-final.img"
    echo "  $0 --type rv --file sdcard.img"
    echo "  $0 -t debug -f sdcard-final.img -d"
    echo "  $0 -t debug -f sdcard-final.img --netforward"
    echo "  $0 -t debug -f sdcard-final.img --netmode tap --tap-ifname tap0"
    echo "  $0 -t debug -f sdcard-final.img --netmode bridge --bridge br0 --tap-ifname tap0"
}

# 解析命令行参数
while [[ $# -gt 0 ]]; do
    case $1 in
        -t|--type)
            BUILD_TYPE="$2"
            shift 2
            ;;
        -f|--file)
            IMAGE_FILE="$2"
            shift 2
            ;;
        -d)
            GDB_DEBUG="1"
            shift
            ;;
        -n|--netforward)
            NET_FORWARD="1"
            shift
            ;;
        --netmode)
            NET_MODE="$2"
            shift 2
            ;;
        --tap-ifname)
            TAP_IFNAME="$2"
            shift 2
            ;;
        --bridge)
            BRIDGE_NAME="$2"
            shift 2
            ;;
        --netdump)
            NET_DUMP_FILE="$2"
            shift 2
            ;;
        --offline)
            OFFLINE_BUILD="1"
            shift
            ;;
        --online)
            OFFLINE_BUILD="0"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "错误: 未知选项 $1"
            usage
            exit 1
            ;;
    esac
done

# 验证构建类型
if [[ "$BUILD_TYPE" != "debug" && "$BUILD_TYPE" != "all" && "$BUILD_TYPE" != "rv" ]]; then
    echo "错误: 构建类型必须是 'rv'、'debug' 或 'all'"
    exit 1
fi

if [[ "$BUILD_TYPE" == "all" ]]; then
    BUILD_TYPE="rv"
fi

# 默认日志级别：debug 显示全部日志，其他关闭日志
if [[ -z "$LOG" ]]; then
    if [[ "$BUILD_TYPE" == "debug" ]]; then
        LOG="TRACE"
    else
        LOG="OFF"
    fi
fi

# 验证镜像文件存在，或者尝试解压 .xz 文件
if [[ ! -f "$IMAGE_FILE" ]]; then
    # 检查是否存在对应的 .xz 文件
    if [[ -f "${IMAGE_FILE}.xz" ]]; then
        echo "发现压缩文件 ${IMAGE_FILE}.xz，正在解压..."
        xz -d -k "${IMAGE_FILE}.xz"
        if [[ ! -f "$IMAGE_FILE" ]]; then
            echo "错误: 解压失败"
            exit 1
        fi
        echo "解压完成: $IMAGE_FILE"
    else
        echo "错误: 镜像文件 '$IMAGE_FILE' 和 '${IMAGE_FILE}.xz' 都不存在"
        exit 1
    fi
fi

echo "开始构建: make $BUILD_TYPE"
echo "使用镜像: $IMAGE_FILE"
if [[ "$GDB_DEBUG" == "1" ]]; then
    GDB_FLAGS="-s -S"
    echo "GDB 调试: 开启 (-s -S)"
else
    echo "GDB 调试: 关闭"
fi

NETDEV_OPTS=""
if [[ "$NET_MODE" == "bridge" ]]; then
    NETDEV_OPTS="tap,id=net,ifname=${TAP_IFNAME},script=no,downscript=no"
    echo "NET 模式: bridge (ifname=${TAP_IFNAME}, bridge=${BRIDGE_NAME})"
    echo "提示: 需要你先在宿主机创建 ${TAP_IFNAME} 并加入 ${BRIDGE_NAME}"
    if [[ "$NET_FORWARD" == "1" ]]; then
        echo "NET 转发: bridge 模式下忽略 hostfwd"
    fi
elif [[ "$NET_MODE" == "tap" ]]; then
    NETDEV_OPTS="tap,id=net,ifname=${TAP_IFNAME},script=no,downscript=no"
    echo "NET 模式: tap (ifname=${TAP_IFNAME})"
    if [[ "$NET_FORWARD" == "1" ]]; then
        echo "NET 转发: tap 模式下忽略 hostfwd"
    fi
else
    NETDEV_OPTS="user,id=net"
    if [[ "$NET_FORWARD" == "1" ]]; then
        NETDEV_OPTS="user,id=net,hostfwd=udp::12345-:12345"
        echo "NET 模式: user"
        echo "NET 转发: 开启 (udp 12345 -> guest 12345)"
    else
        echo "NET 模式: user"
        echo "NET 转发: 关闭"
    fi
fi
if [[ -n "$NET_DUMP_FILE" ]]; then
    NET_DUMP_OBJ="-object filter-dump,id=netdump,netdev=net,file=${NET_DUMP_FILE}"
    echo "NET 抓包: ${NET_DUMP_FILE}"
fi
if [[ "$OFFLINE_BUILD" == "1" ]]; then
    echo "离线构建: 开启 (OFFLINE=1, CARGO_NET_OFFLINE=true)"
else
    echo "离线构建: 关闭"
fi

# 执行构建[1](@ref)
if [[ "$BUILD_TYPE" == "debug" ]]; then
    if [[ "$OFFLINE_BUILD" == "1" ]]; then
        BUILD_CMD="LOG=$LOG OFFLINE=1 CARGO_NET_OFFLINE=true make debug"
    else
        BUILD_CMD="LOG=$LOG make debug"
    fi
    if eval "$BUILD_CMD"; then
        echo "构建成功!"
    else
        echo "错误: 构建失败"
        exit 1
    fi
else
    if [[ "$OFFLINE_BUILD" == "1" ]]; then
        BUILD_CMD="LOG=$LOG OFFLINE=1 CARGO_NET_OFFLINE=true make $BUILD_TYPE"
    else
        BUILD_CMD="LOG=$LOG make $BUILD_TYPE"
    fi
    if eval "$BUILD_CMD"; then
        echo "构建成功!"
    else
        echo "错误: 构建失败"
        exit 1
    fi
fi

EXTRA_DISK_ARGS=""
if [[ -f "disk.img" ]]; then
    EXTRA_DISK_ARGS="-drive file=disk.img,if=none,format=raw,id=x1 -device virtio-blk-device,drive=x1,bus=virtio-mmio-bus.1"
    echo "附加磁盘: disk.img"
fi

# 运行QEMU
echo "启动QEMU模拟器..."
KERNEL_IMAGE="kernel-rv.bin"
if [[ ! -f "$KERNEL_IMAGE" ]]; then
    KERNEL_IMAGE="kernel-rv"
fi
qemu-system-riscv64 -machine virt \
    -kernel "$KERNEL_IMAGE" \
  -m 1024M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file="$IMAGE_FILE",if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -no-reboot \
    -device virtio-net-device,netdev=net \
    -netdev "$NETDEV_OPTS" \
    -rtc base=utc \
    $EXTRA_DISK_ARGS \
    $NET_DUMP_OBJ \
    $GDB_FLAGS

echo "QEMU已退出"

# sudo ip tuntap add dev tap0 mode tap user $USER
# sudo ip link set tap0 up
# sudo ip link add br0 type bridge
# sudo ip link set br0 up
# sudo ip link set tap0 master br0
# sudo ip addr add 10.0.2.2/24 dev br0
# sudo ip link set br0 up

# nc -k -l 10.0.2.2 12346
# nc -u -l 12345  
