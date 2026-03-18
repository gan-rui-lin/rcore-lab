//! LoongArch64 trap handling -- arch layer.
//!
//! This module contains:
//!   - All inline / global assembly for trap entry and return
//!   - Hardware trap classification (`loongarch64_trap_handler`)
//!   - IRQ / TLB initialisation
//!   - User-entry helper (`enter_user_and_trap`)
//!
//! The high-level dispatch (`trap_handler() -> !`) that used to live here
//! has been moved to the kernel crate.  The arch layer now calls back into
//! the kernel via [`crate::api::ArchInterface::kernel_interrupt`].

#![allow(missing_docs)]

use core::arch::global_asm;

use loongArch64::register::estat::{self, Exception, Trap};
use loongArch64::register::{
    badv, crmd, ecfg, eentry, era, pgdh, pgdl, prmd, pwch, pwcl, stlbps, ticlr, tlbidx,
    tlbrehi, tlbrentry,
};

use super::unaligned::emulate_load_store_insn;
use super::context::TrapFrame;
use crate::TrapType;
use crate::api::ArchInterface;

// ---------------------------------------------------------------------------
// Trap-frame save / restore macros (inline assembly)
// ---------------------------------------------------------------------------

global_asm!(
    include_str!("trap.S"),
    trapframe_size = const super::consts::TRAPFRAME_SIZE,
    trap_handler = sym loongarch64_trap_handler,
);

extern "C" {
    fn user_restore(context: *mut TrapFrame);
    fn trap_vector_base();
    fn tlb_fill();
    fn user_rw_exception_entry();
    pub fn try_write_user();
    pub fn try_read_user();
}

// ---------------------------------------------------------------------------
// Init helpers
// ---------------------------------------------------------------------------

pub fn init_interrupt() {
    tlb_init(tlb_fill as _);
}

// ---------------------------------------------------------------------------
// User-mode trap entry / exit (naked functions)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// IRQ enable / disable
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[inline(always)]
pub fn enable_irq() {
    prmd::set_pie(true);
}

#[inline(always)]
pub fn disable_irq() {
    prmd::set_pie(false);
}

#[inline(always)]
pub fn enable_external_irq() {}

// ---------------------------------------------------------------------------
// Enter userspace and return on the next trap
// ---------------------------------------------------------------------------

pub fn enter_user_and_trap(cx: &mut TrapFrame, user_token: usize) -> TrapType {
    super::page_table::activate_page_table(user_token);
    unsafe { user_restore(cx as *mut TrapFrame) };
    loongarch64_trap_handler(cx)
}

// ---------------------------------------------------------------------------
// Trap init / enable
// ---------------------------------------------------------------------------

/// Initialize trap handling for LoongArch64.
pub fn trap_init() {
    set_trap_vector_base();
    init_interrupt();
}

/// Enable timer interrupts.
pub fn trap_enable_timer_interrupt() {
    enable_irq();
}

// ---------------------------------------------------------------------------
// Trap vector base (kernel-mode exception entry)
//
// When a trap arrives from kernel mode, the code in `trap_vector_base`
// allocates a stack frame, saves registers, calls the classifier
// (`loongarch64_trap_handler`) and returns via `ertn`.
//
// When the trap comes from user mode (PLV != 0), it jumps to `user_vec`
// which saves into the per-task TrapFrame and returns to Rust.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// TLB refill handler
// ---------------------------------------------------------------------------

#[inline]
pub fn set_tlb_refill(tlbrentry: usize) {
    tlbrentry::set_tlbrentry(tlbrentry & 0xFFFF_FFFF_FFFF);
}

pub const PS_4K: usize = 0x0c;
pub const _PS_16K: usize = 0x0e;
pub const _PS_2M: usize = 0x15;
pub const _PS_1G: usize = 0x1e;

pub const PAGE_SIZE_SHIFT: usize = 12;

pub fn tlb_init(tlbrentry: usize) {
    tlbidx::set_ps(PS_4K);
    stlbps::set_ps(PS_4K);
    tlbrehi::set_ps(PS_4K);

    pwcl::set_pte_width(8);
    pwcl::set_ptbase(PAGE_SIZE_SHIFT);
    pwcl::set_ptwidth(PAGE_SIZE_SHIFT - 3);

    pwcl::set_dir1_base(PAGE_SIZE_SHIFT + PAGE_SIZE_SHIFT - 3);
    pwcl::set_dir1_width(PAGE_SIZE_SHIFT - 3);

    pwch::set_dir3_base(PAGE_SIZE_SHIFT + PAGE_SIZE_SHIFT - 3 + PAGE_SIZE_SHIFT - 3);
    pwch::set_dir3_width(PAGE_SIZE_SHIFT - 3);

    set_tlb_refill(tlbrentry);
}

#[inline]
pub fn set_trap_vector_base() {
    ecfg::set_vs(0);
    eentry::set_eentry(trap_vector_base as usize);
}

// ---------------------------------------------------------------------------
// Hardware trap classifier
//
// Maps LoongArch `estat` exception / interrupt codes to `TrapType`.
// This is pure arch-level classification -- no kernel policy here.
// ---------------------------------------------------------------------------

fn loongarch64_trap_handler(tf: &mut TrapFrame) -> TrapType {
    let estat = estat::read();
    match estat.cause() {
        Trap::Exception(Exception::Breakpoint) => {
            tf.sepc += 4;
            TrapType::Breakpoint
        }
        Trap::Exception(Exception::AddressNotAligned) => {
            unsafe { emulate_load_store_insn(tf) }
            TrapType::Unknown
        }
        Trap::Interrupt(_) => {
            let irq_num: usize = estat.is().trailing_zeros() as usize;
            match irq_num {
                11 => {
                    ticlr::clear_timer_interrupt();
                    TrapType::Time
                }
                _ => TrapType::Unknown,
            }
        }
        Trap::Exception(Exception::InstructionNotExist) => {
            TrapType::IllegalInstruction(badv::read().raw())
        }
        Trap::Exception(Exception::Syscall) => TrapType::UserEnvCall,
        Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::PagePrivilegeIllegal)
        | Trap::Exception(Exception::PageModifyFault) => {
            TrapType::StorePageFault(badv::read().raw())
        }
        Trap::Exception(Exception::LoadPageFault)
        | Trap::Exception(Exception::PageNonReadableFault)
        | Trap::Exception(Exception::FetchPageFault)
        | Trap::Exception(Exception::FetchInstructionAddressError)
        | Trap::Exception(Exception::MemoryAccessAddressError) => {
            TrapType::LoadPageFault(badv::read().raw())
        }
        _ => TrapType::Unknown,
    }
}

// ---------------------------------------------------------------------------
// User-RW exception entry (for try_read_user / try_write_user)
// ---------------------------------------------------------------------------

pub unsafe fn set_kernel_user_rw_trap() {
    eentry::set_eentry(user_rw_exception_entry as usize);
}

pub unsafe fn set_kernel_trap() {
    eentry::set_eentry(trap_vector_base as usize);
}
