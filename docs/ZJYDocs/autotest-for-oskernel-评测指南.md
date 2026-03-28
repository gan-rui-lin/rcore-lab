# oscomp 自动评测系统 (autotest-for-oskernel) 研究文档

**日期**: 2026/3/20

---

## 1. 概述

`autotest-for-oskernel` 是操作系统大赛 (oscomp) 官方的自动评测系统。它通过 Docker 容器运行，负责：编译参赛内核 -> 在 QEMU 中启动内核并执行测试用例 -> 解析串口输出 -> 对比 baseline 计算得分 -> 生成 HTML 评测报告。

仓库地址: https://github.com/oscomp/autotest-for-oskernel

本文档基于对仓库源码的逐文件分析，详细解读其架构、评测流程、评分机制和输出协议。

---

## 2. 总体架构

### 2.1 目录结构

```
autotest-for-oskernel/
├── kernel/                     # 核心评测逻辑（打包为 kernel.zip）
│   ├── __main__.py             # 入口：prework -> run -> postwork
│   ├── prework.py              # 编译阶段
│   ├── run.py                  # 运行 QEMU + 解析输出
│   ├── run_qemu.py             # QEMU 启动参数
│   ├── postwork.py             # 评分 + HTML 报告生成
│   ├── parse_output_2023.py    # 各测试组输出解析器
│   ├── utils.py                # 环境变量、日志工具
│   ├── judge/                  # 每个测试组的独立评分脚本
│   │   ├── judge_basic-musl.py
│   │   ├── judge_busybox-musl.py
│   │   ├── judge_libctest-musl.py
│   │   ├── judge_lmbench-musl.py
│   │   ├── judge_libcbench-musl.py
│   │   ├── judge_lua-musl.py
│   │   ├── judge_iozone-musl.py
│   │   ├── judge_iperf-musl.py
│   │   ├── judge_netperf-musl.py
│   │   ├── judge_cyclictest-musl.py
│   │   ├── judge_ltp-musl.py
│   │   ├── (每个 -musl 都有对应的 -glibc 版本)
│   │   └── config.json         # QEMU 配置 (smp, mem, timeout)
│   ├── baselines/              # 各测试的 baseline 数据
│   │   ├── libctest_baseline.py
│   │   ├── libcbench_baseline.py
│   │   ├── iozone_baseline.py
│   │   ├── lmbench_baseline.py (在 judge 里内嵌)
│   │   └── ...
│   └── templates/              # HTML 模板
│       ├── general.html
│       ├── table.html
│       ├── comment.html
│       └── ...
└── README.md
```

### 2.2 Docker 运行方式

评测在 Docker 容器中运行，挂载路径：

| Docker 内路径 | 宿主机路径 | 说明 |
|---|---|---|
| `/coursegrader/submit` | `$os`（你的内核源码） | 编译和运行的工作目录 |
| `/coursegrader/testdata` | `$data`（测试数据） | sdcard 镜像 + judge 脚本 |
| `/cg` | `autotest-for-oskernel/` | 评测代码（kernel.zip） |
| `/mnt/cghook/` | `$data` | 日志输出和 cancel hook |

---

## 3. 评测流程详解

### 3.1 三阶段流水线

评测由 `pygrading.Job` 框架驱动，分为三个阶段：

```
__main__.py → Job(prework, run, postwork).start()
```

#### 阶段一: prework（编译）

**文件**: `kernel/prework.py`

1. **安全检查**: 禁止提交 `os.bin`, `os_serial_out.txt` 等文件
2. **执行编译**: 在 `submit_dir` 下运行 `make all`
3. **编译输出记录**: 写入 `/mnt/cghook/console_log`
4. **设置 SBI**: `config['sbi_file'] = 'default'`
5. **初始化测试用例**: 创建一个 score=100 的测试用例容器

关键点：**你的项目根目录必须有 `Makefile`，且 `make all` 必须成功**。

#### 阶段二: run（运行 QEMU）

**文件**: `kernel/run.py` + `kernel/run_qemu.py`

同时启动两个线程，分别运行 RISC-V 和 LoongArch：

```python
trv = Thread(target=run_qemu, args=(job, sbi, "kernel-rv", "sdcard-rv.img", "os_serial_out_rv.txt"))
tla = Thread(target=run_qemu_loong, args=(job, sbi, "kernel-la", "sdcard-la.img", "os_serial_out_la.txt"))
```

**RISC-V QEMU 命令**:
```bash
qemu-system-riscv64 \
  -machine virt -kernel kernel-rv \
  -m 1G -nographic -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -no-reboot \
  -device virtio-net-device,netdev=net \
  -netdev user,id=net \
  -rtc base=utc
```

**LoongArch QEMU 命令**:
```bash
qemu-system-loongarch64 \
  -kernel kernel-la \
  -m 1G -nographic -smp 1 \
  -drive file=sdcard-la.img,if=none,format=raw,id=x0 \
  -device virtio-blk-pci,drive=x0 \
  -no-reboot \
  -device virtio-net-pci,netdev=net0 \
  -netdev user,id=net0 \
  -rtc base=utc
```

关键参数（来自 `judge/config.json`）：
- `qemu.smp`: 1（单核）
- `qemu.mem`: 1G
- `qemu.timeout`: 3600 秒（1 小时）

超时后 QEMU 进程会被 `p.kill()` 杀掉。

**内核二进制文件命名约定**:
- RISC-V: `make all` 后产生 `kernel-rv`
- LoongArch: `make all` 后产生 `kernel-la`

如果项目根目录存在 `disk.img`，会作为第二块 virtio-blk 设备挂载。

#### 阶段三: postwork（评分 + 报告）

**文件**: `kernel/postwork.py`

1. 从 QEMU 输出文件中，按 test group 调用对应的 judge 脚本
2. 汇总所有 group 的分数
3. 按 `{group}-{arch}` 生成分数矩阵（4 列：glibc-rv, glibc-la, musl-rv, musl-la）
4. 渲染 HTML 表格作为评测报告

---

## 4. 输出协议（串口输出格式）

### 4.1 测试组标记（最关键！）

内核的串口输出（stdout）必须包含以下格式的标记，评测系统才能识别并评分：

```
#### OS COMP TEST GROUP START <group-name> ####
... 测试输出 ...
#### OS COMP TEST GROUP END ####
```

其中 `<group-name>` 必须匹配 judge 脚本的名称，支持的 group 有：

| group-name | 对应 judge 脚本 | 说明 |
|---|---|---|
| `basic-musl` | `judge_basic-musl.py` | musl 链接的基础系统调用测试 |
| `basic-glibc` | `judge_basic-glibc.py` | glibc 链接的基础系统调用测试 |
| `busybox-musl` | `judge_busybox-musl.py` | musl 版 busybox 命令测试 |
| `busybox-glibc` | `judge_busybox-glibc.py` | glibc 版 busybox 命令测试 |
| `libctest-musl` | `judge_libctest-musl.py` | musl libc 测试（static + dynamic） |
| `libcbench-musl` | `judge_libcbench-musl.py` | musl libc-bench 性能测试 |
| `lmbench-musl` | `judge_lmbench-musl.py` | lmbench 性能基准 |
| `lua-musl` | `judge_lua-musl.py` | Lua 脚本测试 |
| `iozone-musl` | `judge_iozone-musl.py` | IOzone 文件系统 I/O 性能 |
| `iperf-musl` | `judge_iperf-musl.py` | 网络带宽测试 |
| `netperf-musl` | `judge_netperf-musl.py` | 网络性能测试 |
| `cyclictest-musl` | `judge_cyclictest-musl.py` | 实时性（延迟）测试 |
| `ltp-musl` | `judge_ltp-musl.py` | Linux Test Project |

### 4.2 评测输出解析流程

`parse_serial_out_new()` 函数的工作逻辑：

```python
for line in file:
    if "#### OS COMP TEST GROUP START <group> ####":
        # 启动对应的 judge 子进程
        judge = Popen(judges[group], stdin=PIPE, stdout=PIPE)
    elif "#### OS COMP TEST GROUP END":
        # 关闭 judge 的 stdin，读取 stdout 获得 JSON 结果
        judge.stdin.close()
        result = json.loads(judge.stdout.read())
    else:
        # 将输出行喂给 judge 的 stdin
        judge.stdin.write(line)
```

**重要**：如果某个 group 在串口输出中没有出现，judge 仍然会被调用（stdin 为空），此时返回全 0 分。

### 4.3 各测试组的具体输出格式

#### basic 测试

每个测试用例需要输出：
```
========== START test_xxx ==========
... 测试具体输出 ...
========== END test_xxx ==========
```

judge 脚本为每个测试定义了 `TestBase` 子类，用正则匹配输出内容。例如：

- `test_brk`: 检查 `Before alloc,heap pos:`, `After alloc,heap pos:`, `Alloc again,heap pos:` 三行，验证地址递增 64
- `test_clone`: 检查 `Child says successfully!`, `pid:\d+`, `clone process successfully.`
- `test_write`: 检查 `Hello operating system contest.`

**共 31 个 basic 测试**: brk, chdir, clone, close, dup, dup2, execve, exit, fork, fstat, getcwd, getdents, getpid, getppid, gettimeofday, mkdir, mmap, mount, munmap, open, openat, pipe, read, sleep, times, umount, uname, unlink, wait, waitpid, write, yield

#### busybox 测试

检查格式：`testcase busybox <cmd> success` 或 `testcase busybox <cmd> fail`

judge 脚本有一份预定义命令列表（约 50 条），未出现在输出中的命令算 fail。

#### libctest 测试

检查格式：
```
========== START entry-static.exe <test_name> ==========
Pass!
========== END entry-static.exe <test_name> ==========
```

对 static 和 dynamic 两套测试分别统计，每个测试通过得 1 分。baseline 中有约 100 个测试。

#### libcbench 测试

检查格式：
```
b_malloc_sparse (0)
  time: 0.384919462, virt: 39376, res: 5348, dirty: 5348
```

提取 `time: <float>` 值，与 baseline 对比计算性能得分。

#### lmbench 测试

检查格式：
```
latency measurements
Simple syscall: 9.25013 microseconds
...
Pipe bandwidth: 127.244 MB/sec
...
context switch overhead
2 41.24
4 41.58071
...
```

解析延迟（microseconds）、带宽（KB/sec, MB/sec）、上下文切换等指标。

#### lua 测试

检查格式：`testcase lua <script_name> success/fail`

9 个 lua 脚本测试。

---

## 5. 评分机制

### 5.1 功能性测试评分

**basic / busybox / libctest / lua / ltp**：

每个测试项通过得 1 分，不通过得 0 分。总分 = 各项得分之和。

例如 basic 有 31 个测试，每个测试内部有多个 assert，pass 数等于 assert 通过数（不一定全通过就得满分）。

### 5.2 性能测试评分

**lmbench / libcbench / iozone / iperf / netperf / cyclictest / unixbench**：

使用 `generate_score()` 函数计算：

```python
def generate_score(results, baseline):
    for item in lmbench:
        if item["res"] > 0:
            # 延迟类指标：baseline 越小越好
            if "microseconds" or "seconds" in name:
                score = baseline / result  # baseline / 你的值
            else:
                # 吞吐类指标：结果越大越好
                score = result / baseline  # 你的值 / baseline

            # 归一化：最高 2 分
            if score >= 1:
                score = 2 - (1 / score)   # 渐近 2.0
            else:
                score = 1.0               # 低于 baseline 得 1.0（保底分）
```

**评分逻辑解读**：
- 如果你的性能 = baseline：score = 1.0
- 如果你的性能 = 2x baseline：score = 1.5
- 如果你的性能 = 10x baseline：score = 1.9
- 如果你的性能 < baseline：score = 1.0（保底，不惩罚）
- 如果你的性能 = 0（未输出）：score = 0.0
- 理论最高 score 渐近 2.0

### 5.3 总分计算

```
总分 = sum(所有 group 的 score) * 4 (rv + la, musl + glibc)
```

分数矩阵示例（来自 3-19-result.txt）：

| 测试点 | glibc-la | glibc-rv | musl-la | musl-rv | 总分 |
|---|---|---|---|---|---|
| basic | 41 | 41 | 41 | 41 | 164 |
| busybox | 52 | 48 | 52 | 53 | 205 |
| libctest | - | - | 209 | 209 | 418 |
| **总分** | **93.0** | **89.0** | **302.0** | **303.0** | **787.0** |

---

## 6. 本地评测操作步骤

### 6.1 前置准备

```bash
# 1. 拉取 Docker 镜像
sudo docker pull zhouzhouyi/os-contest:20260104
# 或使用评测指定镜像
# docker.educg.net/cg/os-contest:20250714

# 2. 准备测试数据目录
mkdir -p /path/to/data

# 3. 复制 judge 脚本
cp -rf autotest-for-oskernel/kernel/judge/* /path/to/data/

# 4. 下载 sdcard 镜像
cd /path/to/data
wget https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-rv.img.xz
wget https://github.com/oscomp/testsuits-for-oskernel/releases/download/pre-20250615/sdcard-la.img.xz

# 5. 解压并 gzip（评测系统内部会 gunzip）
unxz sdcard-rv.img.xz && gzip sdcard-rv.img
unxz sdcard-la.img.xz && gzip sdcard-la.img

# 6. 配置 judge config.json（可选，修改 timeout 等）
# /path/to/data/config.json 内容：
# {"debug": false, "qemu.smp": 1, "qemu.mem": "1G", "qemu.timeout": 3600}

# 7. 打包 kernel.zip
cd autotest-for-oskernel/kernel
zip ../kernel.zip -r *
```

### 6.2 运行评测

```bash
sudo docker run --rm \
  -v /path/to/your/os:/coursegrader/submit \
  -v /path/to/data:/coursegrader/testdata \
  -v /path/to/autotest-for-oskernel:/cg \
  -v /path/to/data:/mnt/cghook/ \
  docker.educg.net/cg/os-contest:20250714 python3 /cg/kernel.zip
```

**注意事项**：
- Docker 需要 `qemu-system-riscv64` 和 `qemu-system-loongarch64`，Docker 镜像中已内置
- 停止评测用 `docker stop`，**不要** Ctrl+C 强杀 Python 脚本（会导致文件锁）
- 超时默认 3600 秒（1 小时），可通过 `config.json` 调整

### 6.3 对 rcore-lab 的适配要求

基于源码分析，rcore-lab 要满足以下条件才能被评测系统正确评测：

1. **Makefile `all` 目标**: 编译后在当前目录生成 `kernel-rv`（RISC-V 内核二进制）
2. **sdcard 镜像**: 评测系统会将 `sdcard-rv.img` 复制到编译目录，内核需要从 virtio-blk 设备读取
3. **串口输出协议**: stdout 必须包含 `#### OS COMP TEST GROUP START/END ####` 标记
4. **测试用例格式**: basic 测试需要 `========== START/END ==========`；busybox 需要 `testcase busybox <cmd> success/fail`；libctest 需要 `========== START entry-static.exe <name> ==========` + `Pass!`

---

## 7. 输出协议对照：rcore-lab 与评测系统

### 7.1 当前 rcore-lab 的输出格式（run.sh）

rcore-lab 的 `run.sh` 使用自己的测试框架运行 sdcard 上的测试用例。需要确认其输出是否符合评测系统的协议。

评测系统期望的关键输出标记：

```
#### OS COMP TEST GROUP START basic-musl ####
========== START test_brk ==========
Before alloc,heap pos: 4096
After alloc,heap pos: 4160
Alloc again,heap pos: 4224
========== END test_brk ==========
...
#### OS COMP TEST GROUP END ####
```

**如果 rcore-lab 的输出不包含 `#### OS COMP TEST GROUP START/END ####` 标记**，则所有 judge 脚本都会收到空输入，返回 0 分。

### 7.2 适配方案

实际上，这些标记是由 **sdcard 镜像中的测试脚本** 产生的，而不是内核本身输出的。sdcard 中的 shell 脚本（如 `run_test.sh`）会在执行每组测试前后打印这些标记。

因此：
- 内核只需要**正确运行 sdcard 中的测试程序**并将其 stdout 转发到串口即可
- 不需要内核自己生成这些标记
- 关键是内核的 `initcode` 要能执行 sdcard 上的 busybox shell 脚本

---

## 8. 针对 rcore-lab 的得分提升建议

基于 3-19-result.txt 的当前得分（787 分）分析：

### 8.1 已通过（787 分）
- basic: 164/164（接近满分，yield 为 0）
- busybox: 205（glibc-rv 略低）
- libctest: 418（仅 musl 有分，glibc 无分）

### 8.2 零分项（最大提升空间）
- **libcbench**: 需要内核支持运行 libc-bench（需要 pthread、malloc 等），输出格式是 `b_xxx_yyy (args)\n  time: x.xxx`
- **lmbench**: 需要运行 lmbench 套件，输出以 `latency measurements` 开始的标准格式
- **lua**: 需要内核能运行 Lua 解释器，输出 `testcase lua xxx.lua success/fail`
- **iozone / iperf / netperf**: 需要网络栈支持和文件系统性能
- **cyclictest**: 需要实时调度支持
- **ltp**: Linux Test Project，需要广泛的 POSIX 兼容性
- **glibc 系列**: glibc 版 libctest 当前无分，可能是 glibc 动态链接器路径问题

### 8.3 短期提升策略

1. **修复 test_yield**: basic 中唯一 0 分项，修好即可 +8 分
2. **修复 glibc libctest**: 解决 glibc 动态链接问题，可能 +400 分
3. **支持 libcbench**: 需要 pthread create/join 性能正常，每项性能测试保底 1.0 分，共 27 项 * 4 arch = ~108 分
4. **支持 lmbench**: 需要 fork/exec/pipe/mmap 等系统调用性能正常，共 ~33 项指标 * 4 arch = ~132 分
5. **支持 lua**: 需要内核能运行 Lua 解释器，9 个测试 * 4 arch = ~36 分

---

## 9. 附录：QEMU 配置参数对照

| 参数 | 值 | 说明 |
|---|---|---|
| `-machine` | `virt` | RISC-V virt 平台 |
| `-kernel` | `kernel-rv` | 内核二进制文件名 |
| `-m` | `1G` | 内存大小 |
| `-smp` | `1` | CPU 核心数 |
| `-bios` | `default` | 使用默认 OpenSBI |
| `-drive` | `file=sdcard-rv.img,...` | virtio-blk 磁盘设备 |
| `-no-reboot` | - | 内核 panic 不重启 |
| `-device virtio-net-*` | - | 虚拟网卡 |
| `-rtc base=utc` | - | RTC 使用 UTC 时间 |
| 超时 | 3600 秒 | 超时后 kill QEMU |

**与 rcore-lab 本地测试的差异**:
- 评测系统使用 `kernel-rv` 而非 `os` 作为内核文件名
- 评测系统使用 `-bios default`（OpenSBI），rcore-lab 可能使用自定义 SBI
- 评测系统内存 1G，确保本地测试也用 1G
- 评测系统 `-no-reboot`，内核 panic 后 QEMU 直接退出

---

## 10. 附录：judge 脚本输入输出格式

每个 judge 脚本都是独立的 Python 程序：
- **输入**: 从 stdin 读取对应 group 的串口输出文本
- **输出**: 向 stdout 输出 JSON 数组

JSON 格式：
```json
[
  {"name": "test_brk", "pass": 2, "all": 3, "score": 2},
  {"name": "test_clone", "pass": 4, "all": 4, "score": 4},
  ...
]
```

性能测试的 JSON 格式：
```json
[
  {"name": "lmbench Simple syscall:(microseconds)", "res": 9.25, "baseline": 9.25013, "score": 1.0},
  ...
]
```

可以手动测试 judge 脚本：
```bash
# 从你的内核输出文件中提取某个 group 的内容，喂给对应 judge
cat os_serial_out_rv.txt | python3 judge_basic-musl.py
```

这对于调试非常有用 — 可以直接看到每个测试的 pass/fail 状态。
