//! Network subsystem: global network stack based on smoltcp.

pub mod socket_file;
pub mod syscall;
pub mod unix_socket;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};
use lazy_static::lazy_static;
use log::info;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::phy::{Loopback, Medium};
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};

#[cfg(target_arch = "riscv64")]
use smoltcp::wire::{EthernetAddress, Ipv4Address};

#[cfg(target_arch = "riscv64")]
use crate::drivers::net::VirtIONetDevice;
use crate::sync::UPIntrFreeCell;
use crate::timer::get_time_ms;

pub use socket_file::{SocketFile, SocketType};

/// Maximum number of concurrent sockets.
const MAX_SOCKETS: usize = 64;

/// Ephemeral port counter (49152-65535).
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

/// The global network stack holding device, interface, and sockets.
pub struct NetStack {
    /// The VirtIO network device driver (RISC-V only).
    #[cfg(target_arch = "riscv64")]
    pub device: VirtIONetDevice,
    /// The smoltcp network interface for external network (RISC-V only).
    #[cfg(target_arch = "riscv64")]
    pub iface: Interface,
    /// Loopback device for 127.0.0.1 traffic.
    pub lo_device: Loopback,
    /// Loopback interface.
    pub lo_iface: Interface,
    /// The set of active sockets (shared across both interfaces).
    pub sockets: SocketSet<'static>,
}

impl NetStack {
    /// Poll the external network interface. No-op on architectures without
    /// a VirtIO network device (e.g. LoongArch64 loopback-only mode).
    pub fn poll_external(&mut self, now: smoltcp::time::Instant) {
        #[cfg(target_arch = "riscv64")]
        {
            self.iface.poll(now, &mut self.device, &mut self.sockets);
        }
        let _ = now; // suppress unused warning on non-riscv64
    }


}

lazy_static! {
    /// Global network stack, protected by interrupt-free cell.
    pub static ref NET_STACK: UPIntrFreeCell<Option<NetStack>> =
        unsafe { UPIntrFreeCell::new(None) };
}

/// Get current time as smoltcp Instant.
pub fn smoltcp_now() -> smoltcp::time::Instant {
    smoltcp::time::Instant::from_millis(get_time_ms() as i64)
}

/// Initialize the network stack.
pub fn init() {
    let now = smoltcp_now();

    // --- External network interface (RISC-V only) ---
    #[cfg(target_arch = "riscv64")]
    let (device, iface) = {
        let mut device = VirtIONetDevice::new();
        let mac = device.mac_address();
        info!(
            "[net] VirtIO-Net MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        );

        let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
        let mut config = Config::new(hw_addr);
        config.random_seed = get_time_ms() as u64;

        let mut iface = Interface::new(config, &mut device, now);

        // Configure IP: 10.0.2.15/24 (QEMU user-mode default)
        iface.update_ip_addrs(|addrs| {
            addrs
                .push(IpCidr::new(IpAddress::v4(10, 0, 2, 15), 24))
                .unwrap();
        });

        // Default gateway: 10.0.2.2 (QEMU user-mode default)
        iface
            .routes_mut()
            .add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2))
            .unwrap();

        (device, iface)
    };

    // --- Loopback interface (all architectures) ---
    let mut lo_device = Loopback::new(Medium::Ip);
    let lo_config = Config::new(HardwareAddress::Ip);
    let mut lo_iface = Interface::new(lo_config, &mut lo_device, now);
    lo_iface.update_ip_addrs(|addrs| {
        addrs
            .push(IpCidr::new(IpAddress::v4(127, 0, 0, 1), 8))
            .unwrap();
    });

    let socket_storage: Vec<SocketStorage<'static>> =
        (0..MAX_SOCKETS).map(|_| SocketStorage::EMPTY).collect();
    let sockets = SocketSet::new(socket_storage);

    *NET_STACK.exclusive_access() = Some(NetStack {
        #[cfg(target_arch = "riscv64")]
        device,
        #[cfg(target_arch = "riscv64")]
        iface,
        lo_device,
        lo_iface,
        sockets,
    });

    #[cfg(target_arch = "riscv64")]
    info!("[net] Network stack initialized: 10.0.2.15/24, gateway 10.0.2.2, loopback 127.0.0.1/8");
    #[cfg(not(target_arch = "riscv64"))]
    info!("[net] Network stack initialized: loopback 127.0.0.1/8 (loopback-only mode)");
}

/// Poll the network stack (always acquires lock). Use from syscall path.
///
/// Loopback needs multiple polls to complete a round-trip:
/// TX→loopback_rx→process→TX→loopback_rx. We do 4 rounds which is
/// enough for TCP 3-way handshake (SYN→SYN-ACK→ACK→data).
pub fn poll_net() {
    let mut net = NET_STACK.exclusive_access();
    if let Some(ref mut stack) = *net {
        let now = smoltcp_now();
        stack.poll_external(now);
        for _ in 0..4 {
            stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
        }
    }
}

/// Try to poll the network stack (non-blocking). Use from timer interrupt.
pub fn poll_net_if_available() {
    if let Some(mut net) = NET_STACK.try_exclusive_access() {
        if let Some(ref mut stack) = *net {
            let now = smoltcp_now();
            stack.poll_external(now);
            for _ in 0..4 {
                stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
            }
        }
    }
}

/// Force-poll network stack (blocking). Called from task scheduling context.
pub fn poll_net_force() {
    let mut net = NET_STACK.exclusive_access();
    if let Some(ref mut stack) = *net {
        let now = smoltcp_now();
        stack.poll_external(now);
        for _ in 0..4 {
            stack.lo_iface.poll(now, &mut stack.lo_device, &mut stack.sockets);
        }
    }
}

/// Loopback UDP inject with demux: deliver a packet to the best-matching
/// UDP socket on `target_port`, skipping the sender socket (`sender_handle`).
///
/// Matching priority:
///   1. Connected socket whose `remote_endpoint` matches the sender
///   2. Unconnected (wildcard) socket on the same port
///
/// This allows iperf3's parallel UDP streams to each receive only their
/// own traffic, while the server's unconnected listener still receives
/// new stream-setup cookies.
pub fn loopback_udp_inject(
    stack: &mut NetStack,
    sender_handle: smoltcp::iface::SocketHandle,
    target_port: u16,
    data: &[u8],
    sender_meta: smoltcp::socket::udp::UdpMetadata,
) {
    use smoltcp::socket::udp::Socket as UdpSocket;

    let sender_ep = sender_meta.endpoint; // (127.0.0.1, sender_port)

    // Two-pass: first look for a connected socket matching the sender,
    // then fall back to any unconnected socket on the same port.
    let mut connected_handle: Option<smoltcp::iface::SocketHandle> = None;
    let mut wildcard_handle: Option<smoltcp::iface::SocketHandle> = None;

    for (sh, sock) in stack.sockets.iter() {
        if let smoltcp::socket::Socket::Udp(ref udp_sock) = sock {
            if udp_sock.endpoint().port != target_port {
                continue;
            }
            // Don't deliver to the sender itself
            if sh == sender_handle {
                continue;
            }
            if let Some(remote) = udp_sock.remote_endpoint() {
                // Connected socket: must match sender's addr+port
                if remote.addr == sender_ep.addr && remote.port == sender_ep.port {
                    connected_handle = Some(sh);
                    break; // exact match, no need to keep looking
                }
            } else {
                // Unconnected (wildcard) listener
                if wildcard_handle.is_none() {
                    wildcard_handle = Some(sh);
                }
            }
        }
    }

    let target = connected_handle.or(wildcard_handle);
    if let Some(handle) = target {
        let udp_sock = stack.sockets.get_mut::<UdpSocket>(handle);
        let _ = udp_sock.inject_recv(data, sender_meta);
    }
}

/// Allocate an ephemeral port (49152-65535).
pub fn alloc_ephemeral_port() -> u16 {
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if port >= 49152 {
            return port;
        }
        // Wrap around
        NEXT_PORT.store(49152, Ordering::Relaxed);
    }
}
