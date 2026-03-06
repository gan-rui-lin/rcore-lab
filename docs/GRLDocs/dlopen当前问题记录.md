# dlopen 当前问题记录

日期：2026/3/6

## 现象

在最新的全量测试日志中，entry-dynamic 的 dlopen 用例失败，但其他 entry-dynamic 用例大多通过。失败信息集中在：

- dlsym i failed: Symbol not found: i
- FAIL dlopen [status 245]

从 all207.log 可以看到该失败发生在 "========== START entry-dynamic.exe dlopen ==========" 之后，随即输出上述错误并结束该用例。与之前 all206.log 中的 LoadPageFault 不同，这次没有出现内核崩溃或页错误，属于纯用户态测试失败。

## 关键日志摘录（概述）

- dlopen_dso.so 被 openat/fstat 成功打开。
- dlsym("i") 返回空，dlerror 提示 "Symbol not found: i"。
- 用例判定失败后结束。

## 影响范围

- 仅 entry-dynamic 的 dlopen 用例失败；同一轮中其他动态链接用例均通过。
- entry-static 对应用例没有类似问题（静态链接不走 dlopen 逻辑）。

## 初步判断

这不是内核立即崩溃的问题，而是动态加载器在符号解析阶段没有从 dlopen_dso.so 的动态符号表中找到符号 i。结合内核侧实现，可能原因主要集中在：

1. **文件映射 mmap 行为不完整**：dlopen 依赖 loader 通过 mmap 把 DSO 的 PT_LOAD 映射到内存，如果对带 fd 的 mmap 或 offset 处理有缺陷，会导致 .dynsym/.dynstr 内容不可用，进而 dlsym 找不到符号。
2. **mprotect 或 MAP_FIXED 处理缺陷**：若加载器在重定位时通过 mprotect 修改权限失败，或者 MAP_FIXED 地址冲突导致加载到错误位置，也会造成符号表不可读。
3. **DSO 文件本身异常**：dlopen_dso.so 被裁剪、strip 或未导出符号 i（可能性较小，通常测试件是正确的）。

## 当前待办方向

- 在 sys_mmap 路径增加对 fd != -1 的 mmap 调用日志，记录 fd、offset、len、prot、flags 与返回地址。
- 核对 dlopen_dso.so 的导出符号（例如用 readelf -Ws 或 objdump -T），确认符号 i 是否在动态符号表。
- 对照 musl loader 的映射行为，确认 MAP_PRIVATE|MAP_FIXED 的分支在内核实现是否完整。

## 备注

该问题尚未修复，仅作为当前状态记录。后续修复和验证应单独记录到对应调试文档中。
