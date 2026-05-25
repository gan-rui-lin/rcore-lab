//! VirtIO network device driver adapted for smoltcp.

use alloc::vec;
use alloc::vec::Vec;

use crate::drivers::bus::virtio::VirtioHal;
use arch::DeviceKind;
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::time::Instant;
use virtio_drivers::{DeviceType, VirtIOHeader, VirtIONet};

/// Maximum Ethernet frame size (MTU 1500 + header 14).
const ETH_FRAME_SIZE: usize = 1514;

/// VirtIO network device implementing smoltcp's Device trait.
pub struct VirtIONetDevice {
    inner: VirtIONet<'static, VirtioHal>,
    rx_buf: Vec<u8>,
}

impl VirtIONetDevice {
    fn header_at(addr: usize) -> Option<&'static mut VirtIOHeader> {
        let header = unsafe { &mut *(addr as *mut VirtIOHeader) };
        if header.verify() && header.device_type() == DeviceType::Network {
            Some(header)
        } else {
            None
        }
    }

    fn find_virtio_net_header() -> Option<&'static mut VirtIOHeader> {
        let addr = arch::platform_config()
            .device(DeviceKind::Net)
            .and_then(|device| device.mmio_base())
            .expect("VirtIO net MMIO base missing from platform config");
        if let Some(header) = Self::header_at(addr) {
            log::info!("[net] found virtio-net mmio at {:#x}", addr);
            return Some(header);
        }

        for device in arch::platform_config().devices {
            match device.kind {
                DeviceKind::Block
                | DeviceKind::Net
                | DeviceKind::InputKeyboard
                | DeviceKind::InputMouse => {}
                _ => continue,
            }
            if let Some(probe) = device.mmio_base() {
                if probe != addr {
                    if let Some(header) = Self::header_at(probe) {
                        log::info!("[net] found virtio-net mmio at {:#x}", probe);
                        return Some(header);
                    }
                }
            }
        }

        log::warn!("[net] virtio-net mmio device not found");
        None
    }

    /// Create a new VirtIO network device.
    pub fn try_new() -> Option<Self> {
        let header = Self::find_virtio_net_header()?;
        let inner = match VirtIONet::<VirtioHal>::new(header) {
            Ok(inner) => inner,
            Err(err) => {
                log::warn!("[net] failed to create virtio-net device: {:?}", err);
                return None;
            }
        };
        Some(Self {
            inner,
            rx_buf: vec![0u8; 2048],
        })
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
