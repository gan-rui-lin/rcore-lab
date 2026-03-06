# ZJY 的 rCore-Lab 学习文档

## 目录

本文档集包含了在 macOS 上配置和使用 rCore-Lab 的完整指南。

### 📚 文档列表

1. **[快速开始.md](./快速开始.md)**
   - 一键安装脚本
   - 快速配置指南
   - 常用命令参考
   - 学习路径建议

2. **[macOS环境配置指南.md](./macOS环境配置指南.md)**
   - 详细的环境配置步骤
   - 遇到的问题和解决方案
   - macOS 特定注意事项
   - 完整的工具清单

3. **[环境配置问题排查.md](./环境配置问题排查.md)**
   - 常见错误诊断
   - 故障排查步骤
   - 快速修复脚本
   - 环境检查清单

## 📝 配置总结

### 环境信息
- **系统**: macOS (Darwin 24.6.0)
- **架构**: Apple Silicon (ARM64)
- **配置日期**: 2026-02-12

### 已安装工具
- ✅ Rust 1.93.0 (via Homebrew)
- ✅ rustup 1.28.2
- ✅ nightly-2024-05-02 工具链
- ✅ riscv64gc-unknown-none-elf 目标
- ✅ cargo-binutils
- ✅ QEMU RISC-V 模拟器

### 关键修复
- ✅ Makefile sed 命令 macOS 兼容性修复
- ✅ 通过 Homebrew 解决网络问题
- ✅ PATH 环境变量正确配置

## 🚀 快速开始

如果你是第一次配置环境，建议按以下顺序阅读：

```
1. 快速开始.md（获取基本概念和一键脚本）
   ↓
2. macOS环境配置指南.md（了解详细配置过程）
   ↓
3. 环境配置问题排查.md（遇到问题时参考）
```

## 💡 使用建议

### 日常使用
```bash
# 进入项目目录
cd /Users/mac/Desktop/project/rcore-lab/os

# 运行 OS
make run

# 退出 QEMU
Ctrl+A, X
```

### 学习建议
1. 先通读 [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/)
2. 按章节顺序完成实验（ch1-ch9）
3. 每完成一章就提交代码
4. 使用测试框架验证实现

### 调试技巧
```bash
# 查看反汇编
make disasm

# GDB 调试
make debug

# 查看编译详细信息
make run V=1
```

## 📊 学习进度追踪

### 章节完成情况
- [ ] ch1: 应用程序与基本执行环境
- [ ] ch2: 批处理系统
- [ ] ch3: 多道程序与分时多任务
- [ ] ch4: 地址空间
- [ ] ch5: 进程及进程管理
- [ ] ch6: 文件系统
- [ ] ch7: 进程间通信
- [ ] ch8: 并发
- [ ] ch9: 实战项目

### 实验测试
- [ ] ch3 测试通过
- [ ] ch4 测试通过
- [ ] ch5 测试通过
- [ ] ch6 测试通过
- [ ] ch8 测试通过

## 🔗 相关链接

### 官方资源
- [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/) - 详细教程
- [rCore-Tutorial-Guide](https://LearningOS.github.io/rCore-Tutorial-Guide/) - 简明指南
- [API 文档](https://learningos.github.io/rCore-Tutorial-Code/) - 代码文档
- [GitHub 仓库](https://github.com/rcore-os/rCore-Tutorial-v3) - 源代码

### 技术文档
- [Rust 官方文档](https://doc.rust-lang.org/)
- [RISC-V 规范](https://riscv.org/specifications/)
- [QEMU 文档](https://www.qemu.org/docs/master/)

## 📝 笔记模板

为每个章节创建笔记时，可以使用以下模板：

```markdown
# Chapter X: 章节标题

## 学习日期
YYYY-MM-DD

## 本章目标
- 目标 1
- 目标 2

## 关键概念
### 概念 1
解释...

### 概念 2
解释...

## 代码实现

### 功能 1
```rust
// 代码示例
```

### 功能 2
```rust
// 代码示例
```

## 遇到的问题

### 问题 1
**描述**:
**解决方案**:

## 实验结果
- [ ] 编译通过
- [ ] 运行成功
- [ ] 测试通过

## 心得体会
...

## 参考资料
- 链接 1
- 链接 2
```

## 🛠️ 维护记录

### 2026-02-12
- ✅ 创建文档目录结构
- ✅ 完成环境配置指南
- ✅ 编写问题排查手册
- ✅ 准备快速开始指南
- ✅ 首次成功运行 OS

### 待办事项
- [ ] 开始 ch1 学习
- [ ] 记录每章学习笔记
- [ ] 总结常见问题
- [ ] 分享学习经验

## 💬 备注

### 常见命令速查

```bash
# 环境检查
rustup show
rustup target list | grep riscv

# 构建
make clean
make build

# 运行
make run

# 调试
make disasm
make debug

# Git
git status
git checkout ch3
git branch

# 进程管理
ps aux | grep qemu
pkill -f qemu-system-riscv64

# 查看日志
make run 2>&1 | tee run.log
```

### 环境变量（~/.zshrc）

```bash
# rCore-Lab 环境
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
export PATH="$HOME/.cargo/bin:$PATH"

# 可选：编译加速
export RUSTC_WRAPPER=sccache
export CARGO_BUILD_JOBS=8
```

## 📧 联系方式

如有问题或建议，欢迎交流学习经验。

---

**文档创建日期**: 2026-02-12
**最后更新日期**: 2026-02-12
**作者**: ZJY
**工具**: Claude Code
