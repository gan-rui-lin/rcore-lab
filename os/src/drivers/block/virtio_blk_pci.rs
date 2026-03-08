//! VirtIO block device driver over PCI transport for LoongArch64.
//!
//! This driver uses `virtio-drivers` v0.7.1 with `PciTransport`.
//! It implements simple **polling-only** I/O (no interrupt-driven non-blocking path).

use super::BlockDevice;
use crate::drivers::bus::virtio::VirtioHal;
use spin::Mutex;
use virtio_drivers_new::device::blk::VirtIOBlk;
use virtio_drivers_new::transport::pci::bus::{Cam, PciRoot};
use virtio_drivers_new::transport::pci::{virtio_device_type, PciTransport};
use virtio_drivers_new::transport::DeviceType;

/// LoongArch64 QEMU virt PCI ECAM base (physical address).
const PCI_ECAM_BASE: usize = 0x2000_0000;
/// LoongArch64 DMW uncached window base.
const DMW_BASE: usize = 0x8000_0000_0000_0000;

/// VirtIO block device over PCI transport (polling-only).
pub struct VirtIOPCIBlock {
    blk: Mutex<VirtIOBlk<VirtioHal, PciTransport>>,
}

unsafe impl Send for VirtIOPCIBlock {}
unsafe impl Sync for VirtIOPCIBlock {}

impl VirtIOPCIBlock {
    /// Scan the PCI bus for a VirtIO block device and create the driver.
    pub fn new() -> Self {
        // Create a PciRoot pointing at the ECAM MMIO region via DMW.
        let ecam_vaddr = (PCI_ECAM_BASE | DMW_BASE) as *mut u8;
        let mut pci_root = unsafe { PciRoot::new(ecam_vaddr, Cam::Ecam) };

        // Enumerate bus 0 looking for a VirtIO block device.
        let mut blk_dev_fn = None;
        for (dev_fn, info) in pci_root.enumerate_bus(0) {
            if let Some(DeviceType::Block) = virtio_device_type(&info) {
                info!(
                    "[pci] Found VirtIO block device at {} : {}",
                    dev_fn, info
                );
                blk_dev_fn = Some(dev_fn);
                break;
            }
        }

        let dev_fn = blk_dev_fn.expect("No VirtIO block device found on PCI bus 0");

        // Enable bus-mastering and memory space for this device.
        use virtio_drivers_new::transport::pci::bus::Command;
        let (_, _) = pci_root.get_status_command(dev_fn);
        pci_root.set_command(
            dev_fn,
            Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
        );

        // Create the PCI transport.
        let transport =
            PciTransport::new::<VirtioHal>(&mut pci_root, dev_fn).expect("PciTransport::new");

        // Create the block device driver.
        let blk = VirtIOBlk::<VirtioHal, PciTransport>::new(transport)
            .expect("VirtIOBlk::new over PCI");

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
