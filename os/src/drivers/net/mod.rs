//! VirtIO network device driver adapted for smoltcp.

use alloc::vec;
use alloc::vec::Vec;

use crate::drivers::bus::virtio::VirtioHal;
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use virtio_drivers::{DeviceType, VirtIOHeader, VirtIONet};

/// VirtIO MMIO region on QEMU virt machine.
const VIRTIO_MMIO_BASE: usize = 0x1000_1000;
const VIRTIO_MMIO_STRIDE: usize = 0x1000;
const VIRTIO_MMIO_SLOTS: usize = 8;

/// Maximum Ethernet frame size (MTU 1500 + header 14).
const ETH_FRAME_SIZE: usize = 1514;

/// VirtIO network device implementing smoltcp's Device trait.
pub struct VirtIONetDevice {
    inner: VirtIONet<'static, VirtioHal>,
    rx_buf: Vec<u8>,
}

impl VirtIONetDevice {
    fn find_virtio_net_header() -> &'static mut VirtIOHeader {
        for index in 0..VIRTIO_MMIO_SLOTS {
            let addr = VIRTIO_MMIO_BASE + index * VIRTIO_MMIO_STRIDE;
            let header = unsafe { &mut *(addr as *mut VirtIOHeader) };
            if !header.verify() {
                continue;
            }
            if header.device_type() == DeviceType::Network {
                log::info!("[net] found virtio-net mmio at {:#x}", addr);
                return header;
            }
        }
        panic!("virtio-net mmio device not found");
    }

    /// Create a new VirtIO network device.
    pub fn new() -> Self {
        let header = Self::find_virtio_net_header();
        let inner =
            VirtIONet::<VirtioHal>::new(header).expect("failed to create virtio-net device");
        Self {
            inner,
            rx_buf: vec![0u8; 2048],
        }
    }

    /// Get the MAC address from VirtIO config space.
    pub fn mac_address(&self) -> [u8; 6] {
        self.inner.mac()
    }
}

/// Receive token: holds the received frame data in an owned buffer.
pub struct VirtioRxToken {
    buf: Vec<u8>,
}

impl phy::RxToken for VirtioRxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.buf)
    }
}

/// Transmit token: allocates a buffer, lets smoltcp fill it, then sends.
pub struct VirtioTxToken<'a> {
    driver: &'a mut VirtIONet<'static, VirtioHal>,
}

impl<'a> phy::TxToken for VirtioTxToken<'a> {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = vec![0u8; len];
        let result = f(&mut buf);
        if let Err(e) = self.driver.send(&buf) {
            log::warn!("[net] virtio-net send failed: {:?}", e);
        }
        result
    }
}

impl Device for VirtIONetDevice {
    type RxToken<'a> = VirtioRxToken where Self: 'a;
    type TxToken<'a> = VirtioTxToken<'a> where Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        if !self.inner.can_recv() {
            return None;
        }
        match self.inner.recv(&mut self.rx_buf) {
            Ok(len) => {
                // Copy received data into an owned buffer for the RxToken
                let data = self.rx_buf[..len].to_vec();
                Some((
                    VirtioRxToken { buf: data },
                    VirtioTxToken {
                        driver: &mut self.inner,
                    },
                ))
            }
            Err(e) => {
                log::warn!("[net] virtio-net recv failed: {:?}", e);
                None
            }
        }
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if self.inner.can_send() {
            Some(VirtioTxToken {
                driver: &mut self.inner,
            })
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = ETH_FRAME_SIZE;
        caps.max_burst_size = Some(1);
        caps
    }
}
