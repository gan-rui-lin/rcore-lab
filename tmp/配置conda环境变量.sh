#!/bin/bash

# =============================================
# Conda 环境变量配置脚本
# =============================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo -e "${GREEN}======================================="
echo "  Conda 环境变量配置"
echo -e "=======================================${NC}"
echo

# 检测可能的 conda 安装位置
CONDA_LOCATIONS=(
    "$HOME/miniconda3"
    "$HOME/anaconda3"
    "$HOME/miniforge3"
    "/opt/homebrew/Caskroom/miniconda/base"
    "/opt/anaconda3"
    "/usr/local/miniconda3"
    "/usr/local/anaconda3"
)

CONDA_PATH=""

echo -e "${YELLOW}[1/3]${NC} 搜索 Conda 安装位置..."

for location in "${CONDA_LOCATIONS[@]}"; do
    if [ -f "$location/bin/conda" ]; then
        CONDA_PATH="$location"
        echo -e "${GREEN}✅ 找到 Conda: $CONDA_PATH${NC}"
        break
    fi
done

if [ -z "$CONDA_PATH" ]; then
    echo -e "${RED}❌ 未找到 Conda 安装${NC}"
    echo
    echo "请手动指定 Conda 安装路径，或者安装 Conda："
    echo
    echo "安装 Miniconda："
    echo "  curl -O https://repo.anaconda.com/miniconda/Miniconda3-latest-MacOSX-arm64.sh"
    echo "  bash Miniconda3-latest-MacOSX-arm64.sh"
    echo
    echo "或使用 Homebrew："
    echo "  brew install --cask miniconda"
    echo
    exit 1
fi

echo -e "${YELLOW}[2/3]${NC} 添加到 ~/.zshrc..."

# 备份 .zshrc
if [ -f ~/.zshrc ]; then
    cp ~/.zshrc ~/.zshrc.backup-$(date +%Y%m%d-%H%M%S)
    echo "已备份 ~/.zshrc"
fi

# 检查是否已经配置
if grep -q "conda initialize" ~/.zshrc 2>/dev/null; then
    echo -e "${YELLOW}⚠️  Conda 已在 ~/.zshrc 中配置${NC}"
    echo "如需重新配置，请手动删除相关行后再运行此脚本"
else
    # 添加 conda 初始化代码
    cat >> ~/.zshrc << EOF

# >>> conda initialize >>>
# !! Contents within this block are managed by 'conda init' !!
__conda_setup="\$('$CONDA_PATH/bin/conda' 'shell.zsh' 'hook' 2> /dev/null)"
if [ \$? -eq 0 ]; then
    eval "\$__conda_setup"
else
    if [ -f "$CONDA_PATH/etc/profile.d/conda.sh" ]; then
        . "$CONDA_PATH/etc/profile.d/conda.sh"
    else
        export PATH="$CONDA_PATH/bin:\$PATH"
    fi
fi
unset __conda_setup
# <<< conda initialize <<<

EOF
    echo -e "${GREEN}✅ Conda 配置已添加到 ~/.zshrc${NC}"
fi

echo -e "${YELLOW}[3/3]${NC} 验证配置..."

# 加载配置
source ~/.zshrc

# 验证 conda 命令
if command -v conda &> /dev/null; then
    CONDA_VERSION=$(conda --version)
    echo -e "${GREEN}✅ Conda 配置成功！${NC}"
    echo "   版本: $CONDA_VERSION"
    echo "   路径: $(which conda)"
else
    echo -e "${RED}❌ Conda 配置可能有问题${NC}"
    echo "请手动运行: source ~/.zshrc"
fi

echo
echo -e "${GREEN}======================================="
echo "  ✅ 配置完成！"
echo -e "=======================================${NC}"
echo
echo -e "${YELLOW}下一步：${NC}"
echo "1. 重新加载配置："
echo "   source ~/.zshrc"
echo
echo "2. 验证 conda："
echo "   conda --version"
echo "   conda env list"
echo
echo "3. 如果当前终端不生效，请打开新终端窗口"
echo
