# Conda 安装和环境变量配置指南

## 检查是否已安装 Conda

首先检查 Conda 是否已安装：

```bash
# 方法 1：检查命令
which conda

# 方法 2：尝试运行
conda --version

# 方法 3：查找可能的安装位置
ls -la ~/miniconda3/bin/conda
ls -la ~/anaconda3/bin/conda
```

---

## 情况 1：Conda 已安装但不在 PATH 中

### 使用自动化脚本（推荐）⭐⭐⭐⭐⭐

```bash
cd /Users/mac/Desktop/project/rcore-lab/tmp
bash 配置conda环境变量.sh
```

脚本会自动：
- ✅ 搜索 Conda 安装位置
- ✅ 添加到 ~/.zshrc
- ✅ 验证配置

### 手动配置

如果你知道 Conda 的安装位置（例如 `~/miniconda3`）：

```bash
# 1. 编辑 ~/.zshrc
vim ~/.zshrc
# 或
code ~/.zshrc

# 2. 添加以下内容（替换 CONDA_PATH）
CONDA_PATH="$HOME/miniconda3"  # 根据实际路径修改

# >>> conda initialize >>>
__conda_setup="$('$CONDA_PATH/bin/conda' 'shell.zsh' 'hook' 2> /dev/null)"
if [ $? -eq 0 ]; then
    eval "$__conda_setup"
else
    if [ -f "$CONDA_PATH/etc/profile.d/conda.sh" ]; then
        . "$CONDA_PATH/etc/profile.d/conda.sh"
    else
        export PATH="$CONDA_PATH/bin:$PATH"
    fi
fi
unset __conda_setup
# <<< conda initialize <<<

# 3. 重新加载配置
source ~/.zshrc

# 4. 验证
conda --version
```

---

## 情况 2：Conda 未安装

### 方法 A：使用 Homebrew 安装（最简单）⭐⭐⭐⭐⭐

```bash
# 1. 安装 Miniconda
brew install --cask miniconda

# 2. 初始化（会自动配置 ~/.zshrc）
conda init zsh

# 3. 重新加载配置
source ~/.zshrc

# 4. 验证
conda --version
```

### 方法 B：官方安装脚本

#### Apple Silicon (M1/M2/M3) Mac

```bash
# 1. 下载安装脚本
cd /tmp
curl -O https://repo.anaconda.com/miniconda/Miniconda3-latest-MacOSX-arm64.sh

# 2. 运行安装
bash Miniconda3-latest-MacOSX-arm64.sh

# 按照提示操作：
# - 阅读许可协议（按空格翻页，输入 yes）
# - 确认安装位置（默认 ~/miniconda3）
# - 选择是否初始化（建议选 yes）

# 3. 如果选择了初始化，重新加载配置
source ~/.zshrc

# 4. 验证
conda --version
```

#### Intel Mac

```bash
# 1. 下载安装脚本
cd /tmp
curl -O https://repo.anaconda.com/miniconda/Miniconda3-latest-MacOSX-x86_64.sh

# 2. 运行安装
bash Miniconda3-latest-MacOSX-x86_64.sh

# 3. 重新加载配置
source ~/.zshrc

# 4. 验证
conda --version
```

---

## 验证安装

安装完成后，运行以下命令验证：

```bash
# 检查版本
conda --version

# 查看环境列表
conda env list

# 查看安装位置
which conda

# 查看配置信息
conda info
```

预期输出：
```
conda 24.x.x
# conda environments:
#
base                  *  /Users/mac/miniconda3
```

---

## 常用 Conda 命令

### 环境管理

```bash
# 创建新环境
conda create -n myenv python=3.11

# 激活环境
conda activate myenv

# 退出环境
conda deactivate

# 列出所有环境
conda env list

# 删除环境
conda remove -n myenv --all
```

### 包管理

```bash
# 安装包
conda install numpy pandas

# 搜索包
conda search numpy

# 列出已安装的包
conda list

# 更新包
conda update numpy

# 卸载包
conda remove numpy
```

### 配置 Conda

```bash
# 添加国内镜像源（加速下载）
conda config --add channels https://mirrors.tuna.tsinghua.edu.cn/anaconda/pkgs/free/
conda config --add channels https://mirrors.tuna.tsinghua.edu.cn/anaconda/pkgs/main/
conda config --set show_channel_urls yes

# 查看配置
conda config --show

# 恢复默认源
conda config --remove-key channels
```

---

## 为 rCore-Lab 项目创建专用环境（可选）

如果你想为 rCore-Lab 项目创建独立的 Python 环境：

```bash
# 1. 创建环境（Python 3.11 与 QEMU 编译兼容）
conda create -n rcore python=3.11

# 2. 激活环境
conda activate rcore

# 3. 安装需要的包
conda install setuptools pip

# 4. 之后编译 QEMU 时使用这个环境
conda activate rcore
cd /Users/mac/Desktop/project/rcore-lab/tmp/qemu-7.2.0
./configure --target-list=riscv64-softmmu --prefix=$HOME/qemu-7.2.0
make -j$(sysctl -n hw.ncpu)
```

---

## 故障排查

### 问题 1: conda: command not found

**原因**: Conda 不在 PATH 中

**解决**:
```bash
# 运行自动配置脚本
cd /Users/mac/Desktop/project/rcore-lab/tmp
bash 配置conda环境变量.sh

# 或手动添加到 PATH
export PATH="$HOME/miniconda3/bin:$PATH"
source ~/.zshrc
```

### 问题 2: CommandNotFoundError: Your shell has not been properly configured

**原因**: Conda 未初始化

**解决**:
```bash
# 初始化 conda（会修改 ~/.zshrc）
conda init zsh

# 重新加载
source ~/.zshrc
```

### 问题 3: 新终端窗口 conda 不可用

**原因**: ~/.zshrc 未加载

**解决**:
```bash
# 检查 ~/.zshrc 中是否有 conda 配置
cat ~/.zshrc | grep conda

# 手动加载
source ~/.zshrc

# 或重启终端
```

### 问题 4: conda activate 不工作

**原因**: Shell 未初始化

**解决**:
```bash
# 重新初始化
conda init zsh
source ~/.zshrc

# 或使用 source 命令
source activate myenv
```

---

## 卸载 Conda（如果需要）

### 完全卸载 Miniconda

```bash
# 1. 删除安装目录
rm -rf ~/miniconda3

# 2. 从 ~/.zshrc 中删除 conda 配置
# 编辑 ~/.zshrc，删除 ">>> conda initialize >>>" 和 "<<< conda initialize <<<" 之间的所有内容

# 3. 删除隐藏文件
rm -rf ~/.conda
rm -rf ~/.condarc

# 4. 重新加载配置
source ~/.zshrc
```

---

## 推荐工作流

### 日常开发

```bash
# 1. 打开终端
# 2. 激活项目环境
conda activate myenv

# 3. 工作...

# 4. 完成后退出环境
conda deactivate
```

### 针对 rCore-Lab

如果你要编译 QEMU，推荐：

```bash
# 1. 创建专用环境
conda create -n qemu-build python=3.11

# 2. 每次编译前激活
conda activate qemu-build

# 3. 编译 QEMU
cd /Users/mac/Desktop/project/rcore-lab/tmp
bash 安装QEMU7.sh

# 4. 完成后可以退出
conda deactivate
```

---

## 快速参考

| 命令 | 作用 |
|------|------|
| `conda --version` | 查看版本 |
| `conda env list` | 列出所有环境 |
| `conda create -n ENV` | 创建环境 |
| `conda activate ENV` | 激活环境 |
| `conda deactivate` | 退出环境 |
| `conda install PKG` | 安装包 |
| `conda list` | 列出已安装的包 |
| `conda init zsh` | 初始化 shell |

---

**创建日期**: 2026-02-12
**系统**: macOS
**Shell**: zsh
