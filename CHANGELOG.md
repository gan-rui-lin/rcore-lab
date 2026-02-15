# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Added (2026-02-15)

#### 中断安全基础设施 [5f3b8ee]
- **UPIntrFreeCell 迁移**: 将所有全局同步原语从 `UPSafeCell` 迁移到 `UPIntrFreeCell`
  - 解决中断上下文中的 RefCell 借用冲突问题
  - 自动中断屏蔽/恢复机制
  - 支持嵌套访问和 try_exclusive_access
  - 影响18个文件：task、mm、fs、sync、timer等核心模块

#### 文档改进 [0d79cf1]
- **迁移文档**: `docs/UPIntrFreeCell-Migration.md`
  - 详细的问题分析和解决方案
  - 完整的API变更说明
  - 开发指南和最佳实践
  - 注意事项和未来优化方向

### Fixed (2026-02-15)

#### TLS支持 [e47e5c3]
- **最小TCB初始化**: 为没有PT_TLS段的程序（如busybox）初始化最小线程控制块
  - 在用户栈顶分配16字节TCB
  - 正确设置tp寄存器指向TCB
  - 兼容静态链接的musl程序

---

## [Phase 3] - 2026-02-14

### Added

#### System V IPC支持 [6bf2667]
实现9个IPC系统调用，完整支持进程间通信：

**消息队列**:
- `msgget(186)`: 创建或获取消息队列
- `msgsnd(187)`: 发送消息
- `msgrcv(188)`: 接收消息（支持类型过滤）
- `msgctl(189)`: 消息队列控制操作

**共享内存**:
- `shmget(194)`: 创建或获取共享内存段
- `shmat(196)`: 附加共享内存到地址空间
- `shmdt(197)`: 分离共享内存
- `shmctl(195)`: 共享内存控制操作

**信号扩展**:
- `rt_sigtimedwait(137)`: 带超时的信号等待

**实现特性**:
- 全局IpcManager管理所有IPC对象
- 基于BTreeMap的高效查找
- IPC_PRIVATE和key-based访问
- 完整的IPC flags支持（IPC_CREAT, IPC_EXCL, IPC_NOWAIT）
- 引用计数和资源管理

---

## [Phase 2] - 2026-02-13

### Added

#### 系统调用扩展 [ad8456a]
实现3个重要的系统调用：

- `ioctl(29)`: I/O设备控制
  - TCGETS: 获取终端属性
  - TCSETS: 设置终端属性
  - TIOCGWINSZ: 获取窗口大小
  - TIOCSWINSZ: 设置窗口大小

- `ftruncate(46)`: 文件截断
  - 扩展或收缩文件到指定大小
  - 支持VFS inode操作

- `sendfile(71)`: 零拷贝文件传输
  - 内核态高效文件复制
  - 支持offset参数

#### Shebang支持 [7cbb924]
- **脚本解释器**: 自动识别 `#!/bin/sh` 等shebang
  - 递归解释器查找（最多3层）
  - 正确的参数传递
  - 与busybox shell完美集成

**文档**:
- `docs/shebang-implementation.md`: Shebang实现细节

---

## [Phase 1] - 2026-02-12

### Added

#### 基础系统调用

- `lseek(62)`: 文件定位
- `writev(66)`: 向量写
- `fcntl(25)`: 文件控制操作
- `mprotect(226)`: 内存保护

---

## 技术栈

- **Rust Toolchain**: nightly-2024-05-02
- **Target**: riscv64gc-unknown-none-elf
- **Architecture**: RISC-V 64-bit
- **Kernel**: rCore-lab
- **Userspace**: musl-libc + busybox

---

## 贡献者

- rCore开发团队
- Claude Opus 4.6 (AI辅助开发)

---

## 参考文档

- [UPIntrFreeCell迁移指南](docs/UPIntrFreeCell-Migration.md)
- [Shebang实现文档](docs/shebang-implementation.md)
- [System V IPC规范](https://pubs.opengroup.org/onlinepubs/9699919799/)
- [RISC-V特权级规范](https://riscv.org/specifications/)
