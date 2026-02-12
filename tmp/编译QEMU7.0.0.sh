#!/bin/bash

# =============================================
# QEMU 7.0.0 编译脚本
# 在 conda 环境中运行
# =============================================

set -e  # 遇到错误立即退出

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}======================================="
echo "  QEMU 7.0.0 编译脚本"
echo "  (conda 环境版本)"
echo -e "=======================================${NC}"
echo

# 检查是否在 conda 环境中
if [ -z "$CONDA_DEFAULT_ENV" ]; then
    echo -e "${RED}❌ 错误: 未检测到 conda 环境${NC}"
    echo "请先激活 conda 环境："
    echo "  conda activate os"
    exit 1
fi

echo -e "${GREEN}✅ 当前 conda 环境: $CONDA_DEFAULT_ENV${NC}"
echo -e "${GREEN}✅ Python 版本: $(python --version)${NC}"
echo

# 工作目录
QEMU_VERSION="7.0.0"
QEMU_DIR="/Users/mac/Desktop/project/rcore-lab/tmp/qemu-${QEMU_VERSION}"
INSTALL_PREFIX="$HOME/qemu-${QEMU_VERSION}"

# 检查 QEMU 源码目录
if [ ! -d "$QEMU_DIR" ]; then
    echo -e "${RED}❌ 错误: QEMU 源码目录不存在: $QEMU_DIR${NC}"
    echo "请先下载并解压 QEMU 7.0.0"
    exit 1
fi

echo -e "${YELLOW}[1/7]${NC} 检查 Python 依赖..."
# 安装 setuptools（修复 distutils 问题）
pip install setuptools -q || pip install setuptools --break-system-packages -q
echo -e "${GREEN}✅ setuptools 已安装${NC}"

echo -e "${YELLOW}[2/7]${NC} 安装系统依赖..."
# 检查并安装编译依赖
for pkg in ninja pkg-config glib pixman; do
    if ! brew list $pkg &>/dev/null; then
        echo "  安装 $pkg..."
        brew install $pkg
    else
        echo "  ✓ $pkg 已安装"
    fi
done

echo -e "${YELLOW}[3/7]${NC} 进入 QEMU 源码目录..."
cd "$QEMU_DIR"
echo "  当前目录: $(pwd)"

echo -e "${YELLOW}[4/7]${NC} 清理之前的构建..."
if [ -d "build" ]; then
    rm -rf build
    echo "  已清理旧的 build 目录"
fi

echo -e "${YELLOW}[5/7]${NC} 配置构建..."
echo "  目标: riscv64-softmmu"
echo "  安装位置: $INSTALL_PREFIX"

./configure \
    --target-list=riscv64-softmmu \
    --prefix="$INSTALL_PREFIX" \
    --disable-sdl \
    --disable-gtk \
    --disable-vnc

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ 配置失败${NC}"
    echo
    echo "可能的问题："
    echo "1. Python 依赖问题 - 尝试："
    echo "   pip install setuptools --upgrade"
    echo
    echo "2. 缺少系统依赖 - 尝试："
    echo "   brew install ninja pkg-config glib pixman"
    exit 1
fi

echo -e "${GREEN}✅ 配置成功${NC}"

echo -e "${YELLOW}[6/7]${NC} 开始编译（这可能需要 10-20 分钟）..."
CPU_COUNT=$(sysctl -n hw.ncpu)
echo "  使用 $CPU_COUNT 个 CPU 核心"

make -j$CPU_COUNT

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ 编译失败${NC}"
    exit 1
fi

echo -e "${GREEN}✅ 编译成功${NC}"

echo -e "${YELLOW}[7/7]${NC} 安装到 $INSTALL_PREFIX..."
make install

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ 安装失败${NC}"
    exit 1
fi

echo
echo -e "${GREEN}======================================="
echo "  ✅ QEMU 7.0.0 安装成功！"
echo -e "=======================================${NC}"
echo
echo "安装位置: $INSTALL_PREFIX"
echo "可执行文件: $INSTALL_PREFIX/bin/qemu-system-riscv64"
echo
echo -e "${YELLOW}下一步：${NC}"
echo
echo "1. 添加到 PATH (在 ~/.zshrc 中):"
echo "   export PATH=\"$INSTALL_PREFIX/bin:\$PATH\""
echo
echo "2. 或者立即使用:"
echo "   export PATH=\"$INSTALL_PREFIX/bin:\$PATH\""
echo "   source ~/.zshrc"
echo
echo "3. 验证安装:"
echo "   qemu-system-riscv64 --version"
echo
echo "4. 运行 rCore-Lab:"
echo "   cd /Users/mac/Desktop/project/rcore-lab"
echo "   bash run.sh"
echo
