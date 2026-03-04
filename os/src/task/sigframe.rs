#![allow(missing_docs)]

//! Signal frame structures and setup/restore functions.
//!
//! When delivering a signal with a user handler, the kernel pushes a
//! "signal frame" (rt_sigframe) onto the user stack. This frame contains
//! the saved register state and signal mask, matching the Linux ABI so
//! that musl libc's sigreturn works correctly.

use alloc::vec;
use crate::mm::translated_byte_buffer;
use crate::trap::TrapContext;
use super::signal::SigSet;
use super::action::{SignalAction, SA_SIGINFO, SA_RESTORER};

// ============================================================
// Shared structures (same layout on all architectures)
// ============================================================

/// siginfo_t (128 bytes, matching Linux ABI)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SigInfo {
    pub si_signo: i32,
    pub si_errno: i32,
    pub si_code: i32,
    _pad: [i32; 29], // pad to 128 bytes total
}

/// stack_t (24 bytes on LP64)
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StackT {
    pub ss_sp: usize,
    pub ss_flags: i32,
    _pad: i32,
    pub ss_size: usize,
}

// ============================================================
// RISC-V 64 signal frame
// ============================================================

#[cfg(target_arch = "riscv64")]
mod riscv64_sigframe {
    use super::*;

    /// Saved GP registers: pc first, then x1..x31.
    /// Matches Linux `struct user_regs_struct` for riscv64.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct UserRegs {
        pub pc: usize,           // sepc
        pub regs: [usize; 31],  // x1 through x31
    }

    /// FP state placeholder (528 bytes = musl's __fpregs[66]).
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct FpState {
        pub fpregs: [u64; 66],
    }

    /// sigcontext = GP regs + FP state
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct SigContext {
        pub sc_regs: UserRegs,    // 256 bytes
        pub sc_fpregs: FpState,   // 528 bytes
    }

    /// ucontext_t
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct UContext {
        pub uc_flags: usize,          // 8
        pub uc_link: usize,           // 8
        pub uc_stack: StackT,         // 24
        pub uc_sigmask: u64,          // 8
        pub __unused: [u8; 120],      // padding (1024/8 - sizeof(sigset_t))
        pub uc_mcontext: SigContext,  // 784
    }

    /// rt_sigframe — complete signal frame on user stack
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct RtSigframe {
        pub info: SigInfo,   // 128 bytes
        pub uc: UContext,     // 952 bytes
    }

    /// Write a byte slice to user virtual memory via page table token.
    fn write_to_user_mem(token: usize, dst_va: usize, data: &[u8]) {
        let bufs = translated_byte_buffer(token, dst_va as *const u8, data.len());
        let mut offset = 0;
        for buf in bufs {
            let len = buf.len().min(data.len() - offset);
            buf[..len].copy_from_slice(&data[offset..offset + len]);
            offset += len;
        }
    }

    /// Read a byte slice from user virtual memory via page table token.
    fn read_from_user_mem(token: usize, src_va: usize, dst: &mut [u8]) {
        let bufs = translated_byte_buffer(token, src_va as *const u8, dst.len());
        let mut offset = 0;
        for buf in bufs {
            let len = buf.len().min(dst.len() - offset);
            dst[offset..offset + len].copy_from_slice(&buf[..len]);
            offset += len;
        }
    }

    /// Push a signal frame onto the user stack and set up the trap context
    /// for handler entry.
    ///
    /// Returns the new sp (frame address) on success.
    pub fn setup_signal_frame(
        token: usize,
        trap_cx: &mut TrapContext,
        signum: usize,
        action: &SignalAction,
        old_sigmask: SigSet,
    ) -> Option<usize> {
        let old_sp = trap_cx.x[2];
        let frame_size = core::mem::size_of::<RtSigframe>();

        // 16-byte aligned new sp
        let new_sp = (old_sp.wrapping_sub(frame_size)) & !0xF;

        // Sanity check: new_sp should be a reasonable user address
        if new_sp == 0 || new_sp >= old_sp {
            return None;
        }

        // Save current GP registers into the frame
        let mut regs = [0usize; 31];
        for i in 0..31 {
            regs[i] = trap_cx.x[i + 1];
        }

        let frame = RtSigframe {
            info: SigInfo {
                si_signo: signum as i32,
                si_errno: 0,
                si_code: 0,
                _pad: [0; 29],
            },
            uc: UContext {
                uc_flags: 0,
                uc_link: 0,
                uc_stack: StackT {
                    ss_sp: 0,
                    ss_flags: 2, // SS_DISABLE
                    _pad: 0,
                    ss_size: 0,
                },
                uc_sigmask: old_sigmask.raw(),
                __unused: [0; 120],
                uc_mcontext: SigContext {
                    sc_regs: UserRegs { pc: trap_cx.sepc, regs },
                    sc_fpregs: FpState { fpregs: [0u64; 66] },
                },
            },
        };

        // Write frame to user stack
        let frame_bytes = unsafe {
            core::slice::from_raw_parts(
                &frame as *const RtSigframe as *const u8,
                frame_size,
            )
        };
        write_to_user_mem(token, new_sp, frame_bytes);

        // Determine restorer address (ra for when handler returns)
        let restorer = if action.flags & SA_RESTORER != 0 {
            action.restorer
        } else {
            warn!("[signal] signum={}: no SA_RESTORER, handler return will fault", signum);
            0
        };

        // Set up trap context for handler entry
        trap_cx.sepc = action.handler;   // entry point
        trap_cx.x[1] = restorer;         // ra → restorer (calls sigreturn)
        trap_cx.x[2] = new_sp;           // sp → frame
        trap_cx.x[10] = signum;          // a0 = signal number

        if action.flags & SA_SIGINFO != 0 {
            // a1 = &siginfo, a2 = &ucontext
            trap_cx.x[11] = new_sp;                                     // &frame.info
            trap_cx.x[12] = new_sp + core::mem::size_of::<SigInfo>();   // &frame.uc
        }

        Some(new_sp)
    }

    /// Restore the trap context and signal mask from the signal frame on
    /// the user stack. Called from sys_sigreturn.
    ///
    /// Returns (original_a0, restored_sigmask) on success.
    pub fn restore_signal_frame(
        token: usize,
        trap_cx: &mut TrapContext,
    ) -> Option<(usize, SigSet)> {
        let frame_addr = trap_cx.x[2]; // sp = frame address
        let frame_size = core::mem::size_of::<RtSigframe>();

        // Read frame from user stack
        let mut data = vec![0u8; frame_size];
        read_from_user_mem(token, frame_addr, &mut data);

        let frame = unsafe {
            core::ptr::read_unaligned(data.as_ptr() as *const RtSigframe)
        };

        // Restore GP registers
        trap_cx.sepc = frame.uc.uc_mcontext.sc_regs.pc;
        for i in 0..31 {
            trap_cx.x[i + 1] = frame.uc.uc_mcontext.sc_regs.regs[i];
        }
        // Note: FP state restore not implemented yet

        let mask = SigSet::from_raw(frame.uc.uc_sigmask);
        let original_a0 = trap_cx.x[10]; // a0 was just restored from frame

        Some((original_a0, mask))
    }
}

#[cfg(target_arch = "riscv64")]
pub use riscv64_sigframe::*;

// ============================================================
// LoongArch64 stubs (full implementation in Phase 7)
// ============================================================

#[cfg(target_arch = "loongarch64")]
pub fn setup_signal_frame(
    _token: usize,
    trap_cx: &mut TrapContext,
    signum: usize,
    action: &SignalAction,
    _old_sigmask: SigSet,
) -> Option<usize> {
    // Temporary: redirect era and set a0 (no stack frame)
    // LoongArch64 register mapping: a0=$r4, ra=$r1, sp=$r3
    trap_cx.era = action.handler;
    trap_cx.x[4] = signum;  // a0 = $r4
    if action.flags & SA_RESTORER != 0 {
        trap_cx.x[1] = action.restorer; // ra = $r1
    }
    Some(trap_cx.x[3]) // sp = $r3
}

#[cfg(target_arch = "loongarch64")]
pub fn restore_signal_frame(
    _token: usize,
    _trap_cx: &mut TrapContext,
) -> Option<(usize, SigSet)> {
    None // LoongArch64: fall back to signal_trap_cx restore
}
