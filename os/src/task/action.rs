#![allow(missing_docs)]
use crate::task::signal::{SigSet, MAX_SIG};

// Signal handler special values
pub const SIG_DFL: usize = 0;
pub const SIG_IGN: usize = 1;

// sa_flags constants (Linux ABI)
pub const SA_NOCLDSTOP: usize = 1;
pub const SA_NOCLDWAIT: usize = 2;
pub const SA_SIGINFO: usize = 4;
pub const SA_ONSTACK: usize = 0x0800_0000;
pub const SA_RESTART: usize = 0x1000_0000;
pub const SA_NODEFER: usize = 0x4000_0000;
pub const SA_RESETHAND: usize = 0x8000_0000;
pub const SA_RESTORER: usize = 0x0400_0000;

/// Kernel-internal signal action.
#[derive(Debug, Clone, Copy)]
pub struct SignalAction {
    pub handler: usize,   // SIG_DFL(0), SIG_IGN(1), or handler address
    pub flags: usize,     // sa_flags
    pub restorer: usize,  // sa_restorer (user-provided sigreturn trampoline)
    pub mask: SigSet,     // sa_mask (signals blocked during handler)
}

impl Default for SignalAction {
    fn default() -> Self {
        Self {
            handler: SIG_DFL,
            flags: 0,
            restorer: 0,
            mask: SigSet::empty(),
        }
    }
}

/// Linux user-space rt_sigaction layout (riscv64 / LP64).
/// This is what musl sends via the rt_sigaction syscall.
/// Layout: { sa_handler, sa_flags, sa_restorer, sa_mask }
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KSigAction {
    pub sa_handler: usize,
    pub sa_flags: usize,
    pub sa_restorer: usize,
    pub sa_mask: u64,
}

impl KSigAction {
    /// Convert from user-space ABI to kernel-internal representation.
    pub fn to_signal_action(&self) -> SignalAction {
        SignalAction {
            handler: self.sa_handler,
            flags: self.sa_flags,
            restorer: self.sa_restorer,
            mask: SigSet::from_raw(self.sa_mask),
        }
    }

    /// Convert from kernel-internal representation to user-space ABI.
    pub fn from_signal_action(action: &SignalAction) -> Self {
        Self {
            sa_handler: action.handler,
            sa_flags: action.flags,
            sa_restorer: action.restorer,
            sa_mask: action.mask.raw(),
        }
    }
}

/// Table of signal actions for a process (1-indexed, index 0 unused).
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
