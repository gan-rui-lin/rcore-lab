# GDB 调试记录：dump_tasks 与符号加载

本文记录一次在 QEMU + GDB 下调试内核卡死的过程，重点是如何在 GDB 中调用内核函数以及遇到的坑。

## 1. dump_tasks 入口与符号保留

为了在 GDB 中直接打印任务状态，新增了一个调试入口：
- `#[no_mangle] pub extern "C" fn dump_tasks()`
- 放入 `.text.keep` 并在 `linker.ld` 中 `KEEP(*(.text.keep))`

这样即使启用 `--gc-sections`，函数也不会被裁剪。

## 2. GDB 符号加载方式

**目标**：让 GDB 能解析 `dump_tasks` 符号。

操作步骤：
```
(gdb) symbol-file
(gdb) add-symbol-file kernel-qemu 0x80200000
```
其中 `0x80200000` 与 `linker.ld` 中的 `BASE_ADDRESS` 一致。

确认符号是否存在：
```
(gdb) info functions dump_tasks
```

如果 `info functions` 能看到，但 `call dump_tasks()` 仍提示找不到，通常是语言模式问题。

## 3. 在 GDB 中调用 dump_tasks

**推荐写法**：
```
(gdb) set language rust
(gdb) call os::task::dump_tasks()
# 或者
(gdb) call 'os::task::dump_tasks'()
```

**保证可用的硬调用方式**（按地址）：
```
(gdb) set language c
(gdb) call ((void (*)())0x80202000)()
```
其中 `0x80202000` 可通过 `rust-objdump -t kernel-qemu | rg dump_tasks` 获取。

## 4. 已知坑：BorrowMutError

从 GDB 直接调用 `dump_tasks()` 可能触发：
```
[kernel] Panicked at src/sync/up.rs:32 already borrowed: BorrowMutError
```
原因是：
- `dump_tasks` 内部会访问 `UPSafeCell` 保护的数据结构
- 如果调用时内核已经持有这些 RefCell 的借用，就会 panic

处理方式：
1. **在安全点调用**（例如任务调度循环空闲处）。
2. **使用 try-borrow**：`dump_tasks` 内部改用 `try_exclusive_access()`，避免直接 panic。
3. **必要时移除 RefCell 借用检查**：将 `UPSafeCell` 改为 `UnsafeCell`（返回 `&mut T`），避免被中断/重入打断时触发 BorrowMutError。

## 5. 结果

调用成功后，会看到类似输出：
```
[kernel] ===== task dump =====
[kernel] current: pid=1 tid=0 status=Running
[kernel] ready_queue_len=0
...
```

这可以帮助判断系统是否进入“所有进程睡死”的状态。
