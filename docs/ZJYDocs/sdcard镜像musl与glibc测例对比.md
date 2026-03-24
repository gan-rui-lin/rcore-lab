# sdcard 镜像 musl/ 与 glibc/ 测例对比

**日期**: 2026/3/24

---

## 1. 总体结构

sdcard 镜像（`sdcard-la.img` / `sdcard-rv.img`）的根目录下有两个平行的测试目录：

```
/
├── musl/            ← musl libc 编译的测试套件
│   ├── lib/         ← musl 动态库
│   ├── busybox      ← 静态链接的 busybox（shell + 工具链）
│   ├── iperf3       ← 静态链接
│   ├── netperf      ← 动态链接 (musl)
│   ├── netserver    ← 动态链接 (musl)
│   ├── *_testcode.sh ← 测试脚本
│   └── ...          ← 其他测试二进制
│
├── glibc/           ← glibc 编译的测试套件
│   ├── lib/         ← glibc 动态库
│   ├── busybox      ← 静态链接的 busybox
│   ├── iperf3       ← 静态链接
│   ├── netperf      ← 动态链接 (glibc, PIE)
│   ├── netserver    ← 动态链接 (glibc, PIE)
│   ├── *_testcode.sh ← 测试脚本（与 musl 几乎相同）
│   └── ...
```

两个目录包含**完全相同的测试套件**（binary 名称一一对应），区别仅在于：
1. 链接的 C 库不同（musl vs glibc）
2. 部分二进制的链接方式不同（静态 vs 动态）
3. 测试脚本中的 GROUP 标签不同（`iperf-musl` vs `iperf-glibc`）

---

## 2. 链接方式差异

### 2.1 LoongArch64 (sdcard-la.img)

| 二进制 | musl | glibc |
|--------|------|-------|
| busybox | 静态链接 | 静态链接 |
| iperf3 | **静态链接** | **静态链接** |
| netperf | 动态链接, interp=`/lib64/ld-musl-loongarch-lp64d.so.1` | 动态链接 PIE, interp=`/lib64/ld-linux-loongarch-lp64d.so.1` |
| netserver | 动态链接, interp=同上 | 动态链接 PIE, interp=同上 |
| lmbench 系列 | 静态链接 | 静态链接 |
| lua | 静态链接 | 静态链接 |

### 2.2 RISC-V64 (sdcard-rv.img)

| 二进制 | musl | glibc |
|--------|------|-------|
| busybox | 静态链接 (soft-float) | 静态链接 (soft-float) |
| iperf3 | **静态链接** | **静态链接** |
| netperf | 动态链接, interp=`/lib/ld-musl-riscv64-sf.so.1` | 动态链接 PIE, interp=`/lib/ld-linux-riscv64-lp64d.so.1` |
| netserver | 动态链接, interp=同上 | 动态链接 PIE, interp=同上 |

**关键观察**：
- **iperf3 在两个 libc 下都是静态链接**——不依赖动态链接器，因此 glibc 动态链接有问题时 iperf3 仍可正常运行
- **netperf/netserver 在两个 libc 下都是动态链接**——是唯一真正依赖动态链接器的网络测试程序
- glibc 版本的 netperf 还是 **PIE**（Position-Independent Executable），增加了加载复杂度

---

## 3. 动态库差异

### musl/lib/

```
libc.so                    ← musl libc（同时也是动态链接器 ld-musl-*.so.1）
dlopen_dso.so              ← dlopen 测试用 DSO
tls_align_dso.so           ← TLS 对齐测试用 DSO
tls_get_new-dtv_dso.so     ← TLS DTV 测试用 DSO
tls_init_dso.so            ← TLS 初始化测试用 DSO
```

musl 的设计特点：`libc.so` 本身就是动态链接器，不需要单独的 `ld.so`。内核在 `initcode.rs` 中创建硬链接（如 `/lib64/ld-musl-loongarch-lp64d.so.1` -> `/musl/lib/libc.so`）使 interpreter 路径可用。

### glibc/lib/

```
ld-linux-loongarch-lp64d.so.1  ← glibc 动态链接器（ld.so）
libc.so.6                       ← glibc C 库
libm.so.6                       ← glibc 数学库
dlopen_dso.so                   ← 测试用 DSO
tls_*_dso.so                    ← TLS 测试用 DSO
```

glibc 的 ld.so 和 libc 是分开的。内核需要：
1. 创建 `/lib64/ld-linux-loongarch-lp64d.so.1` -> `/glibc/lib/ld-linux-loongarch-lp64d.so.1` 硬链接
2. 设置 `LD_LIBRARY_PATH=/glibc/lib` 环境变量，使 ld.so 能找到 libc.so.6 和 libm.so.6

### glibc 版本差异

| | LoongArch | RISC-V |
|---|-----------|--------|
| glibc 版本 | 2.38 (GLIBC_2.36 + GLIBC_2.38) | 2.35 (GLIBC_2.27 ~ GLIBC_2.35) |
| netperf 所需版本 | GLIBC_2.36 | GLIBC_2.27 |
| 符号版本化 | 全量版本化（所有符号标记 @GLIBC_2.36） | 部分版本化 |
| `DT_RELR` | 有（紧凑重定位格式） | 无 |

LoongArch 的 glibc 更新（2.36 是 LoongArch 首次支持的版本），因此所有符号都标记为 GLIBC_2.36，对动态链接器的版本解析能力要求更高。

---

## 4. 测试脚本差异

musl 和 glibc 的测试脚本几乎完全相同，唯一差异是 GROUP 标签：

```bash
# musl/iperf_testcode.sh
./busybox echo "#### OS COMP TEST GROUP START iperf-musl ####"

# glibc/iperf_testcode.sh
./busybox echo "#### OS COMP TEST GROUP START iperf-glibc ####"
```

测试逻辑、参数、端口号完全一致。评分脚本通过 GROUP 标签区分来源。

### iperf 测试流程

```bash
iperf3 -s -p 5001 -D              # 后台启动服务器
iperf3 -c 127.0.0.1 -p 5001 ...   # 客户端连接，6 个子测试
```

6 个子测试：BASIC_UDP, BASIC_TCP, PARALLEL_UDP(P5), PARALLEL_TCP(P5), REVERSE_UDP, REVERSE_TCP

### netperf 测试流程

```bash
netserver -D -L 127.0.0.1 -p 12865 &   # 后台启动服务器
netperf -H 127.0.0.1 -p 12865 ...       # 客户端测试，5 个子测试
kill -9 $server_pid                       # 清理服务器
```

5 个子测试：UDP_STREAM, TCP_STREAM, UDP_RR, TCP_RR, TCP_CRR

---

## 5. 对内核的要求差异

| 能力 | musl 测试 | glibc 测试 | 说明 |
|------|-----------|------------|------|
| 静态 ELF 加载 | 需要 | 需要 | busybox, iperf3 等 |
| 动态链接（musl） | 需要 | - | netperf/netserver 通过 musl libc.so 加载 |
| 动态链接（glibc） | - | 需要 | netperf/netserver 通过 glibc ld.so + libc.so.6 加载 |
| PIE 支持 | - | 需要 | glibc netperf 是 PIE，需要非零 load_base |
| GNU 符号版本解析 | - | 需要 | glibc 2.36 所有符号都版本化 |
| MAP_FIXED 语义 | - | 需要 | glibc ld.so 使用 MAP_FIXED 映射库 |
| mmap COW | - | 需要 | 共享页 copy-on-write |
| 网络 loopback | 需要 | 需要 | 所有测试用 127.0.0.1 |
| SIGALRM 定时器 | 需要 | 需要 | netperf 用 setitimer 控制测试时长 |
| fork + exec | 需要 | 需要 | 测试脚本通过 busybox sh 执行 |

**结论**：musl 版本对内核要求较低（只需基本动态链接），glibc 版本额外需要 PIE 加载、MAP_FIXED、COW、GNU 版本符号等高级特性。如果 glibc 动态链接不完善，只有静态链接的测试（iperf3）能通过。

---

## 6. 当前 LoongArch 测试状态

| 测试 | musl | glibc | 原因 |
|------|------|-------|------|
| iperf (6 项) | 6/6 PASS | 6/6 PASS | 静态链接，不依赖动态链接器 |
| netperf UDP_STREAM | PASS | FAIL | glibc ld.so 符号版本解析失败 |
| netperf TCP_STREAM | PASS | FAIL | 同上 |
| netperf UDP_RR | PASS | FAIL | 同上 |
| netperf TCP_RR | PASS | FAIL | 同上 |
| netperf TCP_CRR | FAIL (EBADF) | FAIL | musl: fd 管理问题; glibc: 动态链接失败 |
