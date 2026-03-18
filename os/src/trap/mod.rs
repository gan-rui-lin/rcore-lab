//! Trap handling for the kernel.
//!
//! User-mode traps are handled in `user_trap_loop()` (arch-specific).
//! Kernel-mode traps are dispatched
//! through `ArchInterface::kernel_interrupt` → `kernel_interrupt_dispatch`.

#![allow(missing_docs)]
#![allow(unused_imports)]

use crate::syscall::syscall;
use crate::task::{
    current_add_signal, current_process, current_task, current_trap_cx,
    current_user_token, handle_signals, suspend_current_and_run_next, SignalFlags,
};
use crate::mm::{PageTable, VirtAddr};
use crate::config::PAGE_SIZE;
use crate::timer::{check_timer, set_next_trigger};
use arch::TrapFrameArgs;
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::string::String;

const TIMER_SAMPLE_INTERVAL: u64 = 200;
static TIMER_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg_attr(target_arch = "riscv64", path = "user_trap_riscv64.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "user_trap_loongarch64.rs")]
mod user_trap_arch;

fn handle_user_syscall() {
    let trap_cx = current_trap_cx();
    trap_cx.sepc += 4;
    let result = syscall(trap_cx[TrapFrameArgs::SYSCALL], trap_cx.args());
    current_trap_cx()[TrapFrameArgs::RET] = result as usize;
}

fn handle_user_time_interrupt() {
    set_next_trigger();
    check_timer();
    handle_signals();
    suspend_current_and_run_next();
}

// ---------------------------------------------------------------------------
// Kernel-mode trap dispatch (called from ArchInterface::kernel_interrupt)
// ---------------------------------------------------------------------------

/// Dispatch a kernel-mode trap / interrupt.
///
/// Called from `arch::api::ArchInterface::kernel_interrupt` which is
/// invoked by the architecture's kernel-mode trap handler after it has
/// classified the hardware event into a [`arch::TrapType`].
pub fn kernel_interrupt_dispatch(trap_type: arch::TrapType) {
    match trap_type {
        arch::TrapType::SupervisorExternal => {
            crate::board::irq_handler();
        }
        arch::TrapType::Time => {
            set_next_trigger();
            check_timer();

            let count = TIMER_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
            if count % TIMER_SAMPLE_INTERVAL == 0 {
                if let Some(task) = current_task() {
                    if let Some(process) = task.process.upgrade() {
                        let pid = process.pid.0;
                        let name = process.inner_exclusive_access().name.clone();
                        let (sepc, sp, ra) = {
                            let task_inner = task.inner_exclusive_access();
                            let trap_cx = task_inner.get_trap_cx();
                            (
                                trap_cx.sepc,
                                trap_cx[TrapFrameArgs::SP],
                                trap_cx[TrapFrameArgs::RA],
                            )
                        };
                        info!(
                            "[sample-k] pid={} name={} sepc={:#x} sp={:#x} ra={:#x}",
                            pid, name, sepc, sp, ra
                        );
                    }
                }
            }
        }
        _ => {
            panic!("Unsupported trap from kernel: {:?}", trap_type);
        }
    }
}

// ---------------------------------------------------------------------------
// User-mode trap entry loop (LoongArch64)
//
// On LoongArch64 the kernel enters user mode via a loop: restore user
// registers -> ertn -> (user runs) -> trap -> user_vec returns here.
// This avoids the old trampoline-style return path and keeps the kernel
// control flow in a direct loop.
// ---------------------------------------------------------------------------

pub fn user_trap_loop() -> ! {
    loop {
        let user_token = current_user_token();
        let trap_type = arch::enter_user_and_trap(current_trap_cx(), user_token);

        match trap_type {
            arch::TrapType::UserEnvCall => {
                handle_user_syscall();
            }
            arch::TrapType::Time => {
                handle_user_time_interrupt();
            }
            arch::TrapType::SupervisorExternal => {
                user_trap_arch::handle_user_supervisor_external();
            }
            arch::TrapType::StorePageFault(addr)
            | arch::TrapType::LoadPageFault(addr)
            | arch::TrapType::InstructionPageFault(addr) => {
                user_trap_arch::handle_user_page_fault(addr);
            }
            arch::TrapType::IllegalInstruction(addr) => {
                user_trap_arch::handle_user_illegal_instruction(addr);
            }
            arch::TrapType::Breakpoint => {
                user_trap_arch::handle_user_breakpoint();
            }
            arch::TrapType::Unknown => {
                user_trap_arch::handle_user_unknown_trap(trap_type);
            }
        }
        if current_task().is_some() {
            handle_signals();
        }
    }
}

/// Debug helper for GDB: translate a user virtual address to physical address.
/// Returns 0 if unmapped.
#[cfg(target_arch = "riscv64")]
#[no_mangle]
#[link_section = ".text.keep"]
#[allow(dead_code)]
pub extern "C" fn debug_user_va_to_pa(va: usize) -> usize {
    let token = current_user_token();
    let page_table = PageTable::from_token(token);
    match page_table.translate_va(VirtAddr::from(va)) {
        Some(pa) => pa.into(),
        None => 0,
    }
}
