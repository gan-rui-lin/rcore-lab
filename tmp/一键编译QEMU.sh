#!/bin/bash

# =============================================
# 一键编译 QEMU 7.0.0
# 自动处理 conda 环境
# =============================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}======================================="
echo "  QEMU 7.0.0 一键编译脚本"
echo -e "=======================================${NC}"
echo

# 检查并激活 conda 环境
if [ -z "$CONDA_DEFAULT_ENV" ]; then
    echo -e "${YELLOW}未检测到 conda 环境，尝试激活 'os' 环境...${NC}"

    # 尝试找到 conda
    if [ -f "/opt/anaconda3/etc/profile.d/conda.sh" ]; then
        source /opt/anaconda3/etc/profile.d/conda.sh
    elif [ -f "/opt/miniconda3/etc/profile.d/conda.sh" ]; then
        source /opt/miniconda3/etc/profile.d/conda.sh
    elif [ -f "$HOME/anaconda3/etc/profile.d/conda.sh" ]; then
        source $HOME/anaconda3/etc/profile.d/conda.sh
    elif [ -f "$HOME/miniconda3/etc/profile.d/conda.sh" ]; then
        source $HOME/miniconda3/etc/profile.d/conda.sh
    else
        echo -e "${RED}❌ 找不到 conda，请手动激活环境后再运行：${NC}"
        echo "  conda activate os"
        echo "  bash /Users/mac/Desktop/project/rcore-lab/tmp/编译QEMU7.0.0.sh"
        exit 1
    fi

    # 激活 os 环境
    conda activate os

    if [ $? -ne 0 ]; then
        echo -e "${RED}❌ 激活 conda 环境失败${NC}"
        echo "请手动运行："
        echo "  conda activate os"
        exit 1
    fi
fi

echo -e "${GREEN}✅ Conda 环境: $CONDA_DEFAULT_ENV${NC}"
echo

# 调用实际的编译脚本
bash /Users/mac/Desktop/project/rcore-lab/tmp/编译QEMU7.0.0.sh
