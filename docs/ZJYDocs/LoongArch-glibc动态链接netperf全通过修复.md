# LoongArch glibc 动态链接 netperf 全通过修复

日期: 2026/3/25

## 背景

在 LoongArch64 网络模块适配完成后（smoltcp loopback 共享方案），musl 链接的 iperf/netperf 测试已全部通过，但 glibc 链接的 netperf 测试 0/5 全部失败。glibc 链接的 iperf 反而 6/6 全通过——区别在于 iperf3 是静态链接的，而 netperf/netserver 是动态链接的 PIE 可执行文件，依赖 glibc 的 `ld-linux-loongarch-lp64d.so.1` 动态链接器。

本文记录从 0/5 到 5/5 的调试过程，涉及三个层层递进的 bug，每个都需要前一个修复后才能暴露。

## 罪魁祸首

三个 bug 共同导致 glibc netperf 无法运行:

1. **glibc 共享库搜索路径缺失** — ld.so 找不到 `libc.so.6` 和 `libm.so.6`
2. **匿名 mmap 页未清零** — BSS 段含有脏数据，glibc nptl 锁初始值非零
3. **`unmap_range` 幽灵 PTE** — MAP_FIXED 部分重叠时遗留旧 PTE，COW 路径复制旧文件数据覆盖 BSS

---

## 第一层: glibc 共享库搜索路径缺失

### 现象

```
./netserver: no version information available (required by ./netserver)
./netserver: symbol lookup error: ./netserver: undefined symbol: optind, version GLIBC_2.36
```

5 个 netperf 子测试全部报 `symbol lookup error` 后退出。

### 分析

用 `llvm-readelf` 检查 sdcard 上的二进制:

- `glibc/netperf`: PIE, interpreter `/lib64/ld-linux-loongarch-lp64d.so.1`, NEEDED `libc.so.6` + `libm.so.6`
- `glibc/lib/libc.so.6`: 确实导出了 `optind@@GLIBC_2.36`（VERDEF index 2）
- `glibc/lib/ld-linux-loongarch-lp64d.so.1`: glibc 动态链接器

问题出在 `ensure_busybox_links()` 函数:内核启动时只为 ld.so 解释器创建了 `/lib64/ld-linux-loongarch-lp64d.so.1 → /glibc/lib/ld-linux-loongarch-lp64d.so.1` 硬链接，但**没有为 `libc.so.6` 和 `libm.so.6` 创建到 `/lib64/` 的硬链接**。

glibc ld.so 的标准库搜索路径是 `/lib64/`、`/lib/`、`/usr/lib64/`、`/usr/lib/`。sdcard 上的 glibc 库在 `/glibc/lib/` 下，不在搜索路径中。netperf 也没有 `DT_RPATH` 或 `DT_RUNPATH`。

### 修复

在 `os/src/fs/mod.rs` 的 `ensure_busybox_links()` 中添加:

```rust
#[cfg(target_arch = "loongarch64")]
if open_file("/glibc/lib/ld-linux-loongarch-lp64d.so.1", OpenFlags::empty()).is_some() {
    ensure_hardlink("/lib64/ld-linux-loongarch-lp64d.so.1", "/glibc/lib/ld-linux-loongarch-lp64d.so.1");
    // 新增: glibc ld.so 在 /lib64/ 搜索共享库
    if open_file("/glibc/lib/libc.so.6", OpenFlags::empty()).is_some() {
        ensure_hardlink("/lib64/libc.so.6", "/glibc/lib/libc.so.6");
    }
    if open_file("/glibc/lib/libm.so.6", OpenFlags::empty()).is_some() {
        ensure_hardlink("/lib64/libm.so.6", "/glibc/lib/libm.so.6");
    }
}
```

修复后 `symbol lookup error` 消失，ld.so 成功加载 libc.so.6 和 libm.so.6。

---

## 第二层: 匿名 mmap 页未清零

### 现象

修复库路径后，netperf 不再报错，但也不输出任何测试结果——进程启动后立即挂死。用 `LOG=TRACE` 追踪发现两个进程（netserver pid=3, netperf pid=4）都阻塞在 `FUTEX_WAIT`:

```
[sys_futex] pid=3 FUTEX_WAIT uaddr1=0x600272400 val=2
[sys_futex] pid=4 FUTEX_WAIT uaddr1=0x600272400 val=2
```

`val=2` 在 glibc nptl 中表示锁处于 contended 状态（`__lll_lock_wait`）。但这是进程初始化阶段，不应该有任何锁竞争。

### 分析

地址 `0x600272400` 位于 libc.so.6 的 `.bss` 段（NOBITS, 应该全零初始化）。在 `sys_mmap` 中添加诊断日志:

```
[mmap-debug] pid=3 anon mmap at 0x600272000+0x400 => val=197840
```

**匿名 mmap 返回的页面包含脏数据 `197840`（0x30530）而非零!**

根因: `MapArea::map_one()` 调用 `frame_alloc()` 获取物理页后没有清零。rcore 的帧分配器回收旧页时不清零，再次分配时包含上一个使用者的残留数据。

在 Linux 中，匿名 mmap（`MAP_ANONYMOUS`）保证返回零页。这是 POSIX 强制要求，glibc 依赖这个保证来确保 BSS 段中的全局变量（包括 nptl 互斥锁）初始值为零。

### 修复

在 `os/src/mm/memory_set.rs` 的 `map_one()` 中:

```rust
let frame = frame_alloc().unwrap();
let ppn: PhysPageNum = frame.ppn;
ppn.get_bytes_array().fill(0);  // 新增: 清零物理页
self.data_frames.insert(vpn, frame);
```

但修复后问题依旧——`val` 仍然是 `197840`。这说明清零代码没有生效，或者生效后又被覆盖了。

---

## 第三层: unmap_range 幽灵 PTE（最隐蔽的 bug）

### 现象

即使在 `map_one` 中添加了 `fill(0)`，诊断日志仍然显示 `val=197840`。这说明 `map_one` 走的不是"分配新帧并清零"的正常路径，而是走了 COW 路径（复制旧页数据到新帧）。

### 深入分析: glibc ld.so 的 mmap 序列

glibc ld.so 加载 libc.so.6 时的 mmap 序列:

| 步骤 | 操作 | VPN 范围 | 说明 |
|------|------|---------|------|
| 1 | `mmap(NULL, 0x1edd78, PROT_NONE, ANON)` | [0x600094, 0x600282) | 保留地址空间 |
| 2 | `mmap(0x600094000, R+X, MAP_FIXED, fd)` | [0x600094, 0x60027E) | 映射 LOAD1 (代码段) |
| 3 | `mmap(0x60020D000, RW, MAP_FIXED, fd)` | [0x60020D, 0x600272) | 映射 LOAD2 (数据段) |
| 4 | `mmap(0x600272000, RW, MAP_FIXED, ANON)` | [0x600272, 0x60027E) | 映射 BSS (应全零) |

关键问题出在步骤 3 和 4 的交互:

**步骤 3** 的 `MAP_FIXED` 触发 `unmap_range([0x60020D, 0x600272))`，与步骤 2 的 area A2=[0x600094, 0x60027E) **部分重叠**。旧的 `unmap_range` 实现:

```rust
// 旧实现: 只清理重叠部分的 PTE，然后删除整个 area
let overlap = [0x60020D, 0x600272);
for vpn in overlap { unmap(vpn); }
let area = self.areas.remove(i);
core::mem::forget(area.data_frames);  // 泄漏非重叠部分的帧
```

这导致 **VPN [0x600272, 0x60027E) 的 PTE 仍然存在**（指向步骤 2 映射的文件数据页），但已没有任何 `MapArea` 跟踪它们。它们成了"幽灵 PTE"。

**步骤 4** 的 `MAP_FIXED` 触发 `unmap_range([0x600272, 0x60027E))`，但此时 `self.areas` 中已经没有覆盖这个范围的 area（步骤 3 把 A2 整个删了）。`unmap_range` 找不到重叠，**什么也不做**。

接着 `insert_framed_area` 调用 `map_one(0x600272)`。`map_one` 通过 `page_table.translate(vpn)` 发现 PTE 仍然有效（幽灵 PTE），于是走 COW 路径:

```rust
if let Some(pte) = page_table.translate(vpn) {
    if pte.is_valid() {
        // COW: 复制旧页数据到新帧（包含文件数据 0x30530）
        new_ppn.get_bytes_array().copy_from_slice(old_ppn.get_bytes_array());
        return;  // 跳过清零路径!
    }
}
// 正常路径（被跳过了）
ppn.get_bytes_array().fill(0);
```

这就解释了为什么清零代码不生效——COW 路径在清零之前就 return 了，而且复制了旧文件数据。

### 修复

重写 `unmap_range` 的部分重叠处理。不再删除整个 area + `mem::forget`，而是:

1. 清理重叠 VPN 的 PTE（保持不变）
2. **清理被遗弃的尾部 VPN 的 PTE**（新增）
3. 将 area 缩减到非重叠部分（而非整个删除）

```rust
// 中间重叠: 保留 [a_start, overlap_start)，清理尾部 [overlap_end, a_end)
let mut tail_vpn = overlap_end;
while tail_vpn < a_end {
    // 无论是否在 data_frames 中，都清理 PTE
    if self.areas[i].data_frames.remove(&tail_vpn).is_some() {
        if self.page_table.translate(tail_vpn).map_or(false, |pte| pte.is_valid()) {
            self.page_table.unmap(tail_vpn);
        }
    } else if self.page_table.translate(tail_vpn).map_or(false, |pte| pte.is_valid()) {
        self.page_table.unmap(tail_vpn);
    }
    tail_vpn.step();
}
self.areas[i].vpn_range = VPNRange::new(a_start, overlap_start);
```

修复后诊断日志:

```
[mmap-debug] pid=3 anon mmap at 0x600272000+0x400 => val=0  ✓
[mmap-debug] pid=4 anon mmap at 0x600272000+0x400 => val=0  ✓
```

---

## 修复结果

### 测试成绩

| 测试套件 | 修复前 | 修复后 | 评分 |
|----------|--------|--------|------|
| iperf-musl | 6/6 | 6/6 | 6.70 |
| iperf-glibc | 6/6 | 6/6 | 6.77 |
| netperf-musl | 4/5 (TCP_CRR fail) | **5/5** | 7.10 |
| netperf-glibc | **0/5** | **5/5** | 7.25 |
| **合计** | **16/22** | **22/22** | **27.82** |

### 为什么 musl netperf TCP_CRR 也修好了?

之前 musl netperf TCP_CRR 超时是因为 `unmap_range` 幽灵 PTE 导致 musl libc 的某些初始化路径也出现了问题。修复 `unmap_range` 后，musl 链接的程序也受益。

## 经验总结

1. **glibc 对 mmap 语义的要求远比 musl 严格**。musl 的 malloc/lock 实现更简单，对 BSS 零初始化不那么敏感；glibc nptl 的 `__lll_lock_wait` 精确依赖锁变量的初始值。

2. **MAP_FIXED 的 unmap + remap 必须是原子语义**。旧的 `unmap_range` 在部分重叠时删除整个 area、`mem::forget` 帧，导致 PTE 泄漏。这在简单测试中不会出问题（因为进程退出时整个页表会被回收），但在 glibc ld.so 的复杂 mmap 序列中暴露了。

3. **调试链路: 错误→symbol lookup → 挂死→futex → 脏数据→mmap → COW→幽灵 PTE**。三个 bug 层层嵌套，每修一个才能暴露下一个。这类"洋葱式"调试需要耐心和系统性的排查。
