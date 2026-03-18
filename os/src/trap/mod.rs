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

#[cfg(target_arch = "loongarch64")]
pub fn user_trap_loop() -> ! {
    use arch::TrapType;
    loop {
        let user_token = current_user_token();
        let trap_type = arch::enter_user_and_trap(current_trap_cx(), user_token);
        let trap_cx = current_trap_cx();

        match trap_type {
            TrapType::UserEnvCall => {
                trap_cx.sepc += 4;
                let result = syscall(trap_cx[TrapFrameArgs::SYSCALL], trap_cx.args());
                let trap_cx = current_trap_cx();
                trap_cx[TrapFrameArgs::RET] = result as usize;
            }
            TrapType::Time => {
                set_next_trigger();
                check_timer();
                handle_signals();
                suspend_current_and_run_next();
            }
            TrapType::StorePageFault(addr)
            | TrapType::LoadPageFault(addr)
            | TrapType::InstructionPageFault(addr) => {
                let (pid, tid, name) = if let Some(task) = current_task() {
                    let task_inner = task.inner_exclusive_access();
                    let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
                    let name = task
                        .process
                        .upgrade()
                        .map(|p| p.inner_exclusive_access().name.clone())
                        .unwrap_or_else(|| String::from("<unknown>"));
                    (current_process().pid.0, tid, name)
                } else {
                    (0, 0, String::from("<no-task>"))
                };
                let args = trap_cx.args();
                error!(
                    "[kernel] trap_handler: page fault addr={:#x} pid={} tid={} name={} sepc={:#x}",
                    addr,
                    pid,
                    tid,
                    name,
                    trap_cx.sepc
                );
                error!(
                    "[kernel] trap_handler: ra={:#x} sp={:#x} tp={:#x} syscall={:#x} args={:x?}",
                    trap_cx[TrapFrameArgs::RA],
                    trap_cx[TrapFrameArgs::SP],
                    trap_cx[TrapFrameArgs::TLS],
                    trap_cx[TrapFrameArgs::SYSCALL],
                    args
                );
                let token = current_user_token();
                let page_table = PageTable::from_token(token);
                let fault_va = VirtAddr::from(addr as usize);
                if let Some(pte) = page_table.translate(fault_va.floor()) {
                    if pte.is_valid() {
                        let offset = fault_va.page_offset();
                        let end = core::cmp::min(offset + 8, PAGE_SIZE);
                        let bytes = &pte.ppn().get_bytes_array()[offset..end];
                        error!(
                            "[kernel] trap_handler: fault pte ppn={:#x} flags={:?} bytes={:02x?}",
                            pte.ppn().0,
                            pte.flags(),
                            bytes
                        );
                    } else {
                        error!("[kernel] trap_handler: fault pte invalid flags={:?}", pte.flags());
                    }
                } else {
                    error!("[kernel] trap_handler: fault pte unmapped");
                }
                let sepc_va = VirtAddr::from(trap_cx.sepc);
                if let Some(pte) = page_table.translate(sepc_va.floor()) {
                    if pte.is_valid() {
                        let offset = sepc_va.page_offset();
                        let end = core::cmp::min(offset + 8, PAGE_SIZE);
                        let bytes = &pte.ppn().get_bytes_array()[offset..end];
                        error!(
                            "[kernel] trap_handler: sepc pte ppn={:#x} flags={:?} bytes={:02x?}",
                            pte.ppn().0,
                            pte.flags(),
                            bytes
                        );
                    } else {
                        error!("[kernel] trap_handler: sepc pte invalid flags={:?}", pte.flags());
                    }
                } else {
                    error!("[kernel] trap_handler: sepc pte unmapped");
                }
                current_add_signal(SignalFlags::SIGSEGV);
                // ! 暂时直接 shutdown，后续可以考虑杀死进程
                // use arch::shutdown;
                // shutdown();

            }
            TrapType::IllegalInstruction(addr) => {
                let args = trap_cx.args();
                error!(
                    "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x}",
                    addr,
                    trap_cx.sepc
                );
                error!(
                    "[kernel] trap_handler: ra={:#x} sp={:#x} tp={:#x} syscall={:#x} args={:x?}",
                    trap_cx[TrapFrameArgs::RA],
                    trap_cx[TrapFrameArgs::SP],
                    trap_cx[TrapFrameArgs::TLS],
                    trap_cx[TrapFrameArgs::SYSCALL],
                    args
                );
                current_add_signal(SignalFlags::SIGILL);
            }
            arch::TrapType::Unknown => {}
            _ => warn!("[kernel] trap_handler: unknown trap at sepc={:#x}", trap_cx.sepc),
        }
        if current_task().is_some() {
            handle_signals();
        }
        // loop back to re-enter user mode
    }
}

// ---------------------------------------------------------------------------
// Return to user space
// ---------------------------------------------------------------------------

/// Enter user space and loop on user traps for the current task.
#[cfg(target_arch = "riscv64")]
pub fn user_trap_loop() -> ! {
    fn riscv_insn_len_at(user_token: usize, sepc: usize) -> usize {
        let page_table = PageTable::from_token(user_token);
        let read_byte = |va: usize| -> Option<u8> {
            let pa = page_table.translate_va(VirtAddr::from(va))?;
            Some(*pa.get_ref::<u8>())
        };
        let (b0, b1) = match (read_byte(sepc), read_byte(sepc.wrapping_add(1))) {
            (Some(a), Some(b)) => (a, b),
            _ => return 2,
        };
        let insn16 = u16::from_le_bytes([b0, b1]);
        if (insn16 & 0b11) == 0b11 { 4 } else { 2 }
    }

    loop {
        let user_token = current_user_token();
        let trap_type = arch::enter_user_and_trap(current_trap_cx(), user_token);

        match trap_type {
            arch::TrapType::UserEnvCall => {
                let trap_cx = current_trap_cx();
                trap_cx.sepc += 4;
                let result = syscall(trap_cx[TrapFrameArgs::SYSCALL], trap_cx.args());
                trap_cx[TrapFrameArgs::RET] = result as usize;
            }
            arch::TrapType::Time => {
                set_next_trigger();
                check_timer();
                handle_signals();
                suspend_current_and_run_next();
            }
            arch::TrapType::SupervisorExternal => {
                crate::board::irq_handler();
            }
            arch::TrapType::StorePageFault(addr)
            | arch::TrapType::LoadPageFault(addr)
            | arch::TrapType::InstructionPageFault(addr) => {
                let trap_cx = current_trap_cx();
                error!(
                    "[kernel] trap_handler: page fault addr={:#x} sepc={:#x} ra={:#x} sp={:#x}",
                    addr,
                    trap_cx.sepc,
                    trap_cx[TrapFrameArgs::RA],
                    trap_cx[TrapFrameArgs::SP]
                );
                current_add_signal(SignalFlags::SIGSEGV);
            }
            arch::TrapType::IllegalInstruction(addr) => {
                let trap_cx = current_trap_cx();
                error!(
                    "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x}",
                    addr,
                    trap_cx.sepc
                );
                current_add_signal(SignalFlags::SIGILL);
            }
            arch::TrapType::Breakpoint => {
                let user_token = current_user_token();
                let trap_cx = current_trap_cx();
                let step = riscv_insn_len_at(user_token, trap_cx.sepc);
                trap_cx.sepc = trap_cx.sepc.wrapping_add(step);
            }
            _ => {
                let trap_cx = current_trap_cx();
                warn!("[kernel] trap_handler: unsupported trap {:?} sepc={:#x}", trap_type, trap_cx.sepc);
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
