# SYSCALL 日志级别使用说明

日期：2026/3/1

## 概述

为了更好地区分**系统调用相关的调试信息**和**普通的内核调试信息**，我们新增了一个 `syscall` 日志级别，位于 `INFO` 和 `WARN` 之间。

## 日志级别层次

```
ERROR (10)  - 错误信息（红色）
  ↓
WARN (20)   - 警告信息（亮黄色）
  ↓
SYSCALL (25) - 系统调用信息（青色）★ 新增
  ↓
INFO (30)   - 普通信息（蓝色）
  ↓
DEBUG (40)  - 调试信息（绿色）
  ↓
TRACE (50)  - 详细追踪（暗灰色）
```

## 使用方法

### 1. 在代码中使用 `syscall!` 宏

```rust
// 在系统调用函数中
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    syscall!("sys_write pid={} fd={} buf={:#x} len={}", pid, fd, buf as usize, len);
    // ...
}
```

### 2. 通过环境变量控制日志级别

```bash
# 只显示系统调用日志及以上级别（SYSCALL, WARN, ERROR）
LOG=SYSCALL make run

# 显示普通信息及以上级别（INFO, SYSCALL, WARN, ERROR）
LOG=INFO make run

# 显示所有调试信息
LOG=DEBUG make run

# 显示所有追踪信息（包括非系统调用的 trace）
LOG=TRACE make run
```

### 3. 典型使用场景

#### 场景 1：只关注系统调用

```bash
LOG=SYSCALL bash run.sh -f sdcard-rv.img -t all > syscall.log
```

输出示例：
```
[SYSCALL] sys_openat pid=10 dirfd=-100 path="/dev/null" flags=0x2
[SYSCALL] sys_write pid=10 fd=1 buf=0x3f000 len=13
[SYSCALL] sys_exit pid=10 code=0
```

#### 场景 2：系统调用 + 普通信息

```bash
LOG=INFO bash run.sh -f sdcard-rv.img -t all > info.log
```

输出包含：
- `[INFO]` 普通信息
- `[SYSCALL]` 系统调用
- `[WARN]` 警告
- `[ERROR]` 错误

#### 场景 3：完整调试（包括 trace）

```bash
LOG=TRACE bash run.sh -f sdcard-rv.img -t all > trace.log
```

输出包含所有级别的日志。

## 与旧代码的兼容性

### 已替换的日志

所有形如 `trace!("kernel:pid[...]")` 的系统调用日志已自动替换为 `syscall!("kernel:pid[...]")`，共 48 处：

- `os/src/syscall/process.rs`：系统调用实现
- `os/src/syscall/fs.rs`：文件系统调用
- `os/src/syscall/mod.rs`：统一的系统调用日志记录

### 未改变的 trace!

普通的 `trace!` 调用（非系统调用相关）仍然保持不变，只在 `LOG=TRACE` 时显示。

## 技术实现

### 核心实现

位于 `os/src/logging.rs`：

```rust
pub const SYSCALL_LEVEL: u8 = 25;

#[macro_export]
macro_rules! syscall {
    ($($arg:tt)*) => {
        if $crate::logging::syscall_enabled() {
            println!("\u{1B}[36m[SYSCALL] {}\u{1B}[0m", format_args!($($arg)*));
        }
    };
}
```

### 颜色编码

| 级别 | 颜色 | ANSI 代码 |
|------|------|-----------|
| ERROR | 红色 | 31 |
| WARN | 亮黄色 | 93 |
| SYSCALL | 青色 | 36 ★ |
| INFO | 蓝色 | 34 |
| DEBUG | 绿色 | 32 |
| TRACE | 暗灰色 | 90 |

## 常见问题

### Q1: 为什么不直接用 INFO 级别？

A: 系统调用日志非常频繁，与普通的 INFO 日志混在一起会导致：
- 难以过滤纯系统调用日志
- 日志量过大，影响性能
- 不方便分析系统调用序列

### Q2: SYSCALL 级别会影响性能吗？

A: 不会。当 `LOG` 环境变量设置为 `OFF/ERROR/WARN` 时，`syscall_enabled()` 返回 `false`，宏内部的代码完全不会执行。

### Q3: 如何同时记录系统调用和文件操作？

A: 使用 `LOG=INFO` 或更高级别，然后用 `grep` 过滤：

```bash
LOG=INFO make run 2>&1 | grep -E "\[SYSCALL\]|\[fs\]" > filtered.log
```

## 总结

新增的 `syscall` 日志级别提供了：
- ✅ **精确控制**：单独开启/关闭系统调用日志
- ✅ **性能优化**：按需启用，不影响关闭时的性能
- ✅ **可读性**：青色高亮，易于区分
- ✅ **兼容性**：不影响现有的 trace/info/debug 日志

---

*最后更新：2026/3/1*
