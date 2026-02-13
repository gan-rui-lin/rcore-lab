# TLS (Thread Local Storage) 实现计划

## 背景
busybox (musl libc) 需要完整的 TLS 支持才能正常运行。当前内核只加载 PT_LOAD 段，忽略了 PT_TLS 段。

## 需要实现的功能

### 1. ELF 加载器增强
文件：`os/src/mm/memory_set.rs:178-251`

```rust
pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize, Option<TlsInfo>) {
    // ... 现有的 PT_LOAD 处理代码 ...

    // 新增：处理 PT_TLS 段
    let mut tls_info = None;
    for i in 0..ph_count {
        let ph = elf.program_header(i).unwrap();
        if ph.get_type().unwrap() == xmas_elf::program::Type::Tls {
            tls_info = Some(TlsInfo {
                start_addr: ph.virtual_addr() as usize,
                filesz: ph.file_size() as usize,
                memsz: ph.mem_size() as usize,
                align: ph.align() as usize,
            });
            break;
        }
    }

    (memory_set, user_stack_bottom, elf.header.pt2.entry_point() as usize, tls_info)
}
```

### 2. TLS 数据结构
新文件：`os/src/task/tls.rs`

```rust
pub struct TlsInfo {
    pub start_addr: usize,  // TLS 模板的虚拟地址
    pub filesz: usize,      // 文件中的大小（初始化数据）
    pub memsz: usize,       // 内存中的大小（包括 .tbss）
    pub align: usize,       // 对齐要求
}

pub struct TlsArea {
    pub tp_value: usize,    // tp 寄存器的值
    pub tls_base: usize,    // TLS 区域的基地址
    pub tls_size: usize,    // TLS 区域的总大小
}

impl TlsArea {
    // RISC-V TLS Variant I: tp 指向 TCB (Thread Control Block)
    // 布局: [TLS 数据] [TCB] <- tp 指向这里
    pub fn new(tls_info: &TlsInfo, memory_set: &mut MemorySet) -> Self {
        let tcb_size = 2 * core::mem::size_of::<usize>();  // 至少两个指针大小
        let total_size = (tls_info.memsz + tcb_size + tls_info.align - 1) & !(tls_info.align - 1);

        // 分配 TLS 区域
        let tls_base = // 选择合适的虚拟地址
        memory_set.insert_framed_area(
            tls_base.into(),
            (tls_base + total_size).into(),
            MapPermission::R | MapPermission::W | MapPermission::U,
        );

        // 复制初始化数据
        // ... 从 tls_info.start_addr 复制 filesz 字节 ...

        // tp 指向 TCB
        let tp_value = tls_base + tls_info.memsz + tcb_size;

        Self {
            tp_value,
            tls_base,
            tls_size: total_size,
        }
    }
}
```

### 3. Process 结构体增强
文件：`os/src/task/process.rs`

```rust
pub struct ProcessControlBlockInner {
    // ... 现有字段 ...
    pub tls_area: Option<TlsArea>,  // 新增：TLS 区域信息
}
```

### 4. exec 系统调用增强
文件：`os/src/task/process.rs:141-216`

```rust
pub fn exec(&self, elf_data: &[u8], args: Vec<String>, envs: Vec<String>) {
    // ... 现有代码 ...

    let (memory_set, ustack_bottom, entry_point, tls_info) = MemorySet::from_elf(elf_data);

    // 新增：设置 TLS
    let tls_area = tls_info.map(|info| TlsArea::new(&info, &mut memory_set));

    // ... 设置栈、参数等 ...

    let mut trap_cx = TrapContext::app_init_context(/*...*/);

    // 新增：设置 tp 寄存器
    if let Some(ref tls) = tls_area {
        trap_cx.x[4] = tls.tp_value;  // tp = x4
    }

    // 保存 TLS 信息
    process_inner.tls_area = tls_area;
}
```

### 5. fork 时复制 TLS
文件：`os/src/task/process.rs:218-283`

```rust
pub fn fork(self: &Arc<Self>) -> Arc<Self> {
    let mut parent = self.inner_exclusive_access();

    // 复制 TLS 区域
    let tls_area = parent.tls_area.as_ref().map(|parent_tls| {
        TlsArea::new_from_parent(parent_tls, &mut child_memory_set)
    });

    // ... 其他复制逻辑 ...
}
```

## RISC-V TLS 布局规范

```
高地址
+------------------+
|  TCB (Thread     |
|  Control Block)  | <- tp (x4) 指向这里
+------------------+
|  .tdata (初始化) |
|  .tbss (未初始化)|
+------------------+
低地址
```

## 参考资料
- RISC-V ELF psABI: https://github.com/riscv-non-isa/riscv-elf-psabi-doc
- musl TLS 实现: https://git.musl-libc.org/cgit/musl/tree/src/internal
- ELF TLS 规范: https://www.akkadia.org/drepper/tls.pdf

## 测试步骤
1. 实现上述修改
2. 编译运行
3. 检查 tp 寄存器是否正确设置
4. 验证 busybox 能否正常执行

## 预期难度
⭐⭐⭐⭐ (高难度)
需要深入理解 ELF 格式、TLS 规范和 RISC-V ABI
