# GDB 调试指南（rcore-lab）

这份文档总结在 QEMU + GDB 下调试内核与用户态的基本流程、常用命令，以及常见坑的处理方式。

## 1. 编译与启动

推荐使用调试模式：

```bash
make debug
```

启动 QEMU（通常由 `run.sh` 或 IDE 配置完成），再用 gdb 连接到 QEMU 的 gdb server。

## 2. 加载符号

### 2.1 内核符号

```gdb
(gdb) symbol-file kernel-qemu
# 或
(gdb) add-symbol-file kernel-qemu 0x80200000
```

`0x80200000` 来自 `os/src/linker.ld` 的 `BASE_ADDRESS`。

验证符号是否存在：

```gdb
(gdb) info functions trap_handler
```

### 2.2 用户态符号（busybox）

**注意：**busybox 的 ELF VMA 是 `0x10120`（不是 0x8040xxxx）。

```gdb
(gdb) add-symbol-file /home/grl/codeRepo/rcore-lab/busybox/musl/busybox 0x10120
```

验证：

```gdb
(gdb) info symbol 0x104a7c
```

如果能解析出 `__libc_start_init` 等符号，说明加载正确。

## 3. Rust 符号与语言模式

GDB 对 Rust 符号有时需要指定语言：

```gdb
(gdb) set language rust
```

若仍无法解析，使用引号包裹：

```gdb
(gdb) info functions 'os::trap::trap_handler'
```

## 4. 用户态虚拟地址无法直接读

在内核上下文下，GDB 使用内核页表，直接 `x/i` 用户 VA 会失败：

```gdb
(gdb) x/6i $sepc
# Cannot access memory at address ...
```

### 解决方法：先翻译 VA -> PA

本工程提供调试入口：

```rust
#[no_mangle]
#[link_section = ".text.keep"]
pub extern "C" fn debug_user_va_to_pa(va: usize) -> usize { ... }
```

GDB 使用方法：

```gdb
(gdb) set $va = os::task::processor::current_trap_cx().sepc
(gdb) p/x debug_user_va_to_pa($va)
$1 = 0x82xxxxxx
(gdb) x/6i 0x82xxxxxx
```

> 如果 `debug_user_va_to_pa` 找不到符号：
> 1) 确认 `make debug` 重编译
> 2) `info functions debug_user_va_to_pa`
> 3) 确认它被放入 `.text.keep`，避免 `--gc-sections` 裁剪

## 5. 常用调试点

### 5.1 查看 TrapContext

```gdb
(gdb) set $cx = os::task::processor::current_trap_cx()
(gdb) p/x $cx.sepc
(gdb) p/x $cx.x[1]   # ra
(gdb) p/x $cx.x[2]   # sp
(gdb) p/x $cx.x[3]   # gp
(gdb) p/x $cx.x[4]   # tp
```

### 5.2 反推调用点

```gdb
(gdb) set $ra = $cx.x[1]
(gdb) set $call = $ra - 4
(gdb) p/x debug_user_va_to_pa($call)
(gdb) x/4i debug_user_va_to_pa($call)
```

如果看到 `jalr a5`，继续检查 `a5`：

```gdb
(gdb) p/x $cx.x[15]
```

## 6. 常见坑

- **`add-symbol-file` 基址错误**：用户态符号按 ELF VMA 加载（busybox 是 `0x10120`）。
- **Rust `impl` 路径在 GDB 里不可用**：例如 `<impl ...>::from` 会导致 `unexpected token`。
- **用户态 VA 读不到不是错**：需要 VA->PA 翻译。
- **函数被裁剪**：调试入口需要 `#[link_section = ".text.keep"]`。

## 7. 附：检查 ELF 信息

```bash
readelf -l busybox/musl/busybox
readelf -S busybox/musl/busybox
nm -n busybox/musl/busybox | rg __global_pointer\$
```

