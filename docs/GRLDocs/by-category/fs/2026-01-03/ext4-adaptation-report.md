# ext4 适配记录与问题分析

## 适配概览
本次工作目标是在 rcore-lab 内核上启用 ext4 根文件系统，底层采用外部库 lwext4_rust。该库通过 C 的 lwext4 实现 ext2/3/4 并提供 Rust 封装。适配过程中需要同时解决：
1) lwext4 子模块源码缺失导致 build.rs 失败；
2) 缺少 riscv64 musl 工具链导致 CMake 无法找到编译器；
3) 旧的 bindings 生成结果在当前 Rust 工具链上语法不兼容；
4) C 库输出依赖 libc 的 printf/fflush 等符号，导致内核链接失败；
5) 与现有文件系统抽象不一致，需要参考 chronix 的适配思路，调整 block 读写策略与 mount 方式。

为避免“盲修补”，整体思路是先对齐 lwext4 构建链路，再对齐 ext4 的 block device 语义，最后让 ext4 与当前简单 FS 层对接。

## 关键适配点与修改思路

### 1) lwext4 源码与构建链路
lwext4_rust 的 build.rs 依赖 c/lwext4 目录作为子模块源码，并在首次构建时打补丁、复制 toolchain 文件、执行 make 生成静态库。仓库中该目录为空会直接导致 build.rs 找不到路径。因此优先保证子模块源码存在，再处理工具链。

### 2) musl 工具链与 CMake toolchain
lwext4 官方依赖 musl 交叉编译器（文档里也明确说明）。本机只有 loongarch64 的 musl 工具链，缺少 riscv64，所以 lwext4 的 CMake 会失败。
本次直接安装 riscv64-linux-musl 交叉工具链到 /home/grl/develop，并加入 PATH，保证 CMake 能识别 riscv64-linux-musl-gcc。

### 3) bindings 与 Rust 语法兼容
lwext4_rust 生成的 bindings.rs 在当前 Rust 工具链下出现 `unsafe extern "C"` 语法错误。修复方式是将 `unsafe extern "C"` 修改为 `extern "C"`，并将内部不安全操作放在 unsafe 块里，保证 ABI 与 Rust 语法兼容。

### 4) libc 依赖与调试输出
lwext4 的 debug 输出默认会调用 printf/fflush，并在 newlib 环境依赖 _impure_ptr。内核链接时没有 libc，因此会产生 undefined symbol。
实际解决策略是“对 musl 友好 + 关闭 debug printf”。做法包括：
- ulibc.c 中移除 newlib 专用的 sys/reent.h 依赖，保留 minimal fflush stub。
- 清理 lwext4 build 目录强制 CMake 重新生成配置，避免旧的 ext4_config.h 缓存。
- 让 lwext4 重新编译，确保不再引入 _impure_ptr/fflush 的符号需求。

### 5) 参考 chronix 的 block device 语义
chronix 的 Disk 实现以 block_id/offset 作为游标，read/write/seek 均更新游标，避免“偏移计算 + 读写拆分”不一致的问题。为了对齐该语义，ext4.rs 中的 Ext4Disk 改为使用 block_id/offset 访问，并通过 read_one/write_one 进行块内读写，seek 计算使用 SEEK_SET/SEEK_CUR/SEEK_END 语义。

### 6) 显式 mount 参数
chronix 适配里 Ext4BlockWrapper 使用 mount_point 与 device_name。为保持一致，在 lwext4_rust 中新增 `new_with_mount`，并在 ext4.rs 中以 `("/", "ext4_fs0")` 初始化 ext4 根。

## 当前构建与运行状态
在完成上述修改后，内核成功构建并进入 QEMU：
- ext4 挂载成功：`[kernel] ext4 mounted as root`
- list_apps 输出为空
- 随后 panic：`initproc not found`

这说明 ext4 根已经接管，但 ext4 镜像里没有 init 程序，因此系统在 task 初始化阶段找不到 initproc。

## initproc not found 的原因分析
根因是根文件系统切换为 ext4 后，init 相关二进制不在 ext4 镜像中：
- list_apps 为空，说明根目录没有任何用户程序。
- 任务初始化逻辑在 [os/src/task/mod.rs](os/src/task/mod.rs#L115) 会从根 FS 读取 init（或 initproc）并启动。
- 原先 easy-fs 的 fs.img 里包含用户程序，但 ext4 root 接管后，这些程序不再可见。

因此 initproc not found 不是 ext4 mount 失败，而是镜像内容为空。

## 下一步适配计划（以“能跑”为目标）
1) 将 init/用户程序写入 ext4 镜像（sdcard-rv.img）。
   - 需要提供写入 ext4 镜像的工具或脚本，或在 build 过程中生成 ext4 镜像。
2) 如果短期内无法准备 ext4 镜像内容，可临时保留 easy-fs 作为 init 的来源，或在 ext4 mount 前读取 init 并缓存。
3) 在系统能跑之后，再继续 VFS 风格的扩展适配（更完整的 dentry/inode/file/sb 管理）。

## 关键文件参考
- ext4 适配入口：[os/src/fs/ext4.rs](os/src/fs/ext4.rs)
- 文件系统切换与 mount：[os/src/fs/inode.rs](os/src/fs/inode.rs)
- lwext4_rust block device 封装：[vendor/lwext4_rust/src/blockdev.rs](vendor/lwext4_rust/src/blockdev.rs)
- lwext4 build 脚本：[vendor/lwext4_rust/build.rs](vendor/lwext4_rust/build.rs)
- lwext4 C 源配置：[vendor/lwext4_rust/c/lwext4/CMakeLists.txt](vendor/lwext4_rust/c/lwext4/CMakeLists.txt)

## 小结
适配难点集中在“C 库 + 内核无 libc”这一典型鸿沟。核心策略是：先保证构建链路与工具链正确，再把 block 语义对齐，最后处理 init 的来源问题。当前 ext4 root 已经挂上，后续只需让 ext4 镜像包含用户程序即可进入正常运行阶段。
