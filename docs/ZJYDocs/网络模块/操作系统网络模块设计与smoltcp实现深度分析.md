# 操作系统网络模块设计与 smoltcp 实现深度分析

**日期**: 2026/03/19
**分支**: `glibc-test` / `muti-arch`
**目标读者**: 正在学习如何为教学/嵌入式操作系统编写网络模块的开发者

---

## 一、引言：为什么操作系统需要网络模块

现代用户态程序（curl、wget、浏览器、数据库客户端、甚至 busybox 的 httpd/telnetd）依赖 **BSD Socket API** 进行网络通信。这套 API 是 POSIX 标准的一部分，其调用链如下：

```
用户态: socket() → bind() → listen() → accept() → read()/write() → close()
                          ↓ ecall / syscall 陷入
内核态: sys_socket → ... → TCP/IP 协议栈 → 网卡驱动 → 物理网络
```

操作系统在这条链路中承担了**六层**关键职责：

| 层次 | 职责 | 关键问题 |
|------|------|---------|
| 1. 硬件驱动层 | 与网卡硬件交互，收发以太网帧 | DMA 映射、中断处理、环形缓冲区管理 |
| 2. 协议栈层 | TCP/IP 协议处理（ARP、IP、TCP、UDP） | 状态机、拥塞控制、分片重组、校验和 |
| 3. Socket 抽象层 | 将协议栈能力封装为文件描述符 | 生命周期管理、阻塞/非阻塞语义、poll 支持 |
| 4. 系统调用层 | 将内核能力暴露给用户态 | 参数校验、地址转换、权限检查 |
| 5. 网络轮询层 | 驱动协议栈定期处理收发包 | 中断 vs 轮询、锁竞争、延迟保证 |
| 6. 地址/配置管理 | IP 地址分配、路由表、DNS | 静态配置 vs DHCP、多接口路由 |

缺少任何一层，用户态程序都无法正常联网。下面我们逐层深入分析每一层**需要做什么**、**为什么这样做**、以及 rCore-Lab 当前基于 smoltcp 的实现**是怎么做的**。

---

## 二、第一层：硬件驱动 — VirtIO-Net 设备适配

### 2.1 背景知识：VirtIO 与 virtqueue

在 QEMU 虚拟化环境中，网卡通常以 **VirtIO** 标准呈现。VirtIO 定义了一套通用的虚拟设备接口，核心抽象是 **virtqueue**（虚拟队列）：

- **Descriptor Table**: 描述符数组，每个描述符指向一块 DMA 内存（地址 + 长度 + 标志）
- **Available Ring**: Guest → Host 方向，Guest 填好描述符后将索引写入此环
- **Used Ring**: Host → Guest 方向，Host 处理完后将结果写入此环

VirtIO-Net 设备有两个 virtqueue：
- **RX Queue (queue 0)**: 网卡收到的数据包放入此队列
- **TX Queue (queue 1)**: 操作系统要发送的数据包放入此队列

每个 VirtIO 设备通过 **MMIO（Memory-Mapped I/O）** 映射到固定的物理地址。在 QEMU `virt` machine 上，VirtIO MMIO 区域从 `0x1000_1000` 开始，每个设备间隔 `0x1000`：

```
0x1000_1000  virtio-mmio-bus.0  (通常是 virtio-blk 块设备)
0x1000_2000  virtio-mmio-bus.1  (可能是 virtio-net 网络设备)
0x1000_3000  virtio-mmio-bus.2  ...
...
0x1000_8000  virtio-mmio-bus.7
```

### 2.2 为什么需要 smoltcp Device trait

smoltcp 协议栈不直接操作硬件，而是定义了一个 **Device trait** 作为物理层的抽象接口。操作系统需要实现这个 trait 来桥接硬件驱动与协议栈：

```rust
pub trait Device {
    type RxToken<'a>: RxToken where Self: 'a;
    type TxToken<'a>: TxToken where Self: 'a;

    fn receive(&mut self, timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)>;
    fn transmit(&mut self, timestamp: Instant) -> Option<Self::TxToken<'_>>;
    fn capabilities(&self) -> DeviceCapabilities;
}
```

这个设计采用了 **Token（令牌）模式**，有三个精妙之处：

1. **零拷贝友好**：`receive()` 返回 `(RxToken, TxToken)` 对，smoltcp 在处理收到的包时可以同时构造回复（如 ARP 响应、ICMP echo reply），避免额外的缓冲区拷贝。
2. **所有权安全**：Token 使用 `self`（move 语义），保证一个包只被处理一次，编译期防止重复消费。
3. **非阻塞设计**：没有包可收时返回 `None`，绝不忙等待（spin_loop），这对内核环境至关重要——忙等待会阻塞整个 CPU 核心。

### 2.3 rCore-Lab 的实现：VirtIONetDevice

当前实现位于 [os/src/drivers/net/mod.rs](os/src/drivers/net/mod.rs)，共 135 行。

**设备发现**（动态 MMIO 扫描）：

```rust
fn find_virtio_net_header() -> &'static mut VirtIOHeader {
    for index in 0..VIRTIO_MMIO_SLOTS {
        let addr = VIRTIO_MMIO_BASE + index * VIRTIO_MMIO_STRIDE;
        let header = unsafe { &mut *(addr as *mut VirtIOHeader) };
        if !header.verify() { continue; }
        if header.device_type() == DeviceType::Network {
            return header;
        }
    }
    panic!("virtio-net mmio device not found");
}
```

这段代码遍历 8 个 MMIO slot，通过 VirtIO header 的 `magic number` 验证 (`verify()`) 和设备类型字段 (`device_type()`) 来定位网络设备。**为什么不硬编码地址？** 因为 QEMU 命令行中 `-device virtio-net-device,bus=virtio-mmio-bus.N` 的 N 可以变化，动态扫描更健壮。

**接收 Token 实现**：

```rust
pub struct VirtioRxToken {
    buf: Vec<u8>,  // 拥有数据的独立缓冲区
}

impl phy::RxToken for VirtioRxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where F: FnOnce(&mut [u8]) -> R {
        f(&mut self.buf)
    }
}
```

注意 `buf` 是 `Vec<u8>` 而非引用——这是因为 VirtIO 驱动的 `recv()` 将数据写入驱动内部的 `rx_buf`，然后我们 `.to_vec()` 拷贝出来。虽然多了一次拷贝，但避免了生命周期问题（RxToken 需要在 `receive()` 返回后仍然有效）。

**发送 Token 实现**：

```rust
pub struct VirtioTxToken<'a> {
    driver: &'a mut VirtIONet<'static, VirtioHal>,
}

impl<'a> phy::TxToken for VirtioTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where F: FnOnce(&mut [u8]) -> R {
        let mut buf = vec![0u8; len];
        let result = f(&mut buf);       // smoltcp 填充以太网帧
        self.driver.send(&buf).ok();     // 发送到 VirtIO TX queue
        result
    }
}
```

**设备能力声明**：

```rust
fn capabilities(&self) -> DeviceCapabilities {
    let mut caps = DeviceCapabilities::default();
    caps.medium = Medium::Ethernet;           // 以太网介质
    caps.max_transmission_unit = 1514;        // MTU 1500 + 14 字节以太网头
    caps.max_burst_size = Some(1);            // 单次 poll 最多处理 1 个包
    caps
}
```

`Medium::Ethernet` 告诉 smoltcp 需要处理以太网帧头（dst MAC + src MAC + EtherType），这影响协议栈是否执行 ARP 解析。`max_burst_size = Some(1)` 限制了单次 `poll()` 的处理量，避免在一次轮询中处理过多包导致延迟。

---

## 三、第二层：TCP/IP 协议栈 — 为什么选择 smoltcp

### 3.1 操作系统需要处理哪些网络协议

一个最小可用的 TCP/IP 协议栈需要处理以下协议：

```
┌──────────────────────────────────────┐
│           应用层 (Socket API)          │
├──────────────────────────────────────┤
│   传输层: TCP (可靠字节流)             │
│          UDP (无连接数据报)            │
├──────────────────────────────────────┤
│   网络层: IPv4 (路由、分片、TTL)       │
│          ICMP (错误报告、ping)         │
│          ARP  (IP→MAC 地址解析)        │
├──────────────────────────────────────┤
│   链路层: Ethernet (帧封装/解封装)     │
└──────────────────────────────────────┘
```

如果从零实现，仅 TCP 一个协议就涉及：

- **连接管理**：三次握手（SYN → SYN-ACK → ACK）、四次挥手（FIN → ACK → FIN → ACK）
- **状态机**：RFC 793 定义了 11 个状态（CLOSED, LISTEN, SYN_SENT, SYN_RECEIVED, ESTABLISHED, FIN_WAIT_1, FIN_WAIT_2, CLOSE_WAIT, CLOSING, LAST_ACK, TIME_WAIT）
- **可靠传输**：序列号、确认号、重传定时器、超时重传
- **流量控制**：滑动窗口、窗口缩放（Window Scaling）
- **拥塞控制**：慢启动、拥塞避免、快速重传、快速恢复（Reno/NewReno/CUBIC）
- **校验和**：伪头部校验和计算与验证
- **乱序处理**：接收端乱序段重组

从零编写这些代码需要数千行，且极易出错（RFC 793 的实现细节非常多）。

### 3.2 smoltcp：为裸机/嵌入式设计的 Rust TCP/IP 栈

**smoltcp** 是一个专为 `no_std` 环境设计的 Rust TCP/IP 协议栈，具有以下特性：

| 特性 | 说明 | 对 OS 内核的意义 |
|------|------|-----------------|
| `no_std` 支持 | 不依赖标准库 | 可直接在内核中使用 |
| 零堆分配模式 | 所有缓冲区预分配 | 避免内核态内存分配失败 |
| 事件驱动 | 通过 `poll()` 驱动 | 不需要专用线程 |
| 完整 TCP 状态机 | 实现 RFC 793 全部状态 | 兼容真实应用程序 |
| 纯 Rust 实现 | 内存安全、无 UB | 适合教学与生产 |

rCore-Lab 使用的是 **smoltcp v0.11.0**，而非最新的 v0.12.0。原因是 v0.12 要求 Rust 2024 edition (`edition = "2024"`, 需要 Rust 1.85+)，而 rCore-Lab 的工具链是 `nightly-2024-05-02` (约 Rust 1.79)，两者不兼容。v0.11.0 使用 `edition = "2021"`，完美兼容。

### 3.3 smoltcp 的分层架构

```
┌─────────────────────────────────────────────────────┐
│              Socket 层 (tcp::Socket, udp::Socket)     │
│    提供面向应用的 send/recv/listen/connect API         │
├─────────────────────────────────────────────────────┤
│              Interface 层 (iface::Interface)           │
│    IP 地址管理、路由表、ARP 邻居缓存                    │
│    包解复用：入站包 → 目标 Socket                       │
│    包构造：出站数据 → 添加 IP/Ethernet 头              │
├─────────────────────────────────────────────────────┤
│              Wire 层 (wire::*)                         │
│    协议报文的解析与序列化                               │
│    Ethernet / ARP / IPv4 / TCP / UDP / ICMP            │
├─────────────────────────────────────────────────────┤
│              Physical 层 (phy::Device trait)            │
│    由操作系统实现，桥接硬件驱动                          │
└─────────────────────────────────────────────────────┘
```

smoltcp 的核心驱动循环只有一个函数调用：

```rust
iface.poll(timestamp, &mut device, &mut sockets);
```

这一次 `poll()` 会：
1. 调用 `device.receive()` 取出所有待处理的入站包
2. 解析以太网帧头 → IP 头 → TCP/UDP 头
3. 将数据投递到对应的 Socket 的接收缓冲区
4. 处理 TCP 状态机变迁（SYN、ACK、FIN 等）
5. 发送待发出的数据（TCP 重传、ARP 请求、ICMP 响应等）

### 3.4 启用的 Feature 与编译配置

```toml
# os/Cargo.toml
[dependencies.smoltcp]
path = "../vendor/smoltcp"
default-features = false
features = [
    "alloc",              # 允许使用 Vec 动态分配缓冲区
    "log",                # 集成 log crate 的日志输出
    "medium-ethernet",    # 以太网链路层处理（ARP、帧封装）
    "medium-ip",          # 纯 IP 链路层（用于 loopback 设备）
    "proto-ipv4",         # IPv4 协议支持
    "socket-tcp",         # TCP Socket 支持
    "socket-udp",         # UDP Socket 支持
    "socket-icmp",        # ICMP Socket 支持（ping）
    "socket-raw",         # Raw Socket 支持
]
```

**为什么不启用 `proto-ipv6`？** 简化首次实现。IPv6 支持可以后续添加，且 QEMU user-mode networking 默认使用 IPv4。

**为什么不启用 `socket-tcp-cubic`？** CUBIC 拥塞控制算法需要浮点运算（计算立方根），而内核态应尽量避免使用 FPU（浮点单元）。smoltcp 默认使用 NewReno（纯整数运算）。

---

## 四、第三层：全局网络栈 — 双接口模型

### 4.1 为什么需要两个网络接口

当前实现位于 [os/src/net/mod.rs](os/src/net/mod.rs)，定义了一个 **双接口** 的全局网络栈：

```rust
pub struct NetStack {
    pub device: VirtIONetDevice,      // 外部网络的 VirtIO 设备
    pub iface: Interface,              // 外部网络接口 (10.0.2.15/24)
    pub lo_device: Loopback,           // 回环设备
    pub lo_iface: Interface,           // 回环接口 (127.0.0.1/8)
    pub sockets: SocketSet<'static>,   // 所有 Socket 的集合（共享）
}
```

**为什么需要 loopback 接口？** 因为很多程序（尤其是测试程序）通过 `127.0.0.1` 进行本地通信。如果没有 loopback 接口，`connect(127.0.0.1:port)` 会失败，因为 smoltcp 的外部接口 (10.0.2.15) 不知道如何路由到 127.0.0.1。

loopback 使用 smoltcp 内置的 `Loopback` 设备（纯内存实现，不涉及任何硬件），配合 `Medium::Ip`（不需要以太网帧头和 ARP）。

### 4.2 网络栈初始化

```rust
pub fn init() {
    // 1. 创建 VirtIO 网络设备并读取 MAC 地址
    let mut device = VirtIONetDevice::new();
    let mac = device.mac_address();

    // 2. 创建外部网络接口，绑定 MAC 地址
    let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
    let mut config = Config::new(hw_addr);
    config.random_seed = get_time_ms() as u64;  // TCP 初始序列号随机化
    let mut iface = Interface::new(config, &mut device, now);

    // 3. 配置 IP 地址和默认路由（QEMU user-mode 默认值）
    iface.update_ip_addrs(|addrs| {
        addrs.push(IpCidr::new(IpAddress::v4(10, 0, 2, 15), 24)).unwrap();
    });
    iface.routes_mut()
        .add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2)).unwrap();

    // 4. 创建 loopback 接口
    let mut lo_device = Loopback::new(Medium::Ip);
    let lo_config = Config::new(HardwareAddress::Ip);  // 无以太网头
    let mut lo_iface = Interface::new(lo_config, &mut lo_device, now);
    lo_iface.update_ip_addrs(|addrs| {
        addrs.push(IpCidr::new(IpAddress::v4(127, 0, 0, 1), 8)).unwrap();
    });

    // 5. 创建共享的 SocketSet（最多 64 个并发 Socket）
    let sockets = SocketSet::new(
        (0..64).map(|_| SocketStorage::EMPTY).collect()
    );
}
```

**为什么 IP 是 10.0.2.15？** 这是 QEMU user-mode networking 的默认值。QEMU 在宿主机上创建一个虚拟 NAT 网络：Guest 的 10.0.2.15 通过 QEMU 内部的 NAT 访问宿主机网络，网关是 10.0.2.2。

**为什么需要 `random_seed`？** TCP 的初始序列号（ISN）需要随机化，以防止 TCP 序列号预测攻击（RFC 6528）。smoltcp 使用此种子生成 ISN。

### 4.3 网络轮询：何时驱动协议栈

smoltcp 是**事件驱动**的——它不会主动运行，需要外部定期调用 `poll()` 来处理收发包。rCore-Lab 采用**双轮询策略**：

**策略一：syscall 路径同步轮询**

```rust
pub fn poll_net() {
    let mut net = NET_STACK.exclusive_access();
    if let Some(ref mut stack) = *net {
        let now = smoltcp_now();
        stack.iface.poll(now, &mut stack.device, &mut stack.sockets);
        stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
    }
}
```

每次网络 syscall（`read`/`write`/`connect`/`accept` 等）的开头都会调用 `poll_net()`，确保在检查 Socket 状态前先处理所有待处理的网络事件。

**策略二：中断驱动异步轮询**

```rust
// os/src/boards/qemu.rs — PLIC 中断分发
VIRTIO_NET_IRQ => crate::net::poll_net_if_available(),
```

```rust
pub fn poll_net_if_available() {
    if let Some(mut net) = NET_STACK.try_exclusive_access() {
        // 只在锁可用时轮询，避免死锁
        ...
    }
}
```

当 VirtIO-Net 设备产生中断（有新的包到达），PLIC 将中断路由到内核，内核在中断处理中尝试轮询网络栈。注意使用 `try_exclusive_access()` 而非 `exclusive_access()`——如果锁已被 syscall 路径持有，直接跳过，避免中断上下文死锁。

**为什么需要双策略？** 单靠 syscall 路径轮询存在一个问题：如果用户态没有调用任何网络 syscall，协议栈就不会运行，TCP 的超时重传、keepalive 心跳等机制会失效。中断驱动轮询解决了这个问题。

### 4.4 临时端口分配

```rust
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

pub fn alloc_ephemeral_port() -> u16 {
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if port >= 49152 { return port; }
        NEXT_PORT.store(49152, Ordering::Relaxed);  // 回绕
    }
}
```

当用户态调用 `bind(0.0.0.0:0)` 或 `connect()` 时，内核需要自动分配一个**临时端口（ephemeral port）**。IANA 规定临时端口范围为 49152-65535。当前实现使用原子计数器递增分配，简单高效。

---

## 五、第四层：Socket 文件描述符抽象

### 5.1 为什么 Socket 必须是文件描述符

Unix 哲学的核心是 **"一切皆文件"**。网络 Socket 也不例外——它必须出现在进程的文件描述符表（fd_table）中，这样：

- `read(fd, buf, len)` 可以从 Socket 读取数据
- `write(fd, buf, len)` 可以向 Socket 发送数据
- `close(fd)` 可以关闭 Socket
- `poll(fds, nfds, timeout)` 可以监听 Socket 的可读/可写状态
- `dup(fd)` / `dup2(fd, newfd)` 可以复制 Socket 描述符
- `fcntl(fd, F_GETFD)` 可以获取 FD_CLOEXEC 标志

这意味着 Socket 必须实现内核的 `File` trait。

### 5.2 SocketFile 结构体

当前实现位于 [os/src/net/socket_file.rs](os/src/net/socket_file.rs)：

```rust
pub struct SocketFile {
    pub handle: SocketHandle,          // smoltcp 内部的 Socket 句柄
    pub sock_type: SocketType,         // TCP 或 UDP
    pub nonblock: bool,                // SOCK_NONBLOCK 标志
    pub cloexec: bool,                 // SOCK_CLOEXEC 标志
    pub bound_port: AtomicU16,         // 绑定的本地端口
    pub listening: AtomicBool,         // 是否处于 LISTEN 状态
    pub transferred: AtomicBool,       // 所有权是否已转移（accept 使用）
}
```

**为什么 `bound_port` 和 `listening` 用 Atomic？** 因为 `File` trait 的方法接收 `&self`（不可变引用），而这些字段需要在 `bind()`/`listen()` 中修改。使用原子类型实现**内部可变性（interior mutability）**，避免需要额外的锁。

### 5.3 File trait 实现要点

**读操作（TCP）**：

```rust
fn tcp_read(&self, mut user_buf: UserBuffer) -> usize {
    loop {
        poll_net();                    // 先驱动协议栈处理待收包
        let socket = /* 获取 smoltcp TCP Socket */;

        if socket.can_recv() {         // 有数据可读
            // 将数据从 smoltcp 缓冲区拷贝到用户缓冲区
            return total;
        }
        if !socket.may_recv() {        // 对端已关闭，不会再有数据
            return 0;                  // EOF
        }
        if self.nonblock {
            return usize::MAX;         // 编码为 -EAGAIN
        }
        drop(net);                     // 释放锁！
        suspend_current_and_run_next(); // 让出 CPU，等下次调度再试
    }
}
```

关键设计：
- **先 poll 再读**：确保协议栈已处理最新的入站包
- **阻塞语义**：通过 `suspend_current_and_run_next()` 让出 CPU。注意必须先 `drop(net)` 释放网络栈的锁，否则其他任务无法访问网络栈，造成死锁
- **EOF 检测**：`may_recv()` 返回 false 表示 TCP 连接的接收方向已关闭（对端发送了 FIN）
- **非阻塞模式**：`SOCK_NONBLOCK` 时不阻塞，返回 EAGAIN

**poll 操作**：

```rust
fn poll(&self, events: PollEvents) -> PollEvents {
    poll_net();
    let socket = /* 获取 smoltcp Socket */;
    let mut result = PollEvents::empty();

    if events.contains(PollEvents::POLLIN) && socket.can_recv() {
        result |= PollEvents::POLLIN;    // 可读
    }
    if events.contains(PollEvents::POLLOUT) && socket.can_send() {
        result |= PollEvents::POLLOUT;   // 可写
    }
    if !socket.is_open() {
        result |= PollEvents::POLLHUP;   // 连接已断开
    }
    result
}
```

这使得用户态的 `ppoll()` 系统调用可以同时监听多个 Socket 和文件的 I/O 事件。

### 5.4 Drop 与资源清理

```rust
impl Drop for SocketFile {
    fn drop(&mut self) {
        if self.transferred.load(Ordering::Relaxed) {
            return;  // 所有权已转移给 accept，不要清理
        }
        // 关闭 smoltcp Socket 并从 SocketSet 中移除
        match self.sock_type {
            SocketType::Tcp => socket.abort(),   // TCP: 发送 RST
            SocketType::Udp => socket.close(),   // UDP: 取消绑定
        }
        sockets.remove(self.handle);
    }
}
```

**`transferred` 标志的作用**：在 `accept()` 中，原来的 listen Socket 的句柄会被转移给新的已连接 Socket。此时旧的 `SocketFile` 被替换掉，触发 `Drop`。如果不加 `transferred` 标志，`Drop` 会错误地 abort 掉刚刚建立的连接。这是一个**实战中发现的 bug**——accept 成功后立即 panic (`handle does not refer to a valid socket`)。

---

## 六、第五层：系统调用实现

### 6.1 系统调用编号映射

rCore-Lab 使用 Linux RISC-V 标准的系统调用编号：

| 编号 | 名称 | 功能 | 实现状态 |
|------|------|------|---------|
| 198 | socket | 创建 Socket | 完整（TCP/UDP, SOCK_CLOEXEC, SOCK_NONBLOCK） |
| 199 | socketpair | 创建 Socket 对 | stub (返回 EOPNOTSUPP) |
| 200 | bind | 绑定本地地址 | 完整 |
| 201 | listen | 开始监听 | 完整 |
| 202 | accept | 接受连接 | 完整 |
| 203 | connect | 发起连接 | 完整（TCP/UDP，支持 loopback） |
| 204 | getsockname | 获取本地地址 | 完整 |
| 205 | getpeername | 获取对端地址 | 完整 |
| 206 | sendto | 发送数据 | 完整（TCP + UDP loopback） |
| 207 | recvfrom | 接收数据 | 完整 |
| 208 | setsockopt | 设置 Socket 选项 | stub（静默接受常用选项） |
| 209 | getsockopt | 获取 Socket 选项 | stub（SO_ERROR 返回 0） |
| 210 | shutdown | 关闭连接方向 | 完整 |
| 211 | sendmsg | 发送消息 | stub (返回 EOPNOTSUPP) |
| 212 | recvmsg | 接收消息 | stub (返回 EOPNOTSUPP) |
| 242 | accept4 | accept + flags | 复用 accept 实现 |

所有 syscall 实现位于 [os/src/net/syscall.rs](os/src/net/syscall.rs)（约 900 行），通过 [os/src/syscall/mod.rs](os/src/syscall/mod.rs) 中的 `match` 分发。

### 6.2 地址转换：sockaddr_in ↔ IpEndpoint

用户态使用 Linux 标准的 `struct sockaddr_in`（16 字节），smoltcp 使用 `IpEndpoint`。内核需要在两者之间转换：

```rust
#[repr(C)]
struct SockAddrIn {
    sin_family: u16,        // AF_INET = 2
    sin_port: u16,          // 网络字节序 (big-endian)
    sin_addr: u32,          // 网络字节序 (big-endian)
    sin_zero: [u8; 8],      // 填充
}
```

**关键注意事项**：
- 端口号是**大端序**（网络字节序），需要 `u16::from_be_bytes()` 转换
- IP 地址也是**大端序**，4 个字节按 `[a, b, c, d]` 排列
- 必须通过 `translated_byte_buffer()` 将用户态指针转换为内核可访问的物理地址——用户态指针不能直接在内核中解引用（虚拟地址空间不同）

### 6.3 深入分析：accept 的实现

`accept()` 是最复杂的网络 syscall，因为 smoltcp 没有内置的 accept 队列。在 Linux 内核中，listen Socket 维护一个 backlog 队列，新连接到达时入队，`accept()` 从队列取出。但 smoltcp 的 TCP Socket 是**一对一**的——当 SYN 到达时，listen Socket 本身变成 ESTABLISHED 状态。

rCore-Lab 的解决方案——**handle 交换**：

```
初始状态:
  fd_table[listen_fd] → SocketFile { handle: H1 }     H1: TCP LISTEN on port 80

SYN 到达后（poll 处理）:
  fd_table[listen_fd] → SocketFile { handle: H1 }     H1: TCP ESTABLISHED (remote: 1.2.3.4:5678)

accept() 执行:
  1. 检测到 H1 已 ESTABLISHED
  2. 创建新的 Socket H2，让 H2 在 port 80 上 listen
  3. 标记旧 SocketFile 为 transferred（防止 Drop 清理 H1）
  4. 替换: fd_table[listen_fd] → SocketFile { handle: H2 }   H2: TCP LISTEN on port 80
  5. 分配新 fd: fd_table[new_fd] → SocketFile { handle: H1 }  H1: TCP ESTABLISHED

最终状态:
  fd_table[listen_fd] → H2: LISTEN    (继续等待下一个连接)
  fd_table[new_fd]    → H1: ESTABLISHED (已建立的连接)
```

这个"swap"模式巧妙地在 smoltcp 单 Socket 模型上实现了 Linux accept 语义。

### 6.4 深入分析：UDP loopback 的 inject_recv

smoltcp 的 `Loopback` 设备工作在 `Medium::Ip` 层——它只处理 IP 包，没有以太网帧头。TCP loopback 可以正常工作（因为 TCP 通过 smoltcp 的 `lo_iface` 处理），但 **UDP loopback 存在问题**：smoltcp 的 UDP `sendto` 需要通过 Interface 发送，而 loopback Interface 虽然能接收自己发出的包，但需要同时 poll 才能完成收发。

为解决这个问题，rCore-Lab **修改了 smoltcp 的源码**，在 `vendor/smoltcp/src/socket/udp.rs` 中添加了 `inject_recv()` 方法：

```rust
pub fn inject_recv(&mut self, data: &[u8], meta: impl Into<UdpMetadata>) -> Result<(), ()> {
    let meta = meta.into();
    let buf = self.rx_buffer.enqueue(data.len(), meta).map_err(|_| ())?;
    buf.copy_from_slice(data);
    Ok(())
}
```

当 `sendto(127.0.0.1:port)` 时，内核不走协议栈，而是直接遍历 SocketSet 找到绑定在目标端口的 UDP Socket，调用 `inject_recv()` 将数据"注入"其接收缓冲区：

```rust
if is_loopback {
    for (_sh, sock) in stack.sockets.iter_mut() {
        if let Socket::Udp(ref mut udp_sock) = sock {
            if udp_sock.endpoint().port == target_port {
                udp_sock.inject_recv(&data, sender_meta).ok();
                break;
            }
        }
    }
}
```

这是一个**实用的工程妥协**：修改第三方库虽然不够优雅，但只增加了 6 行代码就解决了 UDP loopback 的兼容性问题。

### 6.5 setsockopt/getsockopt 的 stub 策略

很多用户态程序在创建 Socket 后会调用 `setsockopt()` 设置各种选项（`SO_REUSEADDR`、`TCP_NODELAY`、`SO_KEEPALIVE` 等）。smoltcp 不支持大部分 Socket 选项，但如果返回错误，程序可能会认为 Socket 创建失败并中止。

当前策略是**静默接受常用选项**（返回 0 = 成功），但不实际生效：

```rust
match (level, optname) {
    (SOL_SOCKET, SO_REUSEADDR) => 0,   // 假装设置成功
    (SOL_SOCKET, SO_KEEPALIVE) => 0,
    (IPPROTO_TCP, TCP_NODELAY) => 0,
    _ => 0,  // 未知选项也返回成功，避免破坏应用
}
```

这是一种常见的 **"fake it till you make it"** 策略，在 OS 移植早期很实用。

---

## 七、第六层：LoongArch64 的 stub 实现

### 7.1 为什么需要 stub

rCore-Lab 支持两种架构：RISC-V 64 和 LoongArch64。但 VirtIO-Net 驱动（以及 smoltcp 的 Device trait 实现）目前**仅适配了 RISC-V**。LoongArch64 构建时无法编译网络驱动。

为了让 LoongArch64 构建通过且不影响其他功能，使用 **条件编译** 在两个架构间切换：

```rust
// os/src/main.rs
#[cfg_attr(target_arch = "loongarch64", path = "net_stub.rs")]
pub mod net;
```

当编译目标是 LoongArch64 时，`net` 模块指向 [os/src/net_stub.rs](os/src/net_stub.rs)——一个 65 行的 stub 文件，所有网络 syscall 都返回 `-ENOSYS`（功能未实现）：

```rust
pub fn sys_socket(_domain: usize, _ty: usize, _protocol: usize) -> isize { -ENOSYS }
pub fn sys_bind(_fd: usize, _addr: *const u8, _len: usize) -> isize { -ENOSYS }
// ... 其他 syscall 同理
```

这样用户态程序调用网络 syscall 时会得到明确的"不支持"错误，而不是内核 panic。

---

## 八、QEMU 网络环境与完整数据流

### 8.1 QEMU user-mode networking

QEMU 启动命令中的网络配置：

```bash
-device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1
-netdev user,id=net
```

这创建了一个 **user-mode NAT 网络**：

```
┌──────────────────────────────┐     ┌────────────────┐
│          Guest OS             │     │    宿主机       │
│  10.0.2.15/24                 │     │   Internet     │
│    │                          │     │                │
│    ├─ 网关: 10.0.2.2          ├─NAT─┤                │
│    ├─ DNS:  10.0.2.3          │     │                │
│    └─ DHCP: 10.0.2.2         │     │                │
└──────────────────────────────┘     └────────────────┘
```

Guest 发出的包经过 QEMU 内部的 NAT 转换后，以宿主机身份访问外部网络。不需要 root 权限，也不需要创建 TAP 设备。

### 8.2 完整的数据收发流程

**用户态发送数据（以 TCP write 为例）**：

```
1. 用户态调用 write(fd, "Hello", 5)
2. ecall 陷入内核 → sys_write → SocketFile::write → tcp_write
3. tcp_write 调用 poll_net()
   → iface.poll() 处理入站包（如果有）
4. 从 smoltcp SocketSet 获取 tcp::Socket
5. socket.send_slice("Hello") → 数据写入 smoltcp 的 TX 环形缓冲区
6. 下一次 poll_net() 时：
   → smoltcp 构造 TCP 段（添加序列号、确认号、校验和）
   → 构造 IP 包头（src: 10.0.2.15, dst: 目标 IP）
   → 通过 ARP 缓存查找目标 MAC（或发送 ARP 请求）
   → 构造以太网帧
   → 调用 device.transmit() → VirtioTxToken::consume()
7. VirtioTxToken 将帧写入 VirtIO TX virtqueue
8. QEMU 从 TX queue 取出帧，通过 NAT 发送到网络
```

**用户态接收数据（以 TCP read 为例）**：

```
1. QEMU 收到网络包，写入 VirtIO RX virtqueue，触发 IRQ
2. PLIC 将 IRQ 路由到内核 → irq_handler → poll_net_if_available()
   → device.receive() → VirtioRxToken (包含以太网帧)
   → smoltcp 解析帧头 → IP 头 → TCP 头
   → 数据写入对应 tcp::Socket 的 RX 环形缓冲区
3. 用户态调用 read(fd, buf, len)
4. ecall 陷入内核 → sys_read → SocketFile::read → tcp_read
5. tcp_read 调用 poll_net()（再次处理可能的新包）
6. socket.can_recv() == true → socket.recv_slice(&mut buf)
7. 数据通过 translated_byte_buffer 拷贝到用户态缓冲区
8. 返回读取的字节数
```

### 8.3 PLIC 中断路由

```
VirtIO-Net 设备产生中断
  → PLIC (Platform-Level Interrupt Controller) 仲裁
  → PLIC 将 IRQ 2 路由到 hart 0 的 Supervisor 态
  → S 态外部中断处理 → irq_handler()
  → match VIRTIO_NET_IRQ => poll_net_if_available()
  → plic.complete() 完成中断处理
```

PLIC 初始化时（[os/src/boards/qemu.rs](os/src/boards/qemu.rs)），需要：
1. 设置 Supervisor 态阈值为 0（允许所有优先级的中断）
2. 启用 VIRTIO_NET_IRQ (IRQ 2) 的路由
3. 设置 IRQ 优先级为 1

---

## 九、关键设计决策总结

### 9.1 已做的正确决策

| 决策 | 原因 | 效果 |
|------|------|------|
| 使用 smoltcp 而非自研协议栈 | TCP 实现极其复杂，smoltcp 是成熟的 no_std Rust 库 | 约 2000 行胶水代码完成全部网络功能 |
| 回退到 smoltcp v0.11.0 | v0.12 需要 Rust 2024 edition，与工具链不兼容 | 零 API 适配问题 |
| 双接口模型（eth + loopback） | 很多测试程序通过 127.0.0.1 通信 | socket 测试全部通过 |
| UDP inject_recv | smoltcp loopback 不支持 UDP 直接投递 | 修改 6 行代码解决 UDP loopback |
| transferred 标志 | accept 的 handle 交换会触发错误的 Drop | 消除 accept 后的 panic |
| setsockopt stub | 返回错误会导致很多程序中止 | 用户态程序正常运行 |
| 动态 MMIO 设备扫描 | QEMU bus 编号可能变化 | 更健壮的设备发现 |

### 9.2 已知的局限与未来方向

| 局限 | 影响 | 可能的改进 |
|------|------|-----------|
| 仅支持 IPv4 | 无法处理 IPv6 地址 | 启用 `proto-ipv6` feature |
| 无 accept backlog 队列 | 并发连接请求可能丢失 | 维护一个连接队列 |
| setsockopt 全部 stub | Socket 选项不生效（如 TCP_NODELAY） | 逐个实现有影响的选项 |
| 阻塞 I/O 通过 busy-yield 实现 | CPU 利用率不佳 | 使用 Condvar/waker 精确唤醒 |
| 单核单网卡 | 不支持多核并发或多网卡 | 细粒度锁或 per-CPU 网络栈 |
| LoongArch64 无网络支持 | LA 构建只有 stub | 适配 LA 的 VirtIO 驱动 |
| 无 sendmsg/recvmsg | 不支持 scatter-gather I/O | 实现 msghdr 解析 |
| 无 AF_UNIX（Unix domain socket） | 不支持本地进程间通信 | 需要独立的 AF_UNIX 实现 |

---

## 十、附录

### 10.1 文件清单与职责

| 文件 | 行数 | 层次 | 职责 |
|------|------|------|------|
| `os/src/drivers/net/mod.rs` | 135 | 硬件驱动 | VirtIO-Net 设备适配 smoltcp Device trait |
| `os/src/net/mod.rs` | 141 | 全局网络栈 | 双接口 NetStack、初始化、轮询、端口分配 |
| `os/src/net/socket_file.rs` | 294 | Socket 抽象 | SocketFile 实现 File trait（read/write/poll/Drop） |
| `os/src/net/syscall.rs` | 893 | 系统调用 | 16 个网络 syscall 的完整实现 |
| `os/src/net_stub.rs` | 65 | LA stub | LoongArch64 的 -ENOSYS 占位实现 |
| `os/src/boards/qemu.rs` | 66 | 中断路由 | PLIC 配置 + VIRTIO_NET_IRQ 分发 |
| `os/src/fs/mod.rs` | - | File trait | as_socket/fd_flags/status_flags 等扩展方法 |
| `os/src/syscall/mod.rs` | - | 系统调用分发 | 16 个网络 syscall 的 match 路由 |
| `vendor/smoltcp/src/socket/udp.rs` | +6 | smoltcp 补丁 | inject_recv() loopback 注入方法 |

### 10.2 关键常量速查

| 常量 | 值 | 含义 |
|------|-----|------|
| `VIRTIO_MMIO_BASE` | `0x1000_1000` | VirtIO MMIO 起始地址 |
| `VIRTIO_MMIO_STRIDE` | `0x1000` | 每个 VirtIO 设备的地址间隔 |
| `VIRTIO_NET_IRQ` | 2 | VirtIO-Net 的 PLIC 中断号 |
| `ETH_FRAME_SIZE` | 1514 | 最大以太网帧大小（1500 MTU + 14 头） |
| `MAX_SOCKETS` | 64 | 最大并发 Socket 数 |
| `NEXT_PORT` 起始值 | 49152 | 临时端口分配起始值 |
| Guest IP | `10.0.2.15/24` | QEMU user-mode 默认 Guest IP |
| Gateway | `10.0.2.2` | QEMU user-mode 默认网关 |
| Loopback IP | `127.0.0.1/8` | 回环接口地址 |
| AF_INET | 2 | IPv4 地址族 |
| SOCK_STREAM | 1 | TCP 类型 |
| SOCK_DGRAM | 2 | UDP 类型 |
| SOCK_NONBLOCK | 0o4000 | 非阻塞标志 |
| SOCK_CLOEXEC | 0o2000000 | exec 时关闭标志 |

### 10.3 参考资料

- [smoltcp 官方文档](https://docs.rs/smoltcp/0.11.0/) — API 参考
- [smoltcp GitHub](https://github.com/smoltcp-rs/smoltcp) — 源码与示例
- [VirtIO 1.1 Specification, Section 5.1](https://docs.oasis-open.org/virtio/virtio/v1.1/virtio-v1.1.html) — VirtIO Network Device 规范
- [RFC 793: TCP](https://tools.ietf.org/html/rfc793) — TCP 协议规范
- [RFC 768: UDP](https://tools.ietf.org/html/rfc768) — UDP 协议规范
- [RFC 826: ARP](https://tools.ietf.org/html/rfc826) — ARP 协议规范
- [QEMU Networking](https://wiki.qemu.org/Documentation/Networking) — QEMU 网络配置指南
- [ArceOS net 模块](https://github.com/rcore-os/arceos) — 另一个 smoltcp 集成参考
- [rCore-Tutorial-Book-v3](https://rcore-os.github.io/rCore-Tutorial-Book-v3/) — rCore 教程
