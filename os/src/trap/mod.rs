//! Trap handling for the kernel.
//!
//! User-mode traps are handled in `user_trap_loop()` (arch-specific).
//! Kernel-mode traps are dispatched
//! through `ArchInterface::kernel_interrupt` -> `kernel_interrupt_dispatch`.

#![allow(missing_docs)]
#![allow(unused_imports)]

use crate::config::PAGE_SIZE;
use crate::mm::{PageTable, VirtAddr};
use crate::syscall::syscall;
use crate::task::{
    current_add_signal, current_process, current_task, current_trap_cx, current_user_token,
    handle_signals, process_interval_timers, suspend_current_and_run_next, SignalFlags,
};
use crate::timer::{check_timer, set_next_trigger};
use alloc::string::String;
use arch::TrapFrameArgs;
use core::sync::atomic::{AtomicU64, Ordering};

const TIMER_SAMPLE_INTERVAL: u64 = 200;
static TIMER_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg_attr(target_arch = "riscv64", path = "user_trap_riscv64.rs")]
#[cfg_attr(target_arch = "loongarch64", path = "user_trap_loongarch64.rs")]
mod user_trap_arch;

pub use user_trap_loop as user_trap_entry;

fn handle_user_syscall() {
    let trap_cx = current_trap_cx();
    trap_cx.sepc += 4;
    let result = syscall(trap_cx[TrapFrameArgs::SYSCALL], trap_cx.args());
    current_trap_cx()[TrapFrameArgs::RET] = result as usize;
}

fn handle_user_time_interrupt() {
    set_next_trigger();
    check_timer();
    process_interval_timers(true);
    handle_signals();
    suspend_current_and_run_next();
}

pub fn kernel_interrupt_dispatch(trap_type: arch::TrapType) {
    match trap_type {
        arch::TrapType::SupervisorExternal => {
            crate::platform::handle_external_irq();
        }
        arch::TrapType::Time => {
            set_next_trigger();
            check_timer();
            process_interval_timers(false);

            let count = TIMER_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
            if count % TIMER_SAMPLE_INTERVAL == 0 {
                if let Some(task) = current_task() {
                    if let Some(process) = task.process.upgrade() {
                        let pid = process.pid.0;
                        let name = process.name();
                        let (sepc, sp, ra) = task.with_trap_cx_mut(|trap_cx| {
                            (
                                trap_cx.sepc,
                                trap_cx[TrapFrameArgs::SP],
                                trap_cx[TrapFrameArgs::RA],
                            )
                        });
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
