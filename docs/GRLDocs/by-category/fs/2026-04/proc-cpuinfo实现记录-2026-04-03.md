# /proc/cpuinfo 实现记录

**日期**: 2026/04/03

---

## 1. 背景

在运行 LTP 测试时，多个测试用例因缺少 `/proc/cpuinfo` 文件而失败：

```
cpufreq_boost    1  TBROK  :  tst_virt.c:37: fopen(/proc/cpuinfo,r) failed: errno=ENOENT(2)
```

### 1.1 为什么需要 /proc/cpuinfo

LTP 测试框架的 `tst_virt.c` 在测试启动时会读取 `/proc/cpuinfo` 来检测运行环境是否为虚拟机（QEMU/KVM/Xen 等）。这个检测逻辑位于 `is_kvm()` 函数中：

```c
// testsuits-for-oskernel/ltp-full-20240524/lib/tst_virt.c
static int is_kvm(void)
{
    FILE *cpuinfo;
    char line[64];
    int found;

    cpuinfo = SAFE_FOPEN(NULL, "/proc/cpuinfo", "r");
    found = 0;
    while (fgets(line, sizeof(line), cpuinfo) != NULL) {
        if (strstr(line, "QEMU Virtual CPU")) {
            found = 1;
            break;
        }
    }
    SAFE_FCLOSE(NULL, cpuinfo);
    return found;
}
```

如果 `/proc/cpuinfo` 不存在，`SAFE_FOPEN` 会导致测试以 TBROK（Test Broken）状态退出。

---

## 2. 实现方案

### 2.1 双架构支持

rcore-lab 支持 RISC-V 和 LoongArch 双架构，cpuinfo 需要根据架构输出不同格式。

### 2.2 RISC-V cpuinfo 格式

```
processor	: 0
hart		: 0
isa		: rv64imafdc
mmu		: sv39
uarch		: qemu,virt
```

字段说明：
- `processor`: CPU 编号（从 0 开始）
- `hart`: RISC-V 硬件线程 ID
- `isa`: 支持的 ISA 扩展
- `mmu`: 内存管理单元类型
- `uarch`: 微架构信息

### 2.3 LoongArch cpuinfo 格式

```
system type	: generic-loongson-machine
processor	: 0
package		: 0
core		: 0
cpu family	: Loongson-64bit
model name	: Loongson-3A5000-QEMU
CPU MHz		: 2000.00
BogoMIPS	: 4000.00
tlb_entries	: 2112
address sizes	: 48 bits physical, 48 bits virtual
isa		: loongarch64
features	: cpucfg lam ual fpu
```

### 2.4 代码实现

修改文件: `os/src/fs/vfs/procfs.rs`

```rust
/// Generate /proc/cpuinfo content (architecture-specific)
fn proc_cpuinfo() -> String {
    #[cfg(target_arch = "riscv64")]
    {
        // RISC-V cpuinfo format (single core)
        String::from(
            "processor\t: 0\n\
             hart\t\t: 0\n\
             isa\t\t: rv64imafdc\n\
             mmu\t\t: sv39\n\
             uarch\t\t: qemu,virt\n\
             \n",
        )
    }
    #[cfg(target_arch = "loongarch64")]
    {
        // LoongArch cpuinfo format (single core)
        String::from(
            "system type\t: generic-loongson-machine\n\
             processor\t: 0\n\
             package\t\t: 0\n\
             core\t\t: 0\n\
             cpu family\t: Loongson-64bit\n\
             model name\t: Loongson-3A5000-QEMU\n\
             CPU MHz\t\t: 2000.00\n\
             BogoMIPS\t: 4000.00\n\
             tlb_entries\t: 2112\n\
             address sizes\t: 48 bits physical, 48 bits virtual\n\
             isa\t\t: loongarch64\n\
             features\t: cpucfg lam ual fpu\n\
             \n",
        )
    }
}
```

---

## 3. 验证测试

### 3.1 RISC-V 测试

```bash
SINGLE_TEST=tmp-cpuinfo LOG=INFO timeout 30 bash run.sh -f sdcard-rv.img -t rv
```

输出：
```
=== Testing /proc/cpuinfo ===
processor	: 0
hart		: 0
isa		: rv64imafdc
mmu		: sv39
uarch		: qemu,virt
=== cpuinfo test done ===
```

### 3.2 LoongArch 编译验证

```bash
make la  # 编译成功
```

---

## 4. 对 LTP 测试的影响

### 4.1 已修复的问题

- `cpufreq_boost`: 不再因 `/proc/cpuinfo` 缺失而 TBROK
- `clone03` 等依赖虚拟机检测的测试

### 4.2 仍存在的问题

| 测试 | 状态 | 原因 |
|------|------|------|
| cpuset01 | TCONF (32) | 需要 NUMA 和 libnuma 支持 |
| cpuset_cpu_hog | FAIL (127) | 可执行文件未找到 |
| cpuctl_* 系列 | TBROK | 需要 cgroup CPU controller 支持 |
| cpufreq_boost | 可能仍失败 | 需要 `/sys/devices/system/cpu/cpufreq/` 结构 |

---

## 5. 参考资料

- Linux RISC-V cpuinfo: `arch/riscv/kernel/cpu.c`
- Linux LoongArch cpuinfo: `arch/loongarch/kernel/proc.c`
- 参考实现: `/home/grl/codeRepo/OSKernel2025-rustoswhu/vfs/src/procfs/cpuinfo.rs`

---

## 6. 相关文件

- 内核实现: `os/src/fs/vfs/procfs.rs`
- 测试脚本: `user/src/bin/initcode.rs` (TMP_CPUINFO_PATH)
- LTP 源码: `testsuits-for-oskernel/ltp-full-20240524/lib/tst_virt.c`
