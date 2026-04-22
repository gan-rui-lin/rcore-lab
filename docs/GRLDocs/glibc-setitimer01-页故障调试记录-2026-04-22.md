# glibc setitimer01 页故障调试记录（2026-04-22）

**问题类型**：glibc LTP 单测卡死 / 页故障

**影响范围**：RISC-V 平台上的 `glibc/ltp/testcases/bin/setitimer01`

**结论先行**：本次问题并不是 `setitimer()` 本身没有触发，而是 signal handler 返回时，内核在 RISC-V 路径上错误地接受了一个不可靠的 `sa_restorer` 地址，导致 handler 退出后跳转到 `0x2000000`，最终在 `sepc=0x2000000` 处触发页故障。LoongArch 上没有同样问题，是因为其信号返回路径默认使用内核 trampoline，不依赖用户态 `sa_restorer`。

---

## 1. 现象描述

在执行如下命令时：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/setitimer01 LOG=INFO bash run.sh | tee glibc-itimer-info.log
```

内核日志最终出现：

```text
[ERROR] [kernel] trap_handler: page fault addr=0x2000000 sepc=0x2000000 ra=0x2000000 sp=0x7fffffb08 tp=0x600133e80
```

这一条日志非常关键，因为 `sepc` 和 `ra` 同时落在 `0x2000000`，说明 CPU 并不是在正常执行用户代码，而是在“准备从 signal handler 返回”的时候跳到了一个错误地址。也就是说，真正出错点在 signal return 链路，而不是 `setitimer` 触发本身。

同时，用户观察到一个重要现象：

1. **musl 下的对应测例能过**。
2. **glibc 下不行**。
3. **LoongArch 上 glibc 测例能过**。

这组对照信息提示：问题与 libc ABI、架构分支以及 signal 返回实现有关，而不是单纯的 timer 逻辑错误。

---

## 2. 复现与排查路径

### 2.1 复现命令

最小复现命令为：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/setitimer01 LOG=INFO bash run.sh
```

后续为了确认修复后的稳定性，又运行了：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/setitimer01 LOG=OFF bash run.sh | tee glibc-itimer-info.log
```

最终退出码为 `0`，说明修复后该单测可以完成。

### 2.2 先排除 timer 不触发的假设

`setitimer01` 的源码位于：

`testsuits-for-oskernel/ltp-full-20240524/testcases/kernel/syscalls/setitimer/setitimer01.c`

这个测试会对 `ITIMER_REAL / ITIMER_VIRTUAL / ITIMER_PROF` 做验证，并在 child 中安装 signal handler，随后让 handler 多次被触发并返回。测试逻辑的关键点是：

1. 调 `sys_setitimer()` 设定定时器。
2. 安装 `SIGALRM / SIGVTALRM / SIGPROF` 处理函数。
3. 在 handler 中累计信号次数。
4. 等待信号不断触发后，最终让子进程退出。

也就是说，这个测试必须经过“handler 返回”的路径。若 handler 返回路径有问题，就会表现为卡死或页故障。

### 2.3 对照不同架构和 libc

对照结果表明：

1. **LoongArch**：glibc 测例能通过。
2. **RISC-V + musl**：对应测例能通过。
3. **RISC-V + glibc**：会出现 `sepc/ra=0x2000000` 的页故障。

这说明问题并不出在 `setitimer` 的核心定时器数据结构上，因为同样的 timer 机制在别的组合上能正常运行；问题更可能出在 **glibc 的 signal ABI 细节** 和 **RISC-V 的 signal return 决策** 上。

---

## 3. 根因分析

### 3.1 根因一句话

RISC-V 信号投递路径中，内核过早、过宽松地接受了用户态传入的 `sa_restorer`，导致 signal handler 返回时跳转到不可用地址 `0x2000000`，从而触发页故障。

### 3.2 为什么会跳到 0x2000000

在信号投递时，内核会把 trap context 改写成：

1. `sepc = handler`，让用户态先进入 signal handler。
2. `ra = restorer` 或者 `ra = kernel trampoline`，让 handler 返回时有一个“收尾动作”。

RISC-V 上原先的逻辑对 `sa_restorer` 的判定偏启发式：只要看起来像个“像样的地址”就可能被接受。但 glibc 动态程序里的 `sa_restorer` 并不总是一个可直接使用的绝对地址，它可能是：

1. 未重定位的偏移值；
2. 不是用户映射中的有效地址；
3. 缺少 `SA_RESTORER` 语义支撑的值。

一旦内核把它直接塞进 `ra`，handler 执行完就会跳向这个错误位置。`0x2000000` 这个地址本身就很像“错误地接受了一个异常低或未正确重定位的值”之后，最终转化出来的跳转目标。

### 3.3 为什么 LoongArch 没有卡死

LoongArch 分支的信号返回策略和 RISC-V 不同。它直接走内核侧 trampoline，不依赖用户态 `sa_restorer`。因此即使用户态传入了复杂或不可靠的 restorer 信息，也不会把 return 链路带到一个错误的用户地址上。

这就是为什么相同的 glibc 测例在 LA 上能通过，而在 RV 上会暴露这个问题。

### 3.4 为什么 musl 没有触发

musl 的对应测例没有暴露同样的问题，说明它的 signal action 组合、restorer 取值、或者调用路径没有触发到这个脆弱分支。换句话说，问题不是“所有 libc 都会坏”，而是 “glibc 更容易走到这条有 ABI 细节的返回路径”，因此更容易暴露出内核对 restorer 的错误接受。

---

## 4. 代码级定位

### 4.1 原始实现的问题点

问题主要出在 `os/src/task/mod.rs` 的 `handle_signals()` 中 RISC-V 分支：

```rust
// 原先的思路是用阈值 heuristic 判断 restorer 是否“看起来像有效地址”
let use_restorer = action.restorer != 0
    && action.restorer < USER_ADDR_MAX
    && action.restorer >= 0x10000;
```

这个判断有两个明显缺陷：

1. **没有确认 `SA_RESTORER` 是否真的被设置**。
2. **没有确认该地址是否真的在当前进程页表中可达**。

只靠“地址大于某个阈值”判断，很容易把错误值当成合法值。

### 4.2 修复后的策略

本次修复将判定升级为：

1. 必须显式设置 `SA_RESTORER`。
2. `action.restorer` 必须在用户地址空间范围内。
3. 页表中必须真的能翻译到该页。
4. PTE 必须有 `U` 位，并且页属性至少可读或可执行。

否则，一律回退到内核固定的 `SIG_RETURN_ADDR + sigreturn_trampoline_offset()`。

这个策略的意义在于：**内核不再猜测 glibc 想做什么，而是只接受“明确声明 + 实际可达”的 restorer。**

### 4.3 相关常量补充

为了让代码更清晰，本次在 `os/src/task/action.rs` 中补充了：

```rust
pub const SA_RESTORER: usize = 0x04000000;
```

这使得内核在判断时可以直接表达“是否真的启用了 restorer 语义”。

---

## 5. 修复策略与语义对齐

### 5.1 修复目标

修复目标不是“强行兼容所有奇怪的 restorer 地址”，而是让行为更接近 Linux：

1. glibc 正常情况可以走用户态 restorer。
2. restorer 无效时，内核应安全地回退到 trampoline。
3. 不允许 handler 返回到一个不可达地址并把进程直接送进页故障。

### 5.2 为什么这样更安全

如果内核不验证 restorer，就等于让用户态能把返回地址任意塞进 `ra`。这不仅导致页故障，还可能引入更复杂的安全问题。修复后：

1. 只有合法映射的用户页才能作为返回路径。
2. 对 glibc 动态程序中未重定位/暂不可用的地址，会自动回退。
3. LoongArch 维持原有稳定路径，不会受到影响。

---

## 6. 验证过程与结果

### 6.1 静态检查

在修改后对相关文件进行了错误检查，触发过一次格式错误，随后已修正。当前相关文件没有新的编译错误。

涉及文件：

- `os/src/task/action.rs`
- `os/src/task/mod.rs`

### 6.2 回归验证

执行了单测验证：

```bash
SINGLE_TEST=/glibc/ltp/testcases/bin/setitimer01 LOG=OFF bash run.sh | tee glibc-itimer-info.log
```

结果：退出码为 `0`。

这说明本次修复至少已经消除了该单测在当前环境下的页故障/卡死表现。

### 6.3 观察到的现象变化

修复前：

```text
[ERROR] [kernel] trap_handler: page fault addr=0x2000000 sepc=0x2000000 ra=0x2000000 sp=0x7fffffb08 tp=0x600133e80
```

修复后：

1. 该页故障不再出现。
2. `setitimer01` 单测能够正常结束。
3. 说明 signal handler 返回链路恢复为可用路径。

---

## 7. 经验总结

这次问题有一个很典型的特征：**只有某些 libc、某些架构、某些定时器测例组合才会暴露。**

这类问题最容易误判成“定时器没触发”或“signal 没送达”，但实际上真正的断点通常在：

1. 用户态 handler 是否进入；
2. handler 返回时走哪条路径；
3. `ra/restorer` 是否真的可达；
4. 架构分支是否一致处理了 trampoline。

本次案例里，`setitimer01` 恰好持续触发 signal 并反复返回，所以它不是 timer 逻辑的“普通单测”，而是一个非常好的“signal-return ABI 探针”。

---

## 8. 后续建议

1. **给 RISC-V 信号返回路径补更多回归**：尤其是 glibc 下的 `setitimer01 / setitimer02`、`sigaction`、`signal` 类测试。
2. **继续收紧 restorer 校验**：必要时可进一步结合页权限、可执行位或 trampoline 地址白名单，降低误接受概率。
3. **增加调试日志开关**：对于 signal 投递、restorer 选择、sigreturn 路径，可以在 debug 模式下打印更明确的分支选择信息，方便后续排障。
4. **将本次经验沉淀为 LTP 调试模板**：凡是“glibc-only、架构相关、handler 返回相关”的卡死，都优先检查 `sa_restorer`、`sigreturn` 和 trampoline。

---

## 9. 结论

本次 `glibc setitimer01` 的问题，本质上是 **RISC-V 信号返回地址选择过宽松** 导致的页故障，而不是 `setitimer()` 或 timer interrupt 本身失效。修复后，内核只有在 `SA_RESTORER` 明确存在且地址真的可达时才使用用户态 restorer，否则就稳妥地回退到内核 trampoline。这个改动既解释了为什么 LA 没有卡死，也解释了为什么 musl 没有触发，而 glibc 在 RV 上会暴露问题。
