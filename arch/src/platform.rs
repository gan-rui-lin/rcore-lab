//! Static platform description shared by architecture modules.

#![allow(missing_docs)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DmaMode {
    KernelPageTableTranslate,
    DirectMapWindow { uncached_base: usize },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceKind {
    Uart,
    Block,
    Net,
    InputKeyboard,
    InputMouse,
    Plic,
    PciEcam,
    PciBarWindow,
    Test,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceTransport {
    Mmio { base: usize, size: usize },
    Pci,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceDesc {
    pub kind: DeviceKind,
    pub transport: DeviceTransport,
    pub irq: Option<u32>,
}

impl DeviceDesc {
    pub fn mmio_base(&self) -> Option<usize> {
        match self.transport {
            DeviceTransport::Mmio { base, .. } => Some(base),
            _ => None,
        }
    }

    pub fn mmio_size(&self) -> Option<usize> {
        match self.transport {
            DeviceTransport::Mmio { size, .. } => Some(size),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformConfig {
    pub memory_end: usize,
    pub mmio_regions: &'static [(usize, usize)],
    pub devices: &'static [DeviceDesc],
    pub dma_mode: DmaMode,
}

impl PlatformConfig {
    pub fn device(&self, kind: DeviceKind) -> Option<&DeviceDesc> {
        self.devices.iter().find(|device| device.kind == kind)
    }

    pub fn devices_by_kind(&self, kind: DeviceKind) -> impl Iterator<Item = &DeviceDesc> + '_ {
        self.devices
            .iter()
            .filter(move |device| device.kind == kind)
    }

    pub fn dma_uncached_base(&self) -> Option<usize> {
        match self.dma_mode {
            DmaMode::DirectMapWindow { uncached_base } => Some(uncached_base),
            DmaMode::KernelPageTableTranslate => None,
        }
    }
}
