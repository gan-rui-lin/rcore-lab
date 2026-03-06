# rCore-Lab 项目文档索引

**最后更新**: 2026-03-06 &nbsp;&nbsp; **文档总数**: 45 篇 &nbsp;&nbsp; **分类**: 8 个

---

## 目录结构

```
docs/ZJYDocs/
├── 环境配置/          (9篇) 开发环境、QEMU、工具链
├── 系统调用/          (6篇) syscall 实现与对比
├── 线程与信号/        (11篇) TLS、pthread、信号处理、futex
├── 网络模块/          (2篇) smoltcp 网络栈适配
├── 文件系统/          (2篇) VFS、块缓存
├── 多架构移植/        (8篇) LoongArch、RISC-V 抽象层
├── 调试记录/          (5篇) PageFault、ABI、分支修复
├── 项目管理/          (3篇) 进展总结、变更日志
└── INDEX.md           本文件
```

---

## 环境配置

| 文档 | 说明 |
|------|------|
| [快速开始指南](环境配置/快速开始指南.md) | 项目编译运行入门 |
| [macOS开发环境配置](环境配置/macOS开发环境配置.md) | Rust 工具链 + QEMU 安装 |
| [常见环境问题排查](环境配置/常见环境问题排查.md) | PATH、工具链版本等问题 |
| [macOS下Docker挂载ext4镜像](环境配置/macOS下Docker挂载ext4镜像.md) | ext4-tools.sh 使用说明 |
| [QEMU版本兼容性修复](环境配置/QEMU版本兼容性修复.md) | QEMU 7.2 vs 8.2 问题 |
| [QEMU修复说明](环境配置/QEMU修复说明.md) | virtio 设备修复 |
| [QEMU兼容性最终方案](环境配置/QEMU兼容性最终方案.md) | 完整修复流程 |
| [make_run卡住问题解决](环境配置/make_run卡住问题解决.md) | 串口输出问题 |
| [系统调用日志级别使用说明](环境配置/系统调用日志级别使用说明.md) | LOG=TRACE/INFO/WARN 用法 |

## 系统调用

| 文档 | 说明 |
|------|------|
| [rCore系统调用详细文档](系统调用/rCore系统调用详细文档.md) | rCore-Lab 已实现 syscall 详解 |
| [xv6系统调用详细文档](系统调用/xv6系统调用详细文档.md) | xv6-Lab syscall 对比参考 |
| [缺失系统调用清单](系统调用/缺失系统调用清单.md) | 未实现 syscall 优先级排序 |
| [系统调用对比总结](系统调用/系统调用对比总结.md) | rCore vs xv6 对比 |
| [第一阶段实现总结](系统调用/第一阶段实现总结.md) | 初始 syscall 批量实现记录 |
| [Shebang脚本支持实现](系统调用/Shebang脚本支持实现.md) | `#!/bin/sh` 解析支持 |

## 线程与信号

| 文档 | 说明 |
|------|------|
| [TLS与AUXV实现详细文档](线程与信号/TLS与AUXV实现详细文档.md) | **核心文档** — 完整 Debug 过程 |
| [TLS实现方案设计](线程与信号/TLS实现方案设计.md) | TLS 技术方案 |
| [TLS实现总结](线程与信号/TLS实现总结.md) | TLS 完成清单 |
| [AUXV实现总结](线程与信号/AUXV实现总结.md) | Auxiliary Vector 详解 |
| [pthread_cancel信号循环修复_0226](线程与信号/pthread_cancel信号循环修复_0226.md) | sigreturn 循环问题 |
| [pthread_cancel中断修复_0227](线程与信号/pthread_cancel中断修复_0227.md) | EINTR 处理修复 |
| [pthread_cancel临时方案_0301](线程与信号/pthread_cancel临时方案_0301.md) | 临时 workaround |
| [pthread_cancel折衷方案_0301](线程与信号/pthread_cancel折衷方案_0301.md) | 最终折衷方案 |
| [fd限制与pthread_cancel调试_0304](线程与信号/fd限制与pthread_cancel调试_0304.md) | fd 上限 + cancel 联合调试 |
| [信号与futex重构_0304](线程与信号/信号与futex重构_0304.md) | 信号处理 + futex 重构 |
| [rustoswhu信号futex分析_0301](线程与信号/rustoswhu信号futex分析_0301.md) | 参考项目信号实现分析 |

## 网络模块

| 文档 | 说明 |
|------|------|
| [smoltcp网络栈适配实现记录](网络模块/smoltcp网络栈适配实现记录.md) | **核心文档** — 完整开发历程、10 个问题解决、测试 Pass |
| [整合分析与重构方案](网络模块/整合分析与重构方案.md) | smoltcp 架构分析 + 旧代码评估 + 设计方案 |

## 文件系统

| 文档 | 说明 |
|------|------|
| [文件系统实现分析](文件系统/文件系统实现分析.md) | VFS / EasyFS / FAT32 / EXT4 详解 |
| [块缓存实现与性能优化](文件系统/块缓存实现与性能优化.md) | 块缓存机制与性能调优 |

## 多架构移植

| 文档 | 说明 |
|------|------|
| [整体分析_rustoswhu项目](多架构移植/整体分析_rustoswhu项目.md) | OSKernel2025-rustoswhu 多架构方案总览 |
| [启动流程抽象](多架构移植/启动流程抽象.md) | 多架构 boot 抽象设计 |
| [LoongArch启动流程深度分析](多架构移植/LoongArch启动流程深度分析.md) | LoongArch 启动全过程 |
| [Trap处理抽象](多架构移植/Trap处理抽象.md) | 中断/异常处理抽象 |
| [上下文切换抽象](多架构移植/上下文切换抽象.md) | TaskContext 切换 |
| [信号跳板抽象](多架构移植/信号跳板抽象.md) | 信号处理 trampoline |
| [定时器与中断抽象](多架构移植/定时器与中断抽象.md) | 定时器 + PLIC/中断 |
| [页表抽象](多架构移植/页表抽象.md) | 多架构页表映射 |

## 调试记录

| 文档 | 说明 |
|------|------|
| [Linux_ABI栈布局问题报告](调试记录/Linux_ABI栈布局问题报告.md) | 用户栈 ABI 布局 bug |
| [LoadPageFault调试报告](调试记录/LoadPageFault调试报告.md) | 页错误调试 |
| [内核PageFault分析_0301](调试记录/内核PageFault分析_0301.md) | 内核态页错误 |
| [UPIntrFreeCell迁移记录](调试记录/UPIntrFreeCell迁移记录.md) | 同步原语迁移 |
| [rv32分支合并与busybox修复](调试记录/rv32分支合并与busybox修复.md) | 分支合并 + busybox 适配 |

## 项目管理

| 文档 | 说明 |
|------|------|
| [当前进展总结](项目管理/当前进展总结.md) | 项目状态快速概览 |
| [变更日志](项目管理/变更日志.md) | 版本变更记录 |
| [项目说明](项目管理/项目说明.md) | 项目 README |
