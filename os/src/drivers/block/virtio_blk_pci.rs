//! VirtIO block device driver over PCI transport for LoongArch64.
//!
//! This driver uses `virtio-drivers` v0.7.1 with `PciTransport`.
//! It implements simple **polling-only** I/O (no interrupt-driven non-blocking path).

use super::BlockDevice;
use crate::drivers::bus::virtio::VirtioHal;
use arch::DeviceKind;
use spin::Mutex;
use virtio_drivers_new::device::blk::VirtIOBlk;
use virtio_drivers_new::transport::pci::bus::{BarInfo, Cam, Command, MemoryBarType, PciRoot};
use virtio_drivers_new::transport::pci::{virtio_device_type, PciTransport};
use virtio_drivers_new::transport::DeviceType;

struct PciRangeAllocator {
    end: usize,
    current: usize,
}

impl PciRangeAllocator {
    const fn new(base: usize, size: usize) -> Self {
        Self {
            end: base + size,
            current: base,
        }
    }

    fn alloc(&mut self, size: usize) -> Option<usize> {
        if !size.is_power_of_two() {
            return None;
        }
        let addr = align_up(self.current, size);
        if addr + size > self.end {
            return None;
        }
        self.current = addr + size;
        Some(addr)
    }
}

const fn align_up(addr: usize, align: usize) -> usize {
    (addr + align - 1) & !(align - 1)
}

/// VirtIO block device over PCI transport (polling-only).
pub struct VirtIOPCIBlock {
    blk: Mutex<VirtIOBlk<VirtioHal, PciTransport>>,
}

unsafe impl Send for VirtIOPCIBlock {}
unsafe impl Sync for VirtIOPCIBlock {}

impl VirtIOPCIBlock {
    /// Scan the PCI bus for a VirtIO block device and create the driver.
    pub fn new() -> Self {
        let config = arch::platform_config();
        let ecam_base = config
            .device(DeviceKind::PciEcam)
            .and_then(|device| device.mmio_base())
            .expect("PCI ECAM base missing from platform config");
        let bar_window = config
            .device(DeviceKind::PciBarWindow)
            .expect("PCI BAR window missing from platform config");
        let bar_base = bar_window
            .mmio_base()
            .expect("PCI BAR window base missing from platform config");
        let bar_size = bar_window
            .mmio_size()
            .expect("PCI BAR window size missing from platform config");
        let dmw_base = config
            .dma_uncached_base()
            .expect("LoongArch PCI requires DirectMapWindow dma mode");

        // Create a PciRoot pointing at the ECAM MMIO region via DMW.
        let ecam_vaddr = (ecam_base | dmw_base) as *mut u8;
        let mut pci_root = unsafe { PciRoot::new(ecam_vaddr, Cam::Ecam) };

        // Enumerate bus 0 looking for a VirtIO block device.
        let mut blk_dev_fn = None;
        for (dev_fn, info) in pci_root.enumerate_bus(0) {
            if let Some(DeviceType::Block) = virtio_device_type(&info) {
                info!("[pci] Found VirtIO block device at {} : {}", dev_fn, info);
                blk_dev_fn = Some(dev_fn);
                break;
            }
        }

        let dev_fn = blk_dev_fn.expect("No VirtIO block device found on PCI bus 0");

        // Allocate BARs for the device.
        let mut allocator = PciRangeAllocator::new(bar_base, bar_size);
        let mut bar_index = 0u8;
        while bar_index < 6 {
            let bar_info = pci_root.bar_info(dev_fn, bar_index).expect("pci bar_info");
            if let BarInfo::Memory {
                address,
                size,
                address_type,
                ..
            } = bar_info
            {
                if address == 0 && size != 0 {
                    let addr = allocator.alloc(size as usize).expect("pci bar alloc");
                    match address_type {
                        MemoryBarType::Width64 => {
                            pci_root.set_bar_64(dev_fn, bar_index, addr as u64)
                        }
                        MemoryBarType::Width32 => {
                            pci_root.set_bar_32(dev_fn, bar_index, addr as u32)
                        }
                        MemoryBarType::Below1MiB => {}
                    }
                }
            }
            bar_index += 1;
            if bar_info.takes_two_entries() {
                bar_index += 1;
            }
        }

        // Enable bus-mastering and memory space for this device.
        let (_, _) = pci_root.get_status_command(dev_fn);
        pci_root.set_command(
            dev_fn,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );

        // Create the PCI transport.
        let transport =
            PciTransport::new::<VirtioHal>(&mut pci_root, dev_fn).expect("PciTransport::new");

        // Create the block device driver.
        let blk =
            VirtIOBlk::<VirtioHal, PciTransport>::new(transport).expect("VirtIOBlk::new over PCI");

        info!(
            "[pci] VirtIO block device ready: capacity = {} sectors",
            blk.capacity()
        );

        Self {
            blk: Mutex::new(blk),
        }
    }
}

impl BlockDevice for VirtIOPCIBlock {
    fn read_block(&self, block_id: usize, buf: &mut [u8]) {
        self.blk
            .lock()
            .read_blocks(block_id, buf)
            .expect("VirtIO PCI blk read failed");
    }

    fn write_block(&self, block_id: usize, buf: &[u8]) {
        self.blk
            .lock()
            .write_blocks(block_id, buf)
            .expect("VirtIO PCI blk write failed");
    }

    fn handle_irq(&self) {
        // Polling-only driver -- nothing to do on IRQ for now.
    }
}
