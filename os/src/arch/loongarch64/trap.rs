//! LoongArch64 trap handling
//!
//! This module implements exception and interrupt handling for LoongArch64.

#![allow(dead_code)]
#![allow(missing_docs)]

use core::arch::{asm, global_asm};
use loongArch64::register::{
    badv, ecfg, eentry, estat::{self, Exception, Trap}, prmd, ticlr,
    pwcl, pwch, tlbidx, stlbps, tlbrehi, tlbrentry,
};

use super::context::TrapContext;

/// Size of TrapContext structure (in bytes)
const TRAPFRAME_SIZE: usize = core::mem::size_of::<TrapContext>();

// KSAVE CSR indices
const KSAVE_KSP: usize = 0x30;   // Kernel stack pointer
const KSAVE_CTX: usize = 0x31;   // User context address
const KSAVE_USP: usize = 0x32;   // User stack pointer

// Assembly macros for saving and restoring registers
global_asm!(
    r#"
        .altmacro
        .equ KSAVE_KSP,  0x30
        .equ KSAVE_CTX,  0x31
        .equ KSAVE_USP,  0x32

        .macro SAVE_REGS
            st.d    $ra, $sp,  1*8
            st.d    $tp, $sp,  2*8
            st.d    $a0, $sp,  4*8
            st.d    $a1, $sp,  5*8
            st.d    $a2, $sp,  6*8
            st.d    $a3, $sp,  7*8
            st.d    $a4, $sp,  8*8
            st.d    $a5, $sp,  9*8
            st.d    $a6, $sp, 10*8
            st.d    $a7, $sp, 11*8
            st.d    $t0, $sp, 12*8
            st.d    $t1, $sp, 13*8
            st.d    $t2, $sp, 14*8
            st.d    $t3, $sp, 15*8
            st.d    $t4, $sp, 16*8
            st.d    $t5, $sp, 17*8
            st.d    $t6, $sp, 18*8
            st.d    $t7, $sp, 19*8
            st.d    $t8, $sp, 20*8
            st.d    $r21,$sp, 21*8
            st.d    $fp, $sp, 22*8
            st.d    $s0, $sp, 23*8
            st.d    $s1, $sp, 24*8
            st.d    $s2, $sp, 25*8
            st.d    $s3, $sp, 26*8
            st.d    $s4, $sp, 27*8
            st.d    $s5, $sp, 28*8
            st.d    $s6, $sp, 29*8
            st.d    $s7, $sp, 30*8
            st.d    $s8, $sp, 31*8

            csrrd   $t0, KSAVE_USP
            st.d    $t0, $sp,  3*8       // Save user sp

            csrrd   $t0, 0x1
            st.d    $t0, $sp, 32*8       // Save PRMD

            csrrd   $t0, 0x6
            st.d    $t0, $sp, 33*8       // Save ERA
        .endm

        .macro LOAD_REGS
            ld.d    $t0, $sp, 32*8
            csrwr   $t0, 0x1             // Restore PRMD

            ld.d    $t0, $sp, 33*8
            csrwr   $t0, 0x6             // Restore ERA

            ld.d    $ra, $sp,  1*8
            ld.d    $tp, $sp,  2*8
            ld.d    $a0, $sp,  4*8
            ld.d    $a1, $sp,  5*8
            ld.d    $a2, $sp,  6*8
            ld.d    $a3, $sp,  7*8
            ld.d    $a4, $sp,  8*8
            ld.d    $a5, $sp,  9*8
            ld.d    $a6, $sp, 10*8
            ld.d    $a7, $sp, 11*8
            ld.d    $t0, $sp, 12*8
            ld.d    $t1, $sp, 13*8
            ld.d    $t2, $sp, 14*8
            ld.d    $t3, $sp, 15*8
            ld.d    $t4, $sp, 16*8
            ld.d    $t5, $sp, 17*8
            ld.d    $t6, $sp, 18*8
            ld.d    $t7, $sp, 19*8
            ld.d    $t8, $sp, 20*8
            ld.d    $r21,$sp, 21*8
            ld.d    $fp, $sp, 22*8
            ld.d    $s0, $sp, 23*8
            ld.d    $s1, $sp, 24*8
            ld.d    $s2, $sp, 25*8
            ld.d    $s3, $sp, 26*8
            ld.d    $s4, $sp, 27*8
            ld.d    $s5, $sp, 28*8
            ld.d    $s6, $sp, 29*8
            ld.d    $s7, $sp, 30*8
            ld.d    $s8, $sp, 31*8

            // Restore sp last
            ld.d    $sp, $sp,  3*8
        .endm
    "#
);

/// Trap vector base - main exception entry point
#[naked]
#[no_mangle]
pub unsafe extern "C" fn __alltraps() {
    asm!(
        r#"
            .balign 4096
            // Check if from user mode (PLV in PRMD)
            csrwr   $sp, {ksave_usp}
            csrrd   $sp, 0x1             // Read PRMD
            andi    $sp, $sp, 0x3        // Extract PLV (bits 1:0)
            bnez    $sp, 1f              // If PLV != 0, go to user trap

            // Kernel trap (not implemented yet, should not happen in rcore-lab)
            csrrd   $sp, {ksave_usp}
            break   3                     // Breakpoint for debugging

        1:
            csrrd   $sp, {ksave_ctx}    // Load user context pointer
            SAVE_REGS

            move    $a0, $sp             // Pass context as argument
            bl      {trap_handler}       // Call Rust trap handler

            // Trap handler returns here
            LOAD_REGS
            ertn                         // Exception return
        "#,
        ksave_usp = const KSAVE_USP,
        ksave_ctx = const KSAVE_CTX,
        trap_handler = sym trap_handler,
        options(noreturn)
    )
}


/// TLB refill handler (hardware page table walk)
#[naked]
#[no_mangle]
pub unsafe extern "C" fn __tlb_refill() {
    asm!(
        r#"
            .balign 4096
            csrwr   $t0, 0x8b            // Save $t0 to TLBRSAVE
            csrrd   $t0, 0x1b            // Read PGD base
            lddir   $t0, $t0, 3          // Level 3 lookup
            lddir   $t0, $t0, 1          // Level 1 lookup
            ldpte   $t0, 0               // Load PTE (even)
            ldpte   $t0, 1               // Load PTE (odd)
            tlbfill                      // Fill TLB
            csrrd   $t0, 0x8b            // Restore $t0
            ertn
        "#,
        options(noreturn)
    )
}

/// Main trap handler in Rust
#[no_mangle]
pub fn trap_handler(cx: &mut TrapContext) {
    use crate::task::{current_add_signal, handle_signals, SigNumber};

    let estat = estat::read();
    let scause = estat.cause();

    match scause {
        // System call
        Trap::Exception(Exception::Syscall) => {
            cx.era += 4; // Skip syscall instruction
            let result = crate::syscall::syscall(
                cx.syscall_number(),
                cx.syscall_args(),
            );
            cx.x[4] = result as usize; // a0 = return value (LoongArch: a0 is x4)
        }

        // Timer interrupt
        Trap::Interrupt(_) => {
            let irq = estat.is().trailing_zeros() as usize;
            if irq == 11 {
                ticlr::clear_timer_interrupt();
                crate::timer::set_next_trigger();
                crate::timer::check_timer();
                crate::task::suspend_current_and_run_next();
            } else {
                panic!("Unknown interrupt: {}", irq);
            }
        }

        // Page faults → send SIGSEGV
        Trap::Exception(Exception::LoadPageFault)
        | Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::FetchPageFault) => {
            let bad_addr = badv::read().raw();
            error!(
                "[kernel] {:?} in application, bad addr = {:#x}, ERA = {:#x}",
                scause, bad_addr, cx.era
            );
            current_add_signal(SigNumber::SigSegv as usize);
        }

        // Illegal instruction → send SIGILL
        Trap::Exception(Exception::InstructionNotExist) => {
            error!(
                "[kernel] IllegalInstruction in application, ERA = {:#x}",
                cx.era
            );
            current_add_signal(SigNumber::SigIll as usize);
        }

        // Unaligned access → send SIGBUS
        Trap::Exception(Exception::AddressNotAligned) => {
            error!(
                "[kernel] UnalignedAccess in application at {:#x}",
                cx.era
            );
            current_add_signal(SigNumber::SigBus as usize);
        }

        _ => {
            panic!(
                "Unsupported trap {:?}, ERA = {:#x}, BADV = {:#x}",
                scause,
                cx.era,
                badv::read().raw()
            );
        }
    }

    // Deliver pending signals before returning to user space
    handle_signals();
}

/// Initialize trap handling
pub fn init() {
    extern "C" {
        fn __alltraps();
        fn __tlb_refill();
    }

    // Set exception entry point
    eentry::set_eentry(__alltraps as usize);

    // Set TLB refill entry
    tlbrentry::set_tlbrentry(__tlb_refill as usize);

    // Initialize TLB
    init_tlb();

    // Configure exception settings
    ecfg::set_vs(0);  // Vector spacing = 0 (single entry point)

    info!("[kernel] LoongArch64 trap initialized");
}

/// Initialize TLB settings
fn init_tlb() {
    const PS_4K: usize = 0x0c;          // 4KB page size
    const PAGE_SIZE_SHIFT: usize = 12;   // 2^12 = 4096

    // Set page size
    tlbidx::set_ps(PS_4K);
    stlbps::set_ps(PS_4K);
    tlbrehi::set_ps(PS_4K);

    // Configure page walk control
    pwcl::set_pte_width(8);              // 64-bit PTE
    pwcl::set_ptbase(PAGE_SIZE_SHIFT);
    pwcl::set_ptwidth(PAGE_SIZE_SHIFT - 3);

    pwcl::set_dir1_base(PAGE_SIZE_SHIFT + PAGE_SIZE_SHIFT - 3);
    pwcl::set_dir1_width(PAGE_SIZE_SHIFT - 3);

    pwch::set_dir3_base(PAGE_SIZE_SHIFT + PAGE_SIZE_SHIFT - 3 + PAGE_SIZE_SHIFT - 3);
    pwch::set_dir3_width(PAGE_SIZE_SHIFT - 3);
}

/// Enable timer interrupts
pub fn enable_timer_interrupt() {
    prmd::set_pie(true);
}

/// Return to user mode
#[naked]
pub unsafe extern "C" fn trap_return() -> ! {
    asm!(
        r#"
            // Load trap context from current task
            // Context address should be in KSAVE_CTX
            csrrd   $sp, {ksave_ctx}

            LOAD_REGS
            ertn
        "#,
        ksave_ctx = const KSAVE_CTX,
        options(noreturn)
    )
}

/// Set user trap context for the current task
pub fn set_user_trap_entry(cx_ptr: usize, kernel_sp: usize) {
    unsafe {
        // Save context pointer and kernel sp in KSAVE registers
        asm!(
            "csrwr {ctx}, {ksave_ctx}",
            "csrwr {ksp}, {ksave_ksp}",
            ctx = in(reg) cx_ptr,
            ksp = in(reg) kernel_sp,
            ksave_ctx = const KSAVE_CTX,
            ksave_ksp = const KSAVE_KSP,
        );
    }
}
