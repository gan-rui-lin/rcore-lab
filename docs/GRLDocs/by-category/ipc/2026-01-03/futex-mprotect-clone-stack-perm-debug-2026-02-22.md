# futex 与 mprotect/clone 导致用户栈权限丢失的调试记录

日期：2026/02/22

## 背景与现象

本次调试起因是 `libctest-musl` 中的 `entry-static.exe` 在运行时出现大量错误，最初日志显示 `futex` 未实现（syscall 98）。补齐 `futex` 后，`unimplemented syscall 98 (futex)` 消失，但测试仍然失败，且开始反复出现用户态 `LoadPageFault`。错误现场具有稳定特征：

- `trap_handler: Exception(LoadPageFault) in application`
- `pid=35 name=entry-static.exe`
- `bad addr (stval) = 0x40022ac0`
- `sepc = 0x406d0`
- `stval pte flags = V | U`（缺失 `R/W`）
- 寄存器 `sp = 0x40022ac0`，与 `stval` 完全一致

以上信息表明：发生故障的地址就是用户栈指针位置，且该页 PTE 仅有 `V|U`，缺少 `R/W`，因此对栈的读（Load）立即触发了页故障。这一判定直接指向“用户栈页权限被异常降级”。

## 调试目标

确定用户栈页权限何时、何处从 `R|W|U` 变为仅 `V|U`，并解释为何修复出现在 `clone` 逻辑处。

## 关键日志与推断

### 1. 建栈与克隆阶段权限正常

在 `map_user_stack_and_trap` 和 `TaskUserRes::alloc_user_res` 处加入日志，观察建栈阶段的 PTE bits，结果均为 `...17`，即 `V|R|W|U` 具备写权限。示例：

- `[stack_map] bottom=0x8040a000 top=0x8040f000 pte_bits=0x208bd417`
- `[clone_area] idx=3 start=0x8040a000 end=0x8040f000 pte_bits=0x208c7417`

这一类日志表明：

- 初始建栈时权限是正确的；
- `from_existed_user` 的区域复制后，栈区域依然是 `R|W|U`；
- 权限丢失不是在建栈或 clone 的“建区”阶段发生。

### 2. mprotect 的出现与时间位置

进一步在 `change_protection` 中加入 `mprotect` 日志后，发现：

- `sys_mprotect addr=0x40002000 len=0x21000 prot=0x3`
- `[mprotect] area=0x40000000-0x40023000 modify=0x40002000-0x40023000 perm_bits=0x16 pte_bits=0x20a83417`
- 随后立刻出现 `LoadPageFault`，栈页 PTE 变成 `V|U`

关键点是 `mprotect` 调整的是 `0x40002000-0x40023000`，该范围覆盖到 `stval=0x40022ac0`。也就是说，**栈地址确实被 mprotect 涉及**。但在 mprotect 当时，PTE 仍然显示 `...17`（R/W/U），而在后续 `LoadPageFault` 时变成了 `V|U`。

这个“先正确、后丢失”的模式说明：

- mprotect 操作本身并没有直接清掉 R/W；
- 权限丢失发生在 mprotect 之后的某个重建/复制路径上。

### 3. 罪魁祸首定位：clone 使用 map_perm 覆盖了子区间权限

`change_protection` 的实现逻辑是：

- 逐页更新 PTE flags（`change_pte_flags`）；
- **只有当修改区间完全覆盖 MapArea 才会更新 `area.map_perm`**。

这意味着：如果 mprotect 只改了 MapArea 的一部分，`map_perm` 不会变化。

而 `from_existed_user` 的 clone 逻辑采用：

- `MapArea::from_another(area)` 复制 `map_perm`；
- `memory_set.push(new_area, None)` 重新映射整块区域；

因此在 clone 时，**如果 `map_perm` 没更新，clone 会用旧权限重新建映射，覆盖掉 mprotect 的精细修改**。这与现场表现一致：mprotect 修改了栈的权限，但随后 clone 重新建立映射，导致该页权限被回退为 `V|U`。最终触发栈读 `LoadPageFault`。

这就是本次调试的“罪魁祸首”。

## 修复策略与解释

### 方案 A：结构化修复（理想方案）

- 在 `change_protection` 中拆分 MapArea 或维护细粒度权限表；
- 确保 `map_perm` 能反映子区间修改；
- clone 时继续使用 `map_perm` 即可。

优点：设计完备、语义一致。缺点：改动范围大，容易引入新问题。

### 方案 B：保真复制（当前采用）

在 clone 复制页内容时，**直接把源 PTE 的 flags 原样写回新页**，从而保留 mprotect 的细粒度修改。实现方式如下：

- 遍历 `area.vpn_range` 时，取 `src_pte = user_space.translate(vpn)`；
- 在 `memory_set` 的对应页上 `change_pte_flags(vpn, src_pte.flags())`。

这个做法看起来像“补丁”，但本质是“状态保真复制”，可以保证 clone 后页级权限不被粗粒度 map_perm 覆盖。对现阶段来说，这是风险较小、改动最小且能恢复正确行为的方案。

### 为什么这个方案是合理的“补丁”

- mprotect 已经改变了“真实的页权限”；
- clone 的责任是复制“真实状态”；
- 直接复制 PTE flags 是最直接的“真实状态复制”。

所以它不是盲目 patch，而是**保证状态一致性**的最小修复。

## 当前状态与后续建议

- futex 未实现的问题已消失；
- 栈页权限问题已定位并给出修复；
- 下一步需要关注“卡住”的原因（可能是 futex 阻塞或信号等待）；
- 建议保留 `mprotect` 与 stack/clone 日志，直到卡住原因明确。

## 附：关键路径与文件

- 触发点：`LoadPageFault`，`stval=sp`，PTE 仅 `V|U`。
- mprotect 入口：`sys_mprotect` → `MemorySet::change_protection`。
- clone 路径：`MemorySet::from_existed_user`。
- 用户栈映射：`map_user_stack_and_trap` 与 `TaskUserRes::alloc_user_res`。

## 小结

调试核心结论：

1. 栈页权限丢失不是建栈/clone 初始阶段发生，而是在 mprotect 之后被 clone 覆盖。
2. 根因是 `change_protection` 未同步更新 `map_perm`，导致 clone 使用旧 `map_perm` 重建映射，回退权限。
3. 采用逐页复制 PTE flags 的方式保证权限保真，是当前最稳妥的修复策略。

以上结论已通过多轮日志交叉验证，后续可继续排查“卡住”问题。
