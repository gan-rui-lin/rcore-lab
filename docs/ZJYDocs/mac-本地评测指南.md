# macOS (Apple Silicon) 本地评测指南

**日期**: 2026/3/21

---

## 1. 背景

oscomp 官方评测系统 (`autotest-for-oskernel`) 的 Docker 镜像是 x86_64 架构。在 Apple Silicon Mac 上无法直接运行，需要额外配置 x86 模拟。本文档记录了实测可行的两种评测方式。

---

## 2. 前置准备（通用）

### 2.1 目录约定

```
~/Desktop/project/syscall/
├── rcore-lab/                    # 内核源码（$os）
├── autotest-for-oskernel/        # 评测系统仓库
└── testsuits-for-oskernel/       # 测试用例源码（参考用）
```

### 2.2 准备评测数据目录

```bash
# 使用 /Users 下的路径（Colima 会自动共享，/tmp 不行）
DATA_DIR=$os/autotest-data
mkdir -p "$DATA_DIR"

# 复制 judge 脚本
cp -rf autotest-for-oskernel/kernel/judge/* "$DATA_DIR/"

# 准备 sdcard 镜像（gzip 格式，评测系统内部会 gunzip）
gzip -c sdcard-rv.img > "$DATA_DIR/sdcard-rv.img.gz"
gzip -c sdcard-la.img > "$DATA_DIR/sdcard-la.img.gz"
# 如果没有 LA 镜像，可用空占位：
# dd if=/dev/zero bs=1M count=1 | gzip > "$DATA_DIR/sdcard-la.img.gz"
```

### 2.3 打包 kernel.zip

```bash
cd autotest-for-oskernel/kernel
zip -r ../kernel.zip *
```

---

## 3. 方式一：Docker 完整评测（推荐）

### 3.1 安装 Colima + x86 模拟

rcore-lab 开发环境使用 Colima 而非 Docker Desktop。Colima 的 VM 是 aarch64 Linux，需要安装 `qemu-user-static` 来支持 x86 容器：

```bash
# 拉取评测镜像
docker pull zhouzhouyi/os-contest:20260104

# 在 Colima VM 中安装 x86 模拟支持
colima ssh -- sudo apt-get update -qq
colima ssh -- sudo apt-get install -y -qq qemu-user-static binfmt-support
```

安装后 Docker 即可通过 `--platform linux/amd64` 运行 x86 容器。

> **注意**：如果你用的是 Docker Desktop，改为在 Settings → General 中勾选 "Use Rosetta for x86_64/amd64 emulation on Apple Silicon"，无需安装 qemu-user-static。

### 3.2 预编译内核

Docker 容器中的 x86 Rust 工具链在 QEMU 用户态模拟下会 SIGSEGV，**无法在容器内编译**。解决方案：在宿主机预编译，提交预构建产物。

```bash
# 在宿主机编译
cd $os
LOG=OFF make rv    # 产出 kernel-rv

# 创建提交目录（带预编译好的 kernel-rv 和空壳 Makefile）
SUBMIT_DIR=$os/autotest-submit
mkdir -p "$SUBMIT_DIR"
cp kernel-rv "$SUBMIT_DIR/"
cat > "$SUBMIT_DIR/Makefile" << 'EOF'
all:
	@echo "pre-built"
EOF
```

如果同时有 LoongArch 内核，也拷贝 `kernel-la` 到 `$SUBMIT_DIR/`。

### 3.3 运行评测

```bash
SUBMIT_DIR=$os/autotest-submit
DATA_DIR=$os/autotest-data
CG_DIR=~/Desktop/project/syscall/autotest-for-oskernel

docker run --rm --platform linux/amd64 \
  -v "$SUBMIT_DIR":/coursegrader/submit \
  -v "$DATA_DIR":/coursegrader/testdata \
  -v "$CG_DIR":/cg \
  -v "$DATA_DIR":/mnt/cghook/ \
  zhouzhouyi/os-contest:20260104 python3 /cg/kernel.zip
```

输出 JSON 中 `verdict: "Accepted"` 表示评测成功，`score` 为总分，`rank` 包含各测试组分数。

### 3.4 注意事项

- **路径必须在 `/Users/` 下**：Colima 默认共享 `/Users`，但不共享 `/tmp`。如果用 `/tmp` 路径，Docker 容器内看到的是空目录。
- **停止评测**：用 `docker stop <container_id>`，不要 Ctrl+C。
- **超时**：默认 3600 秒，可在 `$DATA_DIR/config.json` 中修改 `qemu.timeout`。
- **sdcard 被修改**：每次 QEMU 运行后 sdcard 镜像会被写入。如果需要重复评测，建议每次从原始镜像重新 gzip。
- **x86 模拟很慢**：在 QEMU 用户态模拟下，Docker 内的 QEMU-system 是嵌套模拟（x86 模拟 → RISC-V 模拟），速度约为原生的 1/5~1/10。basic 测例约 4 分钟完成。

### 3.5 实测结果示例

2026/3/21 在 `lmbench-test` 分支上（initcode 只开 basic）：

```
verdict: "Accepted"
score: 204

basic-glibc-rv: 102.0  ← 满分
basic-musl-rv:  102.0  ← 满分
basic-glibc-la:   0.0  (未提供 kernel-la)
basic-musl-la:    0.0  (未提供 kernel-la)
```

---

## 4. 方式二：本地 judge 脚本评分（快速验证）

不通过 Docker，直接在宿主机跑 QEMU + 官方 judge 脚本。**judge 脚本与 Docker 内调用的完全相同**，只是跳过了 Docker 编译环节。

### 4.1 运行测试并捕获输出

```bash
cd $os

# 选择要测的套件（通过 SINGLE_TEST 环境变量）
# 例：只跑 glibc-basic
SINGLE_TEST=glibc-basic LOG=OFF bash run.sh -f sdcard-rv.img -t all > output.log 2>&1

# 或跑全部
LOG=OFF bash run.sh -f sdcard-rv.img -t all > output.log 2>&1
```

> **建议**：run.sh 中 QEMU 内存改为 `-m 1G` 以匹配竞赛环境（默认 128M）。

### 4.2 用官方 judge 评分

```bash
JUDGE_DIR=~/Desktop/project/syscall/autotest-for-oskernel/kernel/judge

# 评 basic-glibc
cat output.log | python3 "$JUDGE_DIR/judge_basic-glibc.py"

# 评 basic-musl
cat output.log | python3 "$JUDGE_DIR/judge_basic-musl.py"

# 评 busybox-glibc
cat output.log | python3 "$JUDGE_DIR/judge_busybox-glibc.py"
```

输出 JSON 数组，每个元素包含 `name`、`pass`、`all`、`score`。

### 4.3 快速汇总脚本

```bash
# 一行命令算总分
cat output.log | python3 "$JUDGE_DIR/judge_basic-glibc.py" 2>/dev/null | \
  python3 -c "
import json,sys
data = json.loads(sys.stdin.read())
tp = sum(t['pass'] for t in data)
ta = sum(t['all'] for t in data)
print(f'Score: {tp}/{ta}')
fails = [t for t in data if t['pass'] < t['all']]
for t in fails: print(f'  FAIL: {t[\"name\"]:25s} {t[\"pass\"]}/{t[\"all\"]}')
if not fails: print('  ALL PASS!')
"
```

### 4.4 本地 vs Docker 差异

| 维度 | 本地 judge | Docker 完整评测 |
|------|-----------|----------------|
| 编译 | 宿主机 native | 容器内（x86 模拟，会崩，需预编译） |
| QEMU | 宿主机 native (快) | 容器内 x86 模拟 (慢) |
| judge 脚本 | 完全相同 | 完全相同 |
| 评分准确性 | 等价 | 等价 |
| 速度 | 快（~30秒/套件） | 慢（~4分钟/套件） |
| 适用场景 | 日常开发迭代 | 提交前最终验证 |

**结论**：日常开发用方式二（本地 judge）即可，提交前用方式一（Docker）做最终确认。

---

## 5. 控制测试范围

评测系统会跑 sdcard 上所有测试套件。要缩小范围（加速调试），修改 `user/src/bin/initcode.rs` 中的 `TEST_SUITES`：

```rust
const TEST_SUITES: [&str; 1] = [
    "basic",
    // "busybox",
    // "libctest",
    // ...
];
```

修改后需重新编译 `kernel-rv`。

也可以通过 `SINGLE_TEST` 环境变量在编译期注入（不改源码）：

```bash
# 只跑 glibc 的 basic
SINGLE_TEST=glibc-basic LOG=OFF bash run.sh -f sdcard-rv.img -t all

# 只跑某个具体 ELF
SINGLE_TEST=/glibc/basic/fork LOG=OFF bash run.sh -f sdcard-rv.img -t all
```

详见 [run-sh-测试指南.md](run-sh-测试指南.md)。

---

## 6. 常见问题

### Q: Docker 报 `exec format error`
Colima VM 未安装 x86 模拟。执行：
```bash
colima ssh -- sudo apt-get install -y qemu-user-static binfmt-support
```

### Q: Docker 容器内编译 SIGSEGV
x86 Rust 工具链在 QEMU 用户态模拟下不稳定。使用预编译方案（见 3.2）。

### Q: Docker 容器内目录为空
确保路径在 `/Users/` 下。Colima 不共享 `/tmp`。

### Q: sdcard 镜像被锁 / Failed to lock byte 100
上一次 QEMU 没正常退出。检查 `lsof sdcard-rv.img`，kill 残留 QEMU 进程。

### Q: 评测超时
修改 `$DATA_DIR/config.json` 中 `qemu.timeout`（默认 3600 秒）。
