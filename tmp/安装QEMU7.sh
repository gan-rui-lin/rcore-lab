#!/bin/bash

# =============================================
# QEMU 7.x 编译安装脚本（修复 Python 3.14 兼容性）
# =============================================

set -e  # 遇到错误立即退出

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${GREEN}======================================="
echo "  QEMU 7.2.0 编译安装脚本"
echo -e "=======================================${NC}"
echo

# 选择 QEMU 版本（7.2.0 对 Python 3.14 兼容性更好）
QEMU_VERSION="7.2.0"
INSTALL_PREFIX="$HOME/qemu-${QEMU_VERSION}"

echo -e "${YELLOW}[1/6]${NC} 安装编译依赖..."
brew install ninja pkg-config glib pixman || true

echo -e "${YELLOW}[2/6]${NC} 安装 Python setuptools（修复 distutils）..."
pip3 install setuptools || python3 -m pip install setuptools --break-system-packages

echo -e "${YELLOW}[3/6]${NC} 下载 QEMU ${QEMU_VERSION}..."
cd /tmp
if [ ! -f "qemu-${QEMU_VERSION}.tar.xz" ]; then
    curl -O https://download.qemu.org/qemu-${QEMU_VERSION}.tar.xz
fi

echo -e "${YELLOW}[4/6]${NC} 解压..."
if [ -d "qemu-${QEMU_VERSION}" ]; then
    rm -rf "qemu-${QEMU_VERSION}"
fi
tar xf qemu-${QEMU_VERSION}.tar.xz
cd qemu-${QEMU_VERSION}

echo -e "${YELLOW}[5/6]${NC} 配置编译选项..."
./configure \
    --target-list=riscv64-softmmu \
    --prefix="${INSTALL_PREFIX}" \
    --enable-sdl=no \
    --enable-gtk=no \
    --enable-vnc=no

echo -e "${YELLOW}[6/6]${NC} 编译（可能需要 10-20 分钟）..."
make -j$(sysctl -n hw.ncpu)

echo -e "${GREEN}安装到 ${INSTALL_PREFIX}...${NC}"
make install

echo
echo -e "${GREEN}======================================="
echo "  ✅ QEMU ${QEMU_VERSION} 安装成功！"
echo -e "=======================================${NC}"
echo
echo "安装位置: ${INSTALL_PREFIX}"
echo "可执行文件: ${INSTALL_PREFIX}/bin/qemu-system-riscv64"
echo
echo -e "${YELLOW}下一步：${NC}"
echo "1. 添加到 PATH:"
echo "   echo 'export PATH=\"${INSTALL_PREFIX}/bin:\$PATH\"' >> ~/.zshrc"
echo "   source ~/.zshrc"
echo
echo "2. 验证版本:"
echo "   qemu-system-riscv64 --version"
echo
echo "3. 运行项目:"
echo "   cd /Users/mac/Desktop/project/rcore-lab"
echo "   bash run.sh"
echo
