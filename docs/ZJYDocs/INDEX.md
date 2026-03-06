# rCore-lab 项目文档索引

## 📚 文档目录

### 🔧 环境配置相关

1. **[macOS环境配置指南.md](./macOS环境配置指南.md)**
   - macOS 上的完整环境配置步骤
   - Rust 工具链安装
   - QEMU 安装与配置

2. **[环境配置问题排查.md](./环境配置问题排查.md)**
   - 常见环境问题及解决方案
   - PATH 配置问题
   - 工具链版本问题

3. **[快速开始.md](./快速开始.md)**
   - 项目快速启动指南
   - 基本编译运行步骤

### 🐛 问题修复记录

4. **[QEMU版本问题修复.md](./QEMU版本问题修复.md)**
   - QEMU 7.2.0 与 8.2.0 兼容性问题
   - virtio 设备问题解决

5. **[问题解决-make_run卡住.md](./问题解决-make_run卡住.md)**
   - make run 卡住问题分析
   - 串口输出问题

6. **[最终解决方案-QEMU兼容性.md](./最终解决方案-QEMU兼容性.md)**
   - QEMU 兼容性的最终解决方案
   - 完整的修复流程

### 🎯 TLS 与 Auxiliary Vector 实现

7. **[文件系统实现分析.md](./文件系统实现分析.md)** ⭐ **新增**
   - **VFS 虚拟文件系统详解**
   - **EasyFS / FAT32 / EXT4 完整分析**
   - **文件系统对比与选择建议**
   - **已知问题与解决方案**
   - 中文详细文档，推荐阅读

8. **[当前进展总结.md](./当前进展总结.md)** ⭐ **新增**
   - **项目当前状态总结**
   - **已完成功能清单**
   - **下一步行动计划**
   - **成就与进展评估**
   - 中文快速概览文档

9. **[TLS与AUXV实现详细文档.md](./TLS与AUXV实现详细文档.md)** ⭐ **最重要**
   - **完整的 Debug 过程记录**
   - **解决的所有问题详解**
   - **新增功能点说明**
   - **技术细节与实现原理**
   - **当前状态与下一步计划**
   - 中文详细文档，推荐首先阅读

10. **[TLS_IMPLEMENTATION_PLAN.md](./TLS_IMPLEMENTATION_PLAN.md)**
    - TLS 实现计划（英文）
    - 技术方案设计

11. **[TLS_IMPLEMENTATION_SUMMARY.md](./TLS_IMPLEMENTATION_SUMMARY.md)**
    - TLS 实现总结（英文）
    - 已完成工作清单
    - 测试结果分析

12. **[AUXV_IMPLEMENTATION_SUMMARY.md](./AUXV_IMPLEMENTATION_SUMMARY.md)**
    - Auxiliary Vector 实现总结（英文）
    - 详细的技术说明
    - 调试建议

### 📖 推荐阅读顺序

#### 新手入门
1. [快速开始.md](./快速开始.md) - 了解项目基本操作
2. [macOS环境配置指南.md](./macOS环境配置指南.md) - 配置开发环境
3. [README.md](./README.md) - 项目总体说明

#### 遇到问题时
1. [环境配置问题排查.md](./环境配置问题排查.md) - 环境问题
2. [最终解决方案-QEMU兼容性.md](./最终解决方案-QEMU兼容性.md) - QEMU 问题

#### 深入了解文件系统
1. ⭐ [文件系统实现分析.md](./文件系统实现分析.md) - **推荐**，VFS/EasyFS/FAT32/EXT4 完整分析

#### 深入了解 TLS/AUXV 实现
1. ⭐ [当前进展总结.md](./当前进展总结.md) - **快速概览**，项目当前状态
2. ⭐ [TLS与AUXV实现详细文档.md](./TLS与AUXV实现详细文档.md) - **首选**，中文详细文档
3. [TLS_IMPLEMENTATION_PLAN.md](./TLS_IMPLEMENTATION_PLAN.md) - 技术方案
4. [TLS_IMPLEMENTATION_SUMMARY.md](./TLS_IMPLEMENTATION_SUMMARY.md) - 实现总结
5. [AUXV_IMPLEMENTATION_SUMMARY.md](./AUXV_IMPLEMENTATION_SUMMARY.md) - Auxv 详解

### 🌐 网络模块

13. **[网络模块/smoltcp网络栈适配实现记录.md](./网络模块/smoltcp网络栈适配实现记录.md)** ⭐ **新增**
    - **smoltcp v0.11.0 整合全过程**
    - **VirtIO-Net 驱动重写**
    - **16 个网络系统调用实现**
    - **UDP/TCP loopback 实现**
    - **开发历程与 10 个解决的问题**

14. **[网络协议栈smoltcp整合分析与重构方案.md](./网络协议栈smoltcp整合分析与重构方案.md)**
    - smoltcp 架构分析
    - 旧代码问题评估
    - 整合方案设计

---

## 📊 文档统计

| 类别 | 文档数量 |
|------|---------|
| 环境配置 | 3 |
| 问题修复 | 3 |
| 文件系统 | 1 |
| TLS/AUXV | 5 |
| 网络模块 | 2 |
| **总计** | **14** |

---

## 🔍 快速查找

### 按主题查找

**环境配置**：
- macOS 配置 → `macOS环境配置指南.md`
- 环境问题 → `环境配置问题排查.md`
- 快速启动 → `快速开始.md`

**QEMU 问题**：
- 版本问题 → `QEMU版本问题修复.md`
- 卡住问题 → `问题解决-make_run卡住.md`
- 兼容性 → `最终解决方案-QEMU兼容性.md`

**网络模块**：
- smoltcp 适配全过程 → `网络模块/smoltcp网络栈适配实现记录.md` ⭐
- 整合方案设计 → `网络协议栈smoltcp整合分析与重构方案.md`

**文件系统**：
- VFS/文件系统分析 → `文件系统实现分析.md` ⭐

**TLS/AUXV**：
- **项目进展** → `当前进展总结.md` ⭐
- **完整 Debug 过程** → `TLS与AUXV实现详细文档.md` ⭐
- 实现计划 → `TLS_IMPLEMENTATION_PLAN.md`
- 实现总结 → `TLS_IMPLEMENTATION_SUMMARY.md`
- Auxv 详解 → `AUXV_IMPLEMENTATION_SUMMARY.md`

### 按关键词查找

| 关键词 | 相关文档 |
|--------|---------|
| PATH 配置 | 环境配置问题排查.md, TLS与AUXV实现详细文档.md |
| Rust 工具链 | macOS环境配置指南.md, 环境配置问题排查.md |
| QEMU | QEMU版本问题修复.md, 最终解决方案-QEMU兼容性.md |
| virtio | QEMU版本问题修复.md |
| VFS | 文件系统实现分析.md |
| EasyFS | 文件系统实现分析.md |
| FAT32 | 文件系统实现分析.md |
| EXT4 | 文件系统实现分析.md |
| set_tid_address | TLS与AUXV实现详细文档.md, 当前进展总结.md |
| Thread Local Storage | TLS_IMPLEMENTATION_*.md, TLS与AUXV实现详细文档.md |
| Auxiliary Vector | AUXV_IMPLEMENTATION_SUMMARY.md, TLS与AUXV实现详细文档.md |
| busybox | TLS与AUXV实现详细文档.md, 当前进展总结.md |
| musl libc | TLS与AUXV实现详细文档.md |

---

## 🎯 重要提示

### ⭐ 必读文档

#### 1. **[当前进展总结.md](./当前进展总结.md)** - 快速了解项目状态

包含：
- 🎯 项目当前状态（95% 完成）
- ✅ 已完成功能（TLS, Auxv, set_tid_address）
- ❌ 剩余问题分析
- 🚀 下一步行动计划

**适合**：想快速了解项目进展的开发者（5 分钟阅读）

#### 2. **[TLS与AUXV实现详细文档.md](./TLS与AUXV实现详细文档.md)** - 完整技术细节

这是**最详细**的技术文档，包含：
- 📝 完整的 Debug 过程（6个阶段）
- ✅ 7个已解决的问题
- 🆕 6个新增功能点
- 🔧 详细的技术实现
- 📈 当前状态分析
- 🚀 下一步计划

**适合**：想深入了解 TLS/AUXV 实现的开发者（30 分钟阅读）

#### 3. **[文件系统实现分析.md](./文件系统实现分析.md)** - 文件系统全面解析

包含：
- 🗂️ VFS 虚拟文件系统架构
- 📁 EasyFS / FAT32 / EXT4 详细分析
- ⚖️ 文件系统功能对比
- 🐛 已知问题与解决方案

**适合**：想了解文件系统实现的开发者（20 分钟阅读）

---

## 📝 文档维护

### 更新记录

- **2026-02-12 17:30**: 添加文件系统实现分析文档
- **2026-02-12 17:00**: 添加当前进展总结文档
- **2026-02-12**: 创建索引文档
- **2026-02-12**: 添加 TLS 与 AUXV 实现详细文档
- **2026-02-12**: 整理所有文档到 ZJYDocs 目录

### 贡献指南

新增文档时请：
1. 放在 `docs/ZJYDocs/` 目录下
2. 更新本索引文档
3. 使用清晰的文件名（中文或英文）
4. 在文档中包含目录和总结

---

## 📧 联系方式

如有问题，请查阅相关文档或联系项目维护者。

---

**最后更新**: 2026-03-06
**文档版本**: v1.2
**新增文档**: 网络模块/smoltcp网络栈适配实现记录.md, 网络协议栈smoltcp整合分析与重构方案.md
