//! LoongArch64 boot sequence
//!
//! This module implements the earliest entry point for LoongArch64 CPUs.
//! It initializes Direct Memory Windows (DMW) and enables paging before jumping to Rust code.

use core::arch::asm;

/// Boot stack size (64KB)
const BOOT_STACK_SIZE: usize = 4096 * 16;

/// Boot stack
#[link_section = ".bss.stack"]
static mut BOOT_STACK: [u8; BOOT_STACK_SIZE] = [0; BOOT_STACK_SIZE];

/// The earliest entry point for the primary CPU.
///
/// Initializes DMW (Direct Memory Windows) for LoongArch64:
/// - DMW0: 0x8000_xxxx_xxxx_xxxx (Uncached, for MMIO)
/// - DMW1: 0x9000_xxxx_xxxx_xxxx (Cached, for kernel)
///
/// Then enables paging and jumps to rust_main.
#[naked]
#[no_mangle]
#[link_section = ".text.entry"]
pub unsafe extern "C" fn _start() -> ! {
    asm!(
        "
        # Initialize DMW0 (Direct Memory Window 0)
        # DMW0: UC (Uncached), PLV0, 0x8000_xxxx_xxxx_xxxx
        ori         $t0, $zero, 0x1     # CSR_DMW0_PLV0
        lu52i.d     $t0, $t0, -2048     # Set to 0x8000_0000_0000_0001
        csrwr       $t0, 0x180          # Write to LOONGARCH_CSR_DMWIN0

        # Initialize DMW1 (Direct Memory Window 1)
        # DMW1: CA (Cached), PLV0, 0x9000_xxxx_xxxx_xxxx
        ori         $t0, $zero, 0x11    # CSR_DMW1_MAT | CSR_DMW1_PLV0
        lu52i.d     $t0, $t0, -1792     # Set to 0x9000_0000_0000_0011
        csrwr       $t0, 0x181          # Write to LOONGARCH_CSR_DMWIN1

        # Enable paging
        li.w        $t0, 0xb0           # PLV=0, IE=0, PG=1
        csrwr       $t0, 0x0            # Write to LOONGARCH_CSR_CRMD

        # Initialize PRMD (Pre-exception Mode)
        li.w        $t0, 0x00           # PLV=0, PIE=0, PWE=0
        csrwr       $t0, 0x1            # Write to LOONGARCH_CSR_PRMD

        # Initialize EUEN (Extended Unit Enable)
        li.w        $t0, 0x00           # FPE=0, SXE=0, ASXE=0, BTE=0
        csrwr       $t0, 0x2            # Write to LOONGARCH_CSR_EUEN

        # Set up boot stack
        la.global   $sp, {boot_stack}
        li.d        $t0, {boot_stack_size}
        add.d       $sp, $sp, $t0       # $sp = boot_stack + BOOT_STACK_SIZE

        # Read CPU ID
        csrrd       $a0, 0x20           # Read CPUID into $a0

        # Jump to Rust main
        la.global   $t0, {rust_main}
        jirl        $zero, $t0, 0       # Jump to rust_main
        ",
        boot_stack = sym BOOT_STACK,
        boot_stack_size = const BOOT_STACK_SIZE,
        rust_main = sym crate::rust_main,
        options(noreturn)
    )
}
