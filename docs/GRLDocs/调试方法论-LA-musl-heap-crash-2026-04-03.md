# 调试方法论：LoongArch musl 堆崩溃问题

**日期**: 2026/04/03

本文档记录了调试 LoongArch musl LTP 测试崩溃问题的完整思路和方法论，供后续调试参考。

---

## 一、问题现象

```
=== Running /tmp/ltp_testcode_filtered.sh ===
#### OS COMP TEST GROUP START ltp-musl ####
=== /tmp/ltp_testcode_filtered.sh completed (status=0x8b) ===
```

- `status=0x8b` = 139 = 128 + 11 = **SIGSEGV**
- 测试直接崩溃，没有任何测试用例输出
- basic 测试和 glibc 测试正常

---

## 二、调试思路流程

### 第一步：获取错误上下文

**目标**：找到崩溃时的具体信息（地址、PC、寄存器等）

**方法**：使用 `LOG=INFO` 或 `LOG=ERROR` 运行，获取内核的 trap handler 日志

```bash
LOG=INFO timeout 60 bash run-la.sh -f sdcard-la.img -t all 2>&1 | strings | grep -E "trap_handler|page fault|signum"
```

**结果**：
```
[ERROR] trap_handler: page fault addr=0x120200000 pid=2 name=busybox sepc=0x12018b688
[ERROR] trap_handler: fault pte ppn=0x98479 flags=V | R | X | U
```

**分析要点**：
1. `addr=0x120200000` - 故障地址
2. `flags=V | R | X | U` - PTE 有效、可读、可执行、用户态，但**没有 W（写权限）**
3. 这是一个**写入只读页面**的错误

### 第二步：定位故障地址属于哪个区域

**目标**：确定故障地址是代码段、数据段还是堆/栈

**方法**：查看 ELF 加载日志

```bash
strings /tmp/la-trace.log | grep -E "\[ELF\].*PH_LOAD"
```

**结果**：
```
[ELF] PH_LOAD: vaddr=0x120000000 ... end=0x1201f7888  (代码段)
[ELF] PH_LOAD: vaddr=0x1201fbe48 ... end=0x1201fdb98  (数据段)
```

**分析**：
- 故障地址 `0x120200000` **超出了所有 ELF 段范围**
- 结论：故障地址在**堆区域**

### 第三步：追踪系统调用序列

**目标**：理解堆是如何被破坏的

**方法**：添加 TRACE 日志追踪 `sys_sbrk` 和 `sys_mmap`

```bash
LOG=TRACE timeout 45 bash run-la.sh -f sdcard-la.img -t all > /tmp/la-trace.log 2>&1
strings /tmp/la-trace.log | grep -E "sys_sbrk|sys_mmap.*0x1201f"
```

**结果**：
```
[TRACE] [sys_sbrk] pid=2 ... new=0x120200000 ok
[TRACE] [sys_mmap] pid=2 req=0x1201fe000 len=0x1000 prot=0x0 flags=0x32 -> overlap=1 fixed=true
```

**关键发现**：
1. `sys_sbrk` 扩展堆到 `0x120200000`
2. 紧接着 `sys_mmap` 在**堆起始地址** `0x1201fe000` 调用
3. `prot=0x0` = **PROT_NONE**（没有任何权限）
4. `flags=0x32` = MAP_PRIVATE | MAP_ANONYMOUS | **MAP_FIXED**
5. `overlap=1` 表示覆盖了已有映射

### 第四步：架构对比分析

**目标**：确认问题是否架构特定

**方法**：在 RISC-V 上运行相同的追踪

```bash
LOG=TRACE timeout 45 bash run.sh -f sdcard-rv.img -t all > /tmp/rv-trace.log 2>&1
strings /tmp/rv-trace.log | grep -E "sys_mmap.*prot=0x0|heap_bottom"
```

**结果**：
```
[DEBUG] exec: heap_bottom=0x166000 ...
[TRACE] [sys_sbrk] pid=2 ... heap_bottom=0x166000 new=0x167000 ok
# 没有 prot=0x0 的 mmap 调用！
```

**结论**：
| 特征 | RISC-V | LoongArch |
|------|--------|-----------|
| heap_bottom | 0x166000 | 0x1201fe000 |
| mmap(PROT_NONE) 在堆区域 | ❌ | ✅ |

**问题是 LoongArch 特有的**，由 musl malloc 的不同行为导致。

### 第五步：深入分析代码路径

**目标**：理解为什么 mmap 会破坏堆

**分析 `sys_mmap` 处理流程**：
```rust
if is_fixed {
    // MAP_FIXED: unmap overlapping pages in the target range
    inner.memory_set.unmap_range(VirtAddr(start), VirtAddr(start + len));
}
// 创建新映射
inner.memory_set.insert_mmap_area(..., permission, ...);
```

**分析 `sys_sbrk` 查找堆的逻辑**：
```rust
// 通过 heap_bottom 地址查找堆区域
self.areas.iter_mut().find(|area| area.vpn_range.get_start() == start.floor())
```

**问题链条**：
1. `sys_sbrk` 扩展堆，创建区域 `[0x1201fe000, 0x120200000)` 权限 `R|W|U`
2. `sys_mmap(PROT_NONE, MAP_FIXED)` 在 `0x1201fe000` 覆盖堆
3. `unmap_range` 缩小原堆区域
4. `insert_mmap_area` 创建新区域 `[0x1201fe000, 0x1201ff000)` 权限只有 `U`
5. 下次 `sys_sbrk` 查找 `heap_bottom=0x1201fe000` 的区域
6. 找到的是 mmap 创建的只有 `U` 权限的区域
7. `append_to` 扩展这个错误的区域
8. 新页面没有写权限 → **SIGSEGV**

### 第六步：设计修复方案

**思路**：不依赖地址查找，而是通过权限验证

```rust
pub fn append_to_heap(&mut self, heap_bottom: VirtAddr, new_end: VirtAddr) -> bool {
    let expected_perm = MapPermission::R | MapPermission::W | MapPermission::U;
    
    // 查找权限正确的堆区域
    if let Some(area) = self.areas.iter_mut().find(|area| {
        area.vpn_range.get_start() == heap_bottom.floor() && 
        area.map_perm == expected_perm  // 权限验证！
    }) {
        area.append_to(...);
        return true;
    }
    
    // 如果找不到，创建新的堆区域
    let map_area = MapArea::new(heap_bottom, new_end, MapType::Framed, expected_perm);
    self.push(map_area, None);
    true
}
```

**架构特定修复**：
```rust
#[cfg(target_arch = "loongarch64")]
{ inner.memory_set.append_to_heap(...) }

#[cfg(not(target_arch = "loongarch64"))]
{ inner.memory_set.append_to(...) }  // RISC-V 保持原逻辑
```

---

## 三、调试工具箱

### 3.1 日志级别选择

| 级别 | 用途 | 日志量 |
|------|------|--------|
| `LOG=ERROR` | 只看错误 | 最少 |
| `LOG=WARN` | 错误+警告 | 少 |
| `LOG=INFO` | 关键信息 | 中等 |
| `LOG=TRACE` | 完整追踪 | 巨大 |

**建议**：先用 `ERROR/INFO` 定位范围，再用 `TRACE` 追踪细节

### 3.2 关键日志搜索模式

```bash
# 查找页面错误
grep -E "page fault|trap_handler|signum"

# 查找系统调用序列
grep -E "sys_sbrk|sys_mmap|sys_brk"

# 查找 ELF 加载信息
grep -E "\[ELF\]"

# 查找权限相关
grep -E "prot=|perm=|flags="
```

### 3.3 二进制日志处理

QEMU 输出可能包含 ANSI 转义序列，使用 `strings` 过滤：

```bash
timeout 60 bash run-la.sh ... > /tmp/log 2>&1
strings /tmp/log | grep "pattern"
```

### 3.4 架构对比命令模板

```bash
# LoongArch
LOG=TRACE timeout 60 bash run-la.sh -f sdcard-la.img -t all > /tmp/la.log 2>&1

# RISC-V
LOG=TRACE timeout 60 bash run.sh -f sdcard-rv.img -t all > /tmp/rv.log 2>&1

# 对比
diff <(strings /tmp/la.log | grep "pattern") <(strings /tmp/rv.log | grep "pattern")
```

---

## 四、经验总结

### 4.1 调试原则

1. **先定位范围，再深入细节**
   - 先用 `LOG=ERROR` 看崩溃点
   - 再用 `LOG=TRACE` 追踪调用链

2. **架构对比是利器**
   - 如果一个架构正常、另一个崩溃，对比差异能快速定位

3. **追踪系统调用序列**
   - 内存问题往往是多个系统调用交互的结果
   - 单独看一个调用看不出问题

4. **不要假设地址稳定**
   - 堆地址可能被 mmap 覆盖
   - 使用类型或权限验证更可靠

### 4.2 常见陷阱

1. **日志太多看不完**
   - 用 `grep` 过滤关键模式
   - 先看 ERROR，再扩大范围

2. **二进制字符干扰**
   - 用 `strings` 过滤
   - 或重定向 stderr

3. **超时误判**
   - 设置合理的 timeout（60-120s）
   - 区分"卡死"和"慢"

### 4.3 调试检查清单

- [ ] 确认问题是否可复现
- [ ] 获取 trap handler 的错误日志
- [ ] 解析故障地址属于哪个区域
- [ ] 追踪相关系统调用序列
- [ ] 对比其他架构的行为
- [ ] 分析代码路径找根因
- [ ] 设计最小侵入性修复
- [ ] 验证修复对所有架构有效

---

## 五、参考资料

- `os/src/trap/user_trap_loongarch64.rs` - 页面错误处理
- `os/src/mm/memory_set.rs` - 内存管理
- `os/src/syscall/process.rs` - sbrk/mmap 实现
- chronix 内核的 `UserVmAreaType::Heap` 设计 - 更健壮的堆标识方案
