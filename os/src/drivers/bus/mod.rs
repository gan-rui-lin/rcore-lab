/// VirtIO bus support and DMA helpers.
#[cfg(target_arch = "riscv64")]
#[path = "virtio_rv.rs"]
pub mod virtio;

/// VirtIO bus support and DMA helpers for LoongArch64.
#[cfg(target_arch = "loongarch64")]
#[path = "virtio_la.rs"]
pub mod virtio;
