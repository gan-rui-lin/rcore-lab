#!/bin/bash

# Default configuration
BUILD_TYPE="la"
KERNEL_FILE="kernel-la"
IMAGE_FILE="sdcard-la.img"
# Keep data disk optional by default. For la/rv parity we only require sdcard-la.img.
DATA_DISK="${DATA_DISK-}"
MEMORY="1G"
SMP="1"
GDB_DEBUG="0"
LOG="${LOG-}"
NET_FORWARD="0"

usage() {
    echo "Usage: $0 [options]"
    echo "Options:"
    echo "  -t, --type TYPE      build type (all/debug), default: $BUILD_TYPE"
    echo "  -f, --file FILE      root fs image, default: $IMAGE_FILE"
    echo "  --data-disk FILE     extra data disk image (optional)"
    echo "  --no-data-disk       disable extra data disk"
    echo "  -m, --mem SIZE       memory size, default: $MEMORY"
    echo "  -s, --smp N          smp cores, default: $SMP"
    echo "  -d                   enable GDB (-s -S)"
    echo "  -n, --netforward     enable hostfwd (tcp/udp 5555)"
    echo "  -h, --help           show help"
    echo ""
    echo "Examples:"
    echo "  $0 -f sdcard-la.img"
    echo "  $0 -t debug -d"
}

# Parse args
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
        --data-disk)
            DATA_DISK="$2"
            shift 2
            ;;
        --no-data-disk)
            DATA_DISK=""
            shift
            ;;
        -m|--mem)
            MEMORY="$2"
            shift 2
            ;;
        -s|--smp)
            SMP="$2"
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
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Error: unknown option $1"
            usage
            exit 1
            ;;
    esac
done

if [[ -z "$LOG" ]]; then
    if [[ "$BUILD_TYPE" == "debug" ]]; then
        LOG="TRACE"
    else
        LOG="OFF"
    fi
fi

if [[ ! -f "$IMAGE_FILE" ]]; then
    if [[ -f "${IMAGE_FILE}.xz" ]]; then
        echo "Found ${IMAGE_FILE}.xz, extracting..."
        xz -d -k "${IMAGE_FILE}.xz"
    else
        echo "Error: image file '$IMAGE_FILE' not found"
        exit 1
    fi
fi


GDB_FLAGS=""
if [[ "$GDB_DEBUG" == "1" ]]; then
    GDB_FLAGS="-s -S"
    echo "GDB: enabled (-s -S)"
else
    echo "GDB: disabled"
fi

NETDEV_OPTS="user,id=net0"
if [[ "$NET_FORWARD" == "1" ]]; then
    NETDEV_OPTS="user,id=net0,hostfwd=tcp::5555-:5555,hostfwd=udp::5555-:5555"
    echo "NET forward: enabled (tcp/udp 5555)"
else
    echo "NET forward: disabled"
fi

if [[ "$BUILD_TYPE" == "debug" ]]; then
    echo "Build: make la MODE=debug"
    if ! LOG="$LOG" make la MODE=debug; then
        echo "Error: build failed"
        exit 1
    fi
elif [[ "$BUILD_TYPE" == "la" || "$BUILD_TYPE" == "all" ]]; then
    echo "Build: make $BUILD_TYPE"
    if ! LOG="$LOG" make "$BUILD_TYPE"; then
        echo "Error: build failed"
        exit 1
    fi
else
    echo "Error: build type must be all/debug"
    exit 1
fi

if [[ ! -f "$KERNEL_FILE" ]]; then
    echo "Error: kernel file '$KERNEL_FILE' not found after build"
    exit 1
fi

QEMU_CMD=(
    qemu-system-loongarch64
    -kernel "$KERNEL_FILE"
    -m "$MEMORY"
    -nographic
    -smp "$SMP"
    -drive file="$IMAGE_FILE",if=none,format=raw,id=x0
    -device virtio-blk-pci,drive=x0
    -no-reboot
    -device virtio-net-pci,netdev=net0
    -netdev "$NETDEV_OPTS"
    -rtc base=utc
)

if [[ -n "$DATA_DISK" ]]; then
    if [[ ! -f "$DATA_DISK" ]]; then
        echo "Warning: data disk '$DATA_DISK' not found, skipping"
    else
        QEMU_CMD+=(
            -drive file="$DATA_DISK",if=none,format=raw,id=x1
            -device virtio-blk-pci,drive=x1
        )
    fi
fi

if [[ -n "$GDB_FLAGS" ]]; then
    QEMU_CMD+=( $GDB_FLAGS )
fi

if ! command -v "${QEMU_CMD[0]}" >/dev/null 2>&1; then
    if [[ -x "$HOME/.local/bin/qemu-system-loongarch64" ]]; then
        QEMU_CMD[0]="$HOME/.local/bin/qemu-system-loongarch64"
    elif [[ -x "$HOME/.local/qemu-9.2.1/bin/qemu-system-loongarch64" ]]; then
        QEMU_CMD[0]="$HOME/.local/qemu-9.2.1/bin/qemu-system-loongarch64"
    else
        echo "Error: qemu-system-loongarch64 not found in PATH."
        echo "Hint: export PATH=\"\$HOME/.local/bin:\$PATH\""
        exit 1
    fi
fi

echo "Starting QEMU..."
"${QEMU_CMD[@]}"

echo "QEMU exited"
