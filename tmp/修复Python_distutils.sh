#!/bin/bash

# =============================================
# 修复 Python 3.14 distutils 问题
# 用于编译 QEMU 7.0.0
# =============================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}======================================="
echo "  修复 Python 3.14 distutils 问题"
echo -e "=======================================${NC}"
echo

echo -e "${YELLOW}[1/3]${NC} 检查 Python 版本..."
PYTHON_VERSION=$(python3 --version | grep -oE '[0-9]+\.[0-9]+')
echo "当前 Python 版本: $PYTHON_VERSION"

echo -e "${YELLOW}[2/3]${NC} 安装 setuptools（提供 distutils 替代）..."
if python3 -m pip install setuptools 2>/dev/null; then
    echo -e "${GREEN}✅ setuptools 安装成功${NC}"
else
    echo -e "${YELLOW}尝试使用 --break-system-packages...${NC}"
    python3 -m pip install setuptools --break-system-packages
fi

echo -e "${YELLOW}[3/3]${NC} 验证安装..."
if python3 -c "import setuptools; print('setuptools version:', setuptools.__version__)" 2>/dev/null; then
    echo -e "${GREEN}✅ setuptools 可用${NC}"
else
    echo -e "${RED}❌ setuptools 安装失败${NC}"
    exit 1
fi

echo
echo -e "${GREEN}======================================="
echo "  ✅ 修复完成！"
echo -e "=======================================${NC}"
echo
echo "现在可以重新配置 QEMU 7.0.0:"
echo "  cd /Users/mac/Desktop/project/rcore-lab/tmp/qemu-7.0.0"
echo "  ./configure --target-list=riscv64-softmmu --prefix=\$HOME/qemu-7.0.0"
echo "  make -j\$(sysctl -n hw.ncpu)"
echo "  make install"
echo
