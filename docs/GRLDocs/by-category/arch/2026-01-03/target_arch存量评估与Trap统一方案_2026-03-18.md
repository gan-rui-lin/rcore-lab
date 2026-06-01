# target_arch 存量评估与 Trap 统一方案（2026/03/18）

## 一、背景与目标

本轮重构的核心目标是继续推进“架构（arch crate）与 OS（os crate）解耦”，减少 OS 业务代码中直接出现的 `#[cfg(target_arch = ...)]`，把“平台差异”尽量下沉到 arch 层或通过更清晰的抽象边界承载。

在当前代码基础上，已经完成了一批关键改造：

1. LoongArch 用户态入口汇编从 Rust 内联裸汇编迁移到独立 `trap.S`，并保留 Rust 侧最小 glue；
2. 页表分配/释放与 kernel token 获取回调统一抽到 `arch/src/pagetable.rs`；
3. `os/src/main.rs` 中 board/net 的架构分支做了第一步收敛（`cfg_attr(path=...)` 形式）；
4. `os/src/trap/mod.rs` 里局部可收敛的 `cfg` 已减少一批。

在此基础上，本次调研旨在回答三个问题：

- 目前第一方代码里还剩多少 `target_arch`；
- 这些存量分布在哪些模块、改造难度如何；
- Trap 统一应走什么路径，哪些能马上做，哪些要分阶段做。

---

## 二、统计口径与结果

### 1）统计口径

- 统计命令聚焦第一方 Rust 源码：`os/src`, `arch/src`, `user/src`；
- 排除 `vendor` 与 `target`；
- 关键字为 `target_arch`（包含 `cfg`、`cfg_attr`、注释中提及）。

### 2）统计结果（核心代码）

- 核心 Rust 源码总命中：**95 处**。
- 高密度文件分布如下（按命中数降序）：

1. `os/src/mm/memory_set.rs`：22
2. `os/src/task/mod.rs`：14
3. `os/src/task/process.rs`：8
4. `os/src/task/processor.rs`：6
5. `os/src/task/action.rs`：6
6. `os/src/drivers/mod.rs`：6
7. `os/src/main.rs`：5（已有部分收敛）
8. `user/src/syscall.rs`：4
9. `os/src/drivers/block/mod.rs`：4
10. `os/src/boot.rs`：4
11. `os/src/trap/mod.rs`：3
12. `os/src/fs/mod.rs`：3
13. `arch/src/lib.rs`：3（属于 arch 内部正常分发层）
14. `os/src/syscall/process.rs`：2
15. `os/src/syscall/mod.rs`：2
16. `os/src/drivers/bus/mod.rs`：2
17. `os/src/task/switch.rs`：1

### 3）解释

这个分布非常符合多架构内核演进的常见形态：

- `mm` / `task` / `trap` 是结构性差异最容易堆积 `cfg` 的地方；
- `drivers` 与 `boot` 是平台设备模型与启动路径差异带来的天然分叉点；
- `arch/src/lib.rs` 的 `cfg_attr(path=...)` 是健康分发模式，不应作为“必须清零”的对象。

因此“目标不是把 `target_arch` 变成 0”，而是把它**从 OS 策略层挤压到 arch 适配层**，并把 OS 的 `cfg` 控制在“可读、可审计、可测试”的最小范围。

---

## 三、按模块难度评估

为方便后续排期，把存量改造分成 A/B/C 三档。

### A 档（低风险、可快速推进）

#### A1. 模块路径选择类 cfg
- 典型文件：`os/src/main.rs`, `os/src/drivers/bus/mod.rs`, `os/src/drivers/block/mod.rs`
- 特征：主要是“同名模块，路径不同”或“实现体不同但接口已稳定”。
- 策略：统一使用 `cfg_attr(path=...)` 或把差异下沉到 arch re-export。
- 风险：低，主要是编译层面的可见性与路径正确性。

#### A2. 用户态 syscall 封装寄存器差异
- 典型文件：`user/src/syscall.rs`
- 特征：参数寄存器与 `ecall/syscall` 指令差异。
- 策略：可继续维持小范围 `cfg`，或后续抽成 arch user shim。
- 风险：低到中，需警惕 ABI 回归。

### B 档（中风险，需设计抽象边界）

#### B1. trap 处理流（当前重点）
- 典型文件：`os/src/trap/mod.rs`
- 特征：主循环结构高度相似，但故障日志粒度、断点前进策略、外部中断处理行为不同。
- 策略：
  - 抽公共 `user_trap_loop` 主循环；
  - 用少量 arch-specific helper 承载差异（page fault dump、illegal log、breakpoint step、external irq 处理策略）；
  - 避免把 `trap_type` 的业务分发逻辑复制两份。
- 风险：中。属于高频路径，任何语义漂移都可能引入信号行为变化。

#### B2. task/process 运行时行为差异
- 典型文件：`os/src/task/mod.rs`, `process.rs`, `processor.rs`, `action.rs`
- 特征：线程上下文、调度点、TLS/信号上下文在两架构上的差异。
- 策略：优先抽“行为接口”而非“宏替换”，例如：
  - `arch::task_hooks::*` 或 `arch::signal_hooks::*` 的回调集合；
  - OS 侧只调接口，减少分散 `cfg`。
- 风险：中到高，涉及进程生命周期与信号一致性。

### C 档（高风险，建议后置）

#### C1. memory_set 深层差异
- 典型文件：`os/src/mm/memory_set.rs`（22 处，当前最大）
- 特征：ELF 装载、映射权限、地址布局（高半区/低半区）与 page table 行为有系统性差异。
- 策略：分层拆分，不建议“一把梭”统一：
  1. 先抽 Map 操作回调；
  2. 再抽地址空间布局策略；
  3. 最后处理 debug/trace 差异。
- 风险：高。错误会直接表现为 page fault/illegal instruction，回归代价大。

#### C2. boot 启动路径
- 典型文件：`os/src/boot.rs`
- 特征：启动协议、入口寄存器、页表切换顺序本质不同。
- 策略：保持必要 `cfg`，不追求强行统一。
- 风险：高，但收益有限。

---

## 四、Trap 统一改造方案（本次尝试方向）

### 目标

把 `os/src/trap/mod.rs` 当前两份 `user_trap_loop`（RV/LA）收敛为：

- 1 份公共主循环（syscall/time/signal 的公共框架）；
- N 个架构差异 helper（由 `cfg` 限制在小函数范围，而不是整段主流程复制）。

### 拟抽取的差异点

1. `on_page_fault(addr)`
   - LA：保留详细 pid/tid/name/pte bytes dump。
   - RV：保留简洁日志。
2. `on_illegal_instruction(addr)`
   - LA：输出 syscall/args/寄存器上下文。
   - RV：保持现有简洁路径。
3. `on_breakpoint()`
   - RV：按指令长度推进 `sepc`。
   - LA：保持现有语义（不做 RV 那套长度探测）。
4. `on_supervisor_external()`
   - RV：走 `board::irq_handler()`。
   - LA：维持当前行为（通常不会走该分支，必要时统一为 board handler 也可）。

### 设计原则

- 先“结构统一”，后“细节统一”；
- 严格保持现有行为与日志语义，避免“统一导致行为改变”；
- 所有 helper 保持短小，便于单点审查。

---

## 五、后续路线（建议）

### Phase 1（本次）
- 完成 trap 主循环统一尝试；
- 保证编译通过并跑 `run-la.sh` 基线；
- 检查关键错误日志关键字无新增异常。

### Phase 2（下一轮）
- 对 `task/*` 做接口化收敛，优先处理最外围 `cfg`；
- 保持运行语义不变，先少量高价值点。

### Phase 3（后续）
- 进入 `memory_set.rs` 的分层抽象重构；
- 以“映射操作回调”→“布局策略抽象”为顺序推进。

---

## 六、结论

当前第一方核心代码中 `target_arch` 存量仍有 95 处，集中在 `mm/task/trap/drivers` 四类模块。短期最值得做且风险可控的目标是 `trap` 主循环收敛；中期价值最大但风险最高的是 `memory_set`。

从架构解耦视角，最关键不是“绝对清零 `cfg`”，而是把 `cfg` **压缩到边界层**：

- arch crate 负责硬件/ABI 差异；
- os crate 负责策略与流程；
- 差异通过稳定接口暴露，而不是在 OS 业务路径中散落分支。

本次建议先完成 trap 统一（结构收敛），再以同样方法向 task 与 mm 推进。