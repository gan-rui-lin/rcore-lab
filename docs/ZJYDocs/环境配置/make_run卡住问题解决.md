# 问题解决: make run "卡住"问题

## 问题现象

运行 `make run` 后显示：
```
src_path = ../user/build/app/
target_path = ../user/target/riscv64gc-unknown-none-elf/release/
```

然后就没有任何反应，看起来"卡住"了。

## 问题分析

### 实际情况

**OS 并没有卡住！** 它实际上在正常运行，只是看不到输出。

### 根本原因

1. **LOG 输出被关闭**
   - `os/Makefile` 第 5 行：`LOG ?= OFF`
   - OS 使用 `log` crate 的 `info!()` 宏输出日志
   - 默认 LOG=OFF，所有日志被抑制

2. **OS 代码分析** (`os/src/main.rs`)
   ```rust
   pub fn rust_main() -> ! {
       clear_bss();
       // info!("[kernel] Hello, world!");  // 这行被注释了！
       logging::init();
       mm::init();
       mm::remap_test();
       // ... 所有后续输出都用 info!() 宏
   }
   ```

3. **QEMU 正常运行**
   ```bash
   $ ps aux | grep qemu
   # 可以看到 QEMU 进程在运行，CPU 占用正常
   ```

## 完整解决方案

### 方法 1: 使用提供的脚本（最简单）

```bash
cd /Users/mac/Desktop/project/rcore-lab/os

# 启用日志运行
./完整运行脚本.sh --with-log

# 或不启用日志（OS 会运行但没输出）
./完整运行脚本.sh
```

### 方法 2: 手动运行并启用日志

```bash
# 1. 设置环境变量
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

# 2. 进入目录
cd /Users/mac/Desktop/project/rcore-lab/os

# 3. 启用日志并运行
LOG=DEBUG make run
```

### 方法 3: 修改 Makefile 永久启用日志

编辑 `os/Makefile` 第 5 行，将：
```makefile
LOG ?= OFF
```

改为：
```makefile
LOG ?= DEBUG
```

然后正常运行：
```bash
make run
```

## 验证修复

运行后应该看到类似输出：

```
[RustSBI output from bootloader]
[kernel] ext4 mounted as root
[kernel] Application list: ...
[kernel] Starting init process...
```

如果仍然没有输出，检查：

1. **PATH 是否正确**：
   ```bash
   source ~/.zshrc
   which rustc cargo rust-objcopy
   ```

2. **工具链是否激活**：
   ```bash
   rustup show
   # 应显示 nightly-2024-05-02 为 active
   ```

3. **LOG 变量是否传递**：
   ```bash
   # 显式设置
   export LOG=DEBUG
   make run
   ```

## 退出 QEMU

- **方法 1**: 按 `Ctrl+A`, 然后按 `X`
- **方法 2**: 新终端运行 `pkill qemu-system-riscv64`

## 为什么之前能运行？

之前通过后台运行看到了编译成功：
```bash
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && make run
```

但因为在后台运行，QEMU 的终端输出无法正确显示，加上 LOG=OFF，所以看不到任何东西。

## 技术细节

### LOG 环境变量工作原理

1. **Makefile 第 5 行**：
   ```makefile
   LOG ?= OFF
   ```
   默认设置 LOG 为 OFF

2. **Makefile 第 80 行**：
   ```makefile
   @LOG=$(LOG) cargo build $(MODE_ARG) --features $(FEATURES)
   ```
   将 LOG 传递给 cargo build

3. **OS 构建脚本** (`build.rs`) 或 **条件编译**：
   根据 LOG 环境变量配置日志级别

4. **运行时**：
   ```rust
   use log::{info, debug, warn, error};
   info!("This message only shows if LOG is set");
   ```

### 日志级别

- `OFF`: 无输出
- `ERROR`: 只显示错误
- `WARN`: 警告 + 错误
- `INFO`: 信息 + 警告 + 错误
- `DEBUG`: 调试 + 所有上述
- `TRACE`: 最详细

推荐使用 `LOG=DEBUG` 或 `LOG=INFO`。

## 总结

| 症状 | 原因 | 解决方案 |
|------|------|----------|
| make run "卡住" | LOG=OFF，没有输出 | `LOG=DEBUG make run` |
| 构建失败 | PATH 不正确 | `source ~/.zshrc` |
| rust-objcopy 未找到 | cargo bin 不在 PATH | `export PATH="$HOME/.cargo/bin:$PATH"` |
| QEMU 无响应 | 需要手动退出 | `Ctrl+A, X` |

## 快速参考

```bash
# 完整的一行命令（推荐每次使用）
source ~/.zshrc && cd /Users/mac/Desktop/project/rcore-lab/os && LOG=DEBUG make run

# 或使用脚本
./完整运行脚本.sh --with-log

# 调试编译问题
LOG=DEBUG make build

# 清理重新构建
make clean && LOG=DEBUG make build

# 只运行不重新编译
make run-inner

# 检查 QEMU 是否在运行
ps aux | grep qemu

# 强制终止 QEMU
pkill -9 qemu-system-riscv64
```

## 补充文档

- `README_运行说明.md` - 详细的运行说明
- `完整运行脚本.sh` - 自动化运行脚本
- `test_run.sh` - 快速测试脚本

## 进一步阅读

要深入了解日志系统，查看：
- `os/src/logging.rs` - 日志初始化代码
- `os/Cargo.toml` - log crate 依赖配置

---

**问题状态**: ✅ 已解决
**最后更新**: 2026-02-12
**关键点**: OS 一直都在正常运行，只是需要启用 LOG 才能看到输出
