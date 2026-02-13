#!/bin/bash

# Set PATH to prioritize rustup nightly-2024-05-02 toolchain
export PATH="$HOME/.rustup/toolchains/nightly-2024-05-02-aarch64-apple-darwin/bin:$HOME/.rustup/toolchains/nightly-2024-05-02-aarch64-apple-darwin/lib/rustlib/aarch64-apple-darwin/bin:$PATH"

# 默认配置
BUILD_TYPE="debug"
IMAGE_FILE="sdcard-final.img"
GDB_DEBUG="0"
GDB_FLAGS=""
NET_FORWARD="0"
NET_DUMP_FILE=""
NET_DUMP_OBJ=""
NET_MODE="user"
TAP_IFNAME="tap0"
BRIDGE_NAME="br0"

# 显示用法信息
usage() {
    echo "用法: $0 [选项]"
    echo "选项:"
    echo "  -t, --type TYPE    构建类型 (debug/all), 默认: $BUILD_TYPE"
    echo "  -f, --file FILE    镜像文件名, 默认: $IMAGE_FILE"
    echo "  -d                 启用 GDB 调试 (为 QEMU 添加 -s -S)"
    echo "  -n, --netforward   启用 user net hostfwd (UDP 12345)"
    echo "  --netdump FILE     启用 QEMU 抓包到 FILE (filter-dump)"
    echo "  --netmode MODE     网络模式 (user/tap), 默认: $NET_MODE"
    echo "  --tap-ifname NAME  tap 模式网卡名, 默认: $TAP_IFNAME"
    echo "  --bridge NAME      bridge 模式桥接名(需预先创建), 默认: $BRIDGE_NAME"
    echo "  -h, --help         显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  $0 -t debug -f sdcard-final.img"
    echo "  $0 --type all --file sdcard.img"
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
if [[ "$BUILD_TYPE" != "debug" && "$BUILD_TYPE" != "all" ]]; then
    echo "错误: 构建类型必须是 'debug' 或 'all'"
    exit 1
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

# 执行构建[1](@ref)
if [[ "$BUILD_TYPE" == "debug" ]]; then
    if [[ -n "$LOG" ]]; then
        if LOG="$LOG" make debug; then
            echo "构建成功!"
        else
            echo "错误: 构建失败"
            exit 1
        fi
    elif LOG=TRACE make debug; then
        echo "构建成功!"
    else
        echo "错误: 构建失败"
        exit 1
    fi
elif make "$BUILD_TYPE"; then
    echo "构建成功!"
else
    echo "错误: 构建失败"
    exit 1
fi

# 运行QEMU
echo "启动QEMU模拟器..."
qemu-system-riscv64 -machine virt \
  -kernel kernel-qemu \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file="$IMAGE_FILE",if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
    -netdev "$NETDEV_OPTS" \
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
