#![allow(missing_docs)]
use crate::task::{SignalFlags, MAX_SIG};

/// Action for a signal
///
/// On RISC-V (and most arches), the layout includes `sa_restorer`.
/// On LoongArch, Linux does NOT define SA_RESTORER, so the kernel
/// struct has no `restorer` field — mask follows flags directly.
#[repr(C, align(16))]
#[derive(Debug, Clone, Copy)]
pub struct SignalAction {
    pub handler: usize,
    pub flags: usize,
    #[cfg(not(target_arch = "loongarch64"))]
    pub restorer: usize,
    pub mask: SignalFlags,
}

impl SignalAction {
    /// Returns the restorer address. Always 0 on LoongArch (no SA_RESTORER).
    pub fn restorer(&self) -> usize {
        #[cfg(not(target_arch = "loongarch64"))]
        {
            self.restorer
        }
        #[cfg(target_arch = "loongarch64")]
        {
            0
        }
    }
}

impl Default for SignalAction {
    fn default() -> Self {
        Self {
            handler: 0,
            flags: 0,
            #[cfg(not(target_arch = "loongarch64"))]
            restorer: 0,
            mask: SignalFlags::empty(),
        }
    }
}

pub const SA_RESTART: usize = 0x10000000;
pub const SA_SIGINFO: usize = 4;
pub const SA_RESTORER: usize = 0x04000000;
pub const SA_RESETHAND: usize = 0x80000000;

#[derive(Clone)]
pub struct SignalActions {
    pub table: [SignalAction; MAX_SIG + 1],
}

impl Default for SignalActions {
    fn default() -> Self {
        Self {
            table: [SignalAction::default(); MAX_SIG + 1],
        }
    }
}
