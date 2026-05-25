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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlatformConfig {
    pub memory_end: usize,
    pub mmio_regions: &'static [(usize, usize)],
    pub devices: &'static [DeviceDesc],
    pub dma_mode: DmaMode,
}
