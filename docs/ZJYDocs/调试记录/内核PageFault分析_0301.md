# 内核态 InstructionPageFault 分析文档

日期：2026/3/1

## 问题现象

在运行pthread_cancel测试时，内核发生panic：

```
[ INFO] [exit] pid=34 tid=2 clear_child_tid wake addr=0x9db80 pa=0x8294db80 woke_private=0 woke_shared=0
...
[kernel] Panicked at src/trap/mod.rs:535 Unsupported trap from kernel: Exception(InstructionPageFault), stval = 0xffffffffffd1f618!
```

**关键信息**：
- **scause**: Exception(InstructionPageFault) - 指令页错误
- **stval**: 0xffffffffffd1f618 = -3017192（作为有符号整数）
- **trap来源**: 内核态（trap_from_kernel）
- **触发时机**: 线程tid=2退出并执行clear_child_tid之后

## 问题分析

### 1. stval地址分析

`0xffffffffffd1f618` 是一个非常大的地址（接近64位地址空间的顶端），作为有符号数解释为 `-0x2e09e8` = `-3017192`。

这个地址的特点：
- **不是有效的用户态代码地址**（用户态一般在 0x0～0x4000_0000）
- **不是有效的内核态代码地址**（内核代码一般在 0x8000_0000 以上）
- **看起来像是一个被破坏的指针或相对偏移量**

### 2. panic触发路径

从panic的位置 [os/src/trap/mod.rs:534-540](../../os/src/trap/mod.rs#L534-L540) 可以看到：

```rust
fn trap_from_kernel(_trap_cx: &context::KernelTrapContext) {
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Interrupt(Interrupt::SupervisorExternal) => { ... }
        Trap::Interrupt(Interrupt::SupervisorTimer) => { ... }
        _ => {
            panic!(
                "Unsupported trap from kernel: {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
}
```

**关键点**：`trap_from_kernel` 只处理两种内核态中断（SupervisorExternal和SupervisorTimer），对于其他所有trap（包括异常）都会panic。

### 3. 为什么会从内核态触发InstructionPageFault

InstructionPageFault的stval寄存器保存的是**触发fault的指令地址（PC）**，不是访问的数据地址。

因此，这个panic说明：**在内核态执行过程中，PC被设置为了0xffffffffffd1f618，导致取指令时发生页错误**。

可能的原因：

#### 原因A：函数返回地址（ra）被破坏

在RISC-V中，函数返回使用`ret`指令，实际上是`jalr x0, x1, 0`（跳转到ra寄存器）。如果ra被破坏为0xffffffffffd1f618，函数返回时就会触发InstructionPageFault。

**可能的破坏场景**：
1. **内核栈溢出**：栈上保存的ra被覆盖
2. **任务切换时TaskContext损坏**：`__switch`从损坏的TaskContext恢复ra
3. **信号处理破坏了内核栈**：在handle_signals或sys_sigreturn中栈操作错误

#### 原因B：任务切换时TaskContext中的ra字段损坏

从日志看，panic发生在`clear_child_tid`之后，此时会调用`exit_current_and_run_next`切换到下一个任务。

切换流程（[os/src/task/mod.rs:280-281](../../os/src/task/mod.rs#L280-L281)）：
```rust
let mut _unused = TaskContext::zero_init();
schedule(&mut _unused as *mut _);
```

然后调用`__switch`（[os/src/task/switch.S:10-33](../../os/src/task/switch.S#L10-L33)）：
```assembly
__switch:
    # 保存当前任务的ra、sp、s0-s11
    sd ra, 0(a0)
    sd sp, 8(a0)
    ...
    # 恢复下一个任务的ra、sp、s0-s11
    ld ra, 0(a1)      # 从next_task_cx_ptr加载ra
    ld sp, 8(a1)
    ...
    ret               # 跳转到ra
```

**如果下一个任务的TaskContext中ra字段被破坏为0xffffffffffd1f618，则ret会跳转到该地址，触发InstructionPageFault。**

#### 原因C：信号处理相关的PC破坏

pthread_cancel测试涉及SIGCANCEL（信号33）的处理。在信号处理流程中：

1. **handle_signals设置PC**（[os/src/task/mod.rs:563](../../os/src/task/mod.rs#L563)）：
   ```rust
   trap_cx.sepc = action.handler;
   ```

2. **sys_sigreturn恢复PC**（[os/src/syscall/process.rs:1745-1747](../../os/src/syscall/process.rs#L1745-L1747)）：
   ```rust
   restored.sepc = ucontext.uc_mcontext.gregs[0];
   restored.x[1..].copy_from_slice(&ucontext.uc_mcontext.gregs[1..]);
   *inner.get_trap_cx() = restored;
   ```

**如果用户态的`action.handler`或`ucontext.uc_mcontext.gregs[0]`被破坏，就会导致PC被设置为错误的值。**

但这里有一个问题：信号处理修改的是**TrapContext（用户态trap context）**，而不是**TaskContext（内核态task context）**。TrapContext中的sepc会在`trap_return`中通过`sret`指令跳转到用户态，此时应该触发**用户态**的InstructionPageFault，不应该触发**内核态**的InstructionPageFault。

**因此，问题更可能是TaskContext的ra被破坏，而不是TrapContext的sepc被破坏。**

### 4. 内核trap处理的启用状态

用户在 [os/src/trap/mod.rs:42-45](../../os/src/trap/mod.rs#L42-L45) 注释掉了`set_kernel_trap_entry()`，想要禁用内核态trap处理：

```rust
pub fn init() {
    // ! 不让内核处理中断
    // set_kernel_trap_entry();
}
```

**但是**，在 [os/src/trap/mod.rs:111](../../os/src/trap/mod.rs#L111) 的`trap_handler`中，每次从用户态进入时都会调用`set_kernel_trap_entry()`：

```rust
pub fn trap_handler() -> ! {
    set_kernel_trap_entry();  // 这里重新启用了内核态trap处理！
    ...
}
```

因此，**内核态trap处理实际上是启用的**，只是在内核启动时（init阶段）是禁用的。这解释了为什么会捕获到内核态的InstructionPageFault并panic。

## 调试策略

### 策略1：添加日志追踪TaskContext

在关键位置添加日志，追踪TaskContext的ra字段：

1. **exit_current_and_run_next开始时**：
   ```rust
   info!("[exit] current task_cx ra={:#x} sp={:#x}",
         task_inner.task_cx.ra, task_inner.task_cx.sp);
   ```

2. **schedule切换前**：
   ```rust
   info!("[schedule] switching to next_task_cx ra={:#x} sp={:#x}",
         next_task_cx.ra, next_task_cx.sp);
   ```

3. **__switch返回后**（如果可以到达）：
   ```rust
   info!("[switch] returned, current ra={:#x}", ...);
   ```

### 策略2：检查ready queue中任务的TaskContext

在panic发生前，定期检查ready queue中所有任务的TaskContext是否合法：

```rust
for task in ready_queue_snapshot() {
    let task_inner = task.inner_exclusive_access();
    if task_inner.task_cx.ra == 0 || task_inner.task_cx.ra > 0x8fff_ffff_ffff_ffff {
        error!("[check] Invalid task_cx.ra={:#x} pid={} tid={}",
               task_inner.task_cx.ra, ...);
    }
}
```

### 策略3：使用GDB调试

按照`pthread_cancel_pc_analysis`计划文档的步骤：

1. **启动QEMU with GDB stub**：
   ```bash
   LOG=INFO bash run.sh -t debug -f sdcard-final.img -d > debug.log 2>&1 &
   ```

2. **连接GDB**：
   ```bash
   riscv64-unknown-elf-gdb
   (gdb) target remote :1234
   (gdb) set architecture riscv:rv64
   ```

3. **在panic位置前设置断点**：
   ```gdb
   # 在trap_from_kernel入口
   (gdb) b trap_from_kernel

   # 当触发时检查
   (gdb) info registers pc ra sp
   (gdb) x/10i $pc
   (gdb) x/10i $ra
   ```

4. **追踪任务切换**：
   ```gdb
   # 在__switch设置断点
   (gdb) b __switch

   # 查看a0（current_task_cx_ptr）和a1（next_task_cx_ptr）
   (gdb) x/14xg $a0   # 查看current TaskContext
   (gdb) x/14xg $a1   # 查看next TaskContext

   # 检查next的ra是否合法
   (gdb) x/xg $a1     # next->ra
   ```

### 策略4：检查pthread_cancel handler

从busybox中反汇编SIGCANCEL handler（参考计划文档）：

```bash
riscv64-unknown-elf-objdump -d /tmp/busybox --start-address=0x3e134 --stop-address=0x3e200
```

检查handler是否会破坏栈或返回地址。

### 策略5：临时禁用内核态trap处理

在`trap_handler`中注释掉`set_kernel_trap_entry()`，完全禁用内核态trap处理：

```rust
pub fn trap_handler() -> ! {
    // set_kernel_trap_entry();  // 临时禁用
    ...
}
```

**预期结果**：如果问题确实是内核态PC被破坏，禁用内核trap处理后，系统会直接崩溃或陷入死循环，而不是panic。这可以确认问题的根源。

## 可能的修复方向

### 修复A：增强TaskContext的保护

在TaskContext中添加canary值，检测是否被破坏：

```rust
pub struct TaskContext {
    canary: usize,  // 0xdeadbeef
    ra: usize,
    sp: usize,
    s: [usize; 12],
}

impl TaskContext {
    pub fn goto_trap_return(kstack_ptr: usize) -> Self {
        Self {
            canary: 0xdeadbeef,
            ra: trap_return as usize,
            sp: kstack_ptr,
            s: [0; 12],
        }
    }

    pub fn check_canary(&self) -> bool {
        self.canary == 0xdeadbeef
    }
}
```

在`__switch`前检查：
```rust
pub fn schedule(switched_task_cx_ptr: *mut TaskContext) {
    let processor = PROCESSOR.exclusive_access();
    let next_task = processor.take_current().unwrap();
    let next_task_cx_ptr = &next_task.inner_exclusive_access().task_cx as *const _;

    // 检查next_task的TaskContext是否合法
    unsafe {
        let next_ctx = &*next_task_cx_ptr;
        if !next_ctx.check_canary() {
            panic!("TaskContext canary corrupted!");
        }
        if next_ctx.ra == 0 || next_ctx.ra > 0x8fff_ffff_ffff_ffff {
            panic!("TaskContext ra invalid: {:#x}", next_ctx.ra);
        }
    }

    __switch(switched_task_cx_ptr, next_task_cx_ptr);
}
```

### 修复B：修复SIGCANCEL处理

根据`pthread_cancel_pc_analysis`文档的建议，调整SIGCANCEL的处理逻辑：

1. 增加MAX_SIGCANCEL_LOOP阈值（从2增加到10或更多）
2. 检查canceldisable标志，确保符合POSIX语义
3. 确保cancel_handler正确恢复栈和寄存器

### 修复C：检查内核栈大小

如果是内核栈溢出导致，需要增加内核栈大小：

```rust
// os/src/config.rs
pub const KERNEL_STACK_SIZE: usize = 8 * 4096;  // 从4096*2增加到8*4096
```

## 相关代码位置

- **Panic位置**: [os/src/trap/mod.rs:534-540](../../os/src/trap/mod.rs#L534-L540)
- **trap_handler**: [os/src/trap/mod.rs:110-421](../../os/src/trap/mod.rs#L110-L421)
- **trap_from_kernel**: [os/src/trap/mod.rs:469-542](../../os/src/trap/mod.rs#L469-L542)
- **trap_return**: [os/src/trap/mod.rs:428-449](../../os/src/trap/mod.rs#L428-L449)
- **__restore汇编**: [os/src/trap/trap.S:48-72](../../os/src/trap/trap.S#L48-L72)
- **exit_current_and_run_next**: [os/src/task/mod.rs:177-282](../../os/src/task/mod.rs#L177-L282)
- **schedule**: [os/src/task/processor.rs:103-110](../../os/src/task/processor.rs#L103-L110)
- **__switch汇编**: [os/src/task/switch.S:10-33](../../os/src/task/switch.S#L10-L33)
- **TaskContext定义**: [os/src/task/context.rs:6-32](../../os/src/task/context.rs#L6-L32)
- **handle_signals**: [os/src/task/mod.rs:448-583](../../os/src/task/mod.rs#L448-L583)
- **sys_sigreturn**: [os/src/syscall/process.rs:1701-1778](../../os/src/syscall/process.rs#L1701-L1778)

## 总结

**问题核心**：内核态PC被设置为非法地址0xffffffffffd1f618（-3MB），导致InstructionPageFault。

**最可能的原因**：在任务切换（`__switch`）时，下一个任务的TaskContext中的ra字段被破坏。

**触发场景**：pthread_cancel测试，线程退出并执行clear_child_tid后切换到下一个任务。

**调试优先级**：
1. 高优先级：添加TaskContext日志追踪，确认ra破坏的时机和位置
2. 中优先级：使用GDB追踪__switch过程，检查TaskContext内容
3. 低优先级：检查SIGCANCEL handler，排查信号处理的影响

**预期修复**：在找到ra被破坏的根源后，可能需要：
- 修复内存越界写入
- 增加TaskContext的边界检查
- 调整pthread_cancel的处理逻辑
- 增加内核栈大小

---

*最后更新：2026/3/1*
