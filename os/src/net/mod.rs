//! Network subsystem: global network stack based on smoltcp.

pub mod socket_file;
pub mod syscall;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, Ordering};
use lazy_static::lazy_static;
use log::info;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::phy::{Loopback, Medium};
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, Ipv4Address};

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
    /// The VirtIO network device driver.
    pub device: VirtIONetDevice,
    /// The smoltcp network interface (external network).
    pub iface: Interface,
    /// Loopback device for 127.0.0.1 traffic.
    pub lo_device: Loopback,
    /// Loopback interface.
    pub lo_iface: Interface,
    /// The set of active sockets (shared across both interfaces).
    pub sockets: SocketSet<'static>,
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
    let mut device = VirtIONetDevice::new();
    let mac = device.mac_address();
    info!(
        "[net] VirtIO-Net MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    );

    let hw_addr = HardwareAddress::Ethernet(EthernetAddress(mac));
    let mut config = Config::new(hw_addr);
    config.random_seed = get_time_ms() as u64;

    let now = smoltcp_now();
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

    // Create loopback interface for 127.0.0.1
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
        device,
        iface,
        lo_device,
        lo_iface,
        sockets,
    });

    info!("[net] Network stack initialized: 10.0.2.15/24, gateway 10.0.2.2, loopback 127.0.0.1/8");
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
        stack.iface.poll(now, &mut stack.device, &mut stack.sockets);
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
            stack.iface.poll(now, &mut stack.device, &mut stack.sockets);
            stack
                .lo_iface
                .poll(now, &mut stack.lo_device, &mut stack.sockets);
        }
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
