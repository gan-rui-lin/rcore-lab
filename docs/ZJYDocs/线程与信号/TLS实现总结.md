# TLS (Thread Local Storage) 实现总结

## ✅ 已完成的工作

### 1. 完整的 PT_TLS 支持实现

#### 新增文件
- **`os/src/task/tls.rs`**: 完整的 TLS 模块
  - `TlsInfo`: 存储 ELF PT_TLS 段信息
  - `TlsArea`: 管理 TLS 区域（数据 + TCB）
  - RISC-V Variant I 布局实现
  - 支持 fork 时复制 TLS

#### 修改文件
- **`os/src/task/mod.rs`**: 导出 TLS 类型
- **`os/src/mm/memory_set.rs`**: ELF 加载器增强
  - `from_elf` 现在返回 `(MemorySet, usize, usize, Option<TlsInfo>)`
  - 扫描并解析 PT_TLS 段
  - 添加详细的 ELF 段日志

- **`os/src/task/process.rs`**: Process 增强
  - `ProcessControlBlockInner` 添加 `tls_area: Option<TlsArea>` 字段
  - `new()`: 初始化时处理 TLS
  - `exec()`: exec 时重新初始化 TLS
  - `fork()`: fork 时复制 TLS

- **`os/src/trap/context.rs`**: TrapContext 支持 tp 寄存器设置

### 2. Workaround: 最小 TCB 实现

对于没有 PT_TLS 段的程序（如当前的 busybox），实现了最小 TCB 分配：
- 在固定地址 `0x70001000` 分配一页内存
- 初始化 TCB：`[dtv=0, self_ptr=tcb_addr]`
- 设置 `tp` 寄存器指向 TCB

## 📊 测试结果

### 发现的关键信息

1. **initcode 和 busybox 都没有 PT_TLS 段**：
   ```
   [ INFO] [ELF] Scanning 5 program headers for PT_TLS
   [ INFO] [ELF] No PT_TLS segment found
   ```

2. **tp 寄存器现在正确设置**：
   ```
   Before: tp (x4) = 0x0          ❌ 空指针
   After:  tp (x4) = 0x70001000   ✅ 指向 TCB
   ```

3. **busybox 仍然崩溃**：
   ```
   [kernel] trap_handler: Exception(InstructionPageFault) in application
     bad addr (stval) = 0x0
     bad instruction (sepc) = 0x0
     Registers:
       ra (x1) = 0x104a7c
       tp (x4) = 0x70001000  ← 现在是有效的！
   ```

## ❌ 剩余问题

### 问题：busybox 仍然跳转到地址 0x0

**可能原因**：

1. **未初始化的函数指针**
   - musl libc 可能期望 TLS 中有特定的函数指针
   - 这些函数指针未被初始化，导致间接调用到 0x0

2. **缺少动态链接器初始化**
   - 如果 busybox 依赖动态链接器（ld-musl）
   - 需要正确设置辅助向量（auxiliary vector）
   - 需要实现 `AT_` 类型（AT_PHDR, AT_PHENT, AT_PHNUM 等）

3. **其他未实现的系统调用**
   - 可能还有其他系统调用在 `set_tid_address` 后被调用
   - 建议添加更详细的系统调用跟踪

4. **TLS 变量访问方式**
   - 静态 TLS (Initial-exec, Local-exec)
   - 动态 TLS (Global-dynamic, Local-dynamic)
   - 可能需要支持 TLS 描述符或 `__tls_get_addr`

## 🔍 调试建议

### 1. 检查 busybox 的链接方式
```bash
file /path/to/busybox
readelf -l /path/to/busybox | grep TLS
readelf -d /path/to/busybox | grep NEEDED
```

### 2. 添加更详细的系统调用跟踪
在 `os/src/syscall/mod.rs` 的未实现系统调用部分添加：
```rust
_ => {
    known = false;
    error!(
        "{} {}: unimplemented syscall {} ({}) at sepc={:#x}",
        pid,
        name,
        syscall_id,
        syscall_name(syscall_id),
        current_trap_cx().sepc  // 添加调用位置
    );
    -ENOSYS
},
```

### 3. 使用 GDB 调试
```bash
bash run.sh -d
# 在另一个终端：
riscv64-unknown-elf-gdb -ex 'file target/riscv64gc-unknown-none-elf/debug/os' \
                        -ex 'target remote localhost:1234' \
                        -ex 'b *0x104a7c'  # 在 ra 地址设置断点
```

### 4. 检查辅助向量
musl libc 需要从栈上读取辅助向量。检查 `exec` 是否正确设置了：
- argc
- argv[]
- envp[]
- auxv[] (AT_PHDR, AT_ENTRY, AT_PHNUM 等)

### 5. 尝试更简单的测试程序
创建一个最小的 musl 程序来测试 TLS：
```c
#include <stdio.h>

__thread int tls_var = 42;

int main() {
    printf("TLS var = %d\n", tls_var);
    return 0;
}
```

## 📝 RISC-V TLS 布局（已实现）

```
High address
+------------------+
|  TCB[1]: self    | <- tp points here (0x70001000)
+------------------+
|  TCB[0]: dtv     |
+------------------+
|  .tbss (zero)    | (for PT_TLS programs)
+------------------+
|  .tdata (init)   | (for PT_TLS programs)
+------------------+
Low address
```

## 🎯 下一步行动

1. **短期**：添加辅助向量（auxv）支持到 exec
2. **中期**：实现更多 musl 需要的系统调用
3. **长期**：支持动态链接器和完整的 TLS 模型

## 📚 参考资料

- [RISC-V ELF psABI](https://github.com/riscv-non-isa/riscv-elf-psabi-doc)
- [ELF TLS Specification](https://www.akkadia.org/drepper/tls.pdf)
- [musl libc source](https://git.musl-libc.org/cgit/musl/)
- [Linux Auxiliary Vectors](https://man7.org/linux/man-pages/man3/getauxval.3.html)

## 总结

我们成功实现了完整的 PT_TLS 支持框架，并且为没有 PT_TLS 的程序提供了最小 TCB workaround。`tp` 寄存器现在正确设置，这是一个重大进展。但 busybox 仍然崩溃，可能需要更多的系统调用支持或辅助向量实现才能完全运行。
