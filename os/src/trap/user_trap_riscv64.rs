use super::*;

pub(super) fn handle_user_supervisor_external() {
    crate::platform::handle_external_irq();
}

pub(super) fn handle_user_page_fault(addr: usize) {
    // Try COW first, then demand paging (both under the process lock)
    {
        let process = current_process();
        if process.handle_cow_or_demand_fault(addr) {
            return;
        }
    }

    // Not COW-able: log and SIGSEGV
    let trap_cx = current_trap_cx();
    error!(
        "[kernel] trap_handler: page fault addr={:#x} sepc={:#x} ra={:#x} sp={:#x} tp={:#x}",
        addr,
        trap_cx.sepc,
        trap_cx[TrapFrameArgs::RA],
        trap_cx[TrapFrameArgs::SP],
        trap_cx[TrapFrameArgs::TLS]
    );
    if let Some(task) = current_task() {
        // Synchronous faults should be delivered to the faulting thread.
        // If SIGSEGV stays masked, user mode can loop on the same fault forever.
        task.with_signals_mut(|signals| {
            signals.signal_mask.remove(SignalFlags::SIGSEGV);
            signals.signal_pending.insert(SignalFlags::SIGSEGV);
            signals.interrupted_by_signal = true;
        });
    } else {
        current_add_signal(SignalFlags::SIGSEGV);
    }
}

pub(super) fn handle_user_illegal_instruction(addr: usize) {
    let trap_cx = current_trap_cx();
    let sepc = trap_cx.sepc;
    let ra = trap_cx[TrapFrameArgs::RA];
    let sp = trap_cx[TrapFrameArgs::SP];
    let tp = trap_cx[TrapFrameArgs::TLS];
    let in_sigreturn_trampoline = arch::is_sigreturn_trampoline_pc(sepc);

    if let Some(task) = current_task() {
        let pid = current_process().pid.0;
        let tid = task.tid();
        let handling_sig = task.handling_sig();
        let repeat = task.record_illegal_instruction(sepc);

        // Synchronous illegal-instruction trap should be delivered to the faulting task.
        task.with_signals_mut(|signals| {
            signals.signal_mask.remove(SignalFlags::SIGILL);
            signals.signal_pending.insert(SignalFlags::SIGILL);
            signals.interrupted_by_signal = true;
        });

        let fatal_sigreturn_loop = in_sigreturn_trampoline && handling_sig != -1;

        if fatal_sigreturn_loop {
            warn!(
                "[kernel] trap_handler: illegal instruction in sigreturn trampoline, pid={} tid={} sepc={:#x} ra={:#x} sp={:#x} tp={:#x} handling_sig={} -> force exit",
                pid, tid, sepc, ra, sp, tp, handling_sig
            );
            crate::task::exit_current_and_run_next(-(crate::task::SIGILL as i32));
            panic!("Unreachable after fatal illegal instruction in sigreturn trampoline");
        }

        if repeat == 1 {
            let level = if in_sigreturn_trampoline {
                "[warn/sigtrx]"
            } else {
                "[error]"
            };
            if in_sigreturn_trampoline {
                warn!(
                    "{} [kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x} ra={:#x} sp={:#x} tp={:#x} pid={} tid={}",
                    level, addr, sepc, ra, sp, tp, pid, tid
                );
            } else {
                error!(
                    "{} [kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x} ra={:#x} sp={:#x} tp={:#x} pid={} tid={}",
                    level, addr, sepc, ra, sp, tp, pid, tid
                );
            }
        } else if repeat % 128 == 0 {
            warn!(
                "[kernel] trap_handler: repeated illegal instruction suppressed pid={} tid={} sepc={:#x} repeat={}",
                pid, tid, sepc, repeat
            );
        }
        return;
    }

    if in_sigreturn_trampoline {
        warn!(
            "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x} ra={:#x} sp={:#x} tp={:#x}",
            addr, sepc, ra, sp, tp
        );
    } else {
        error!(
            "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x} ra={:#x} sp={:#x} tp={:#x}",
            addr, sepc, ra, sp, tp
        );
    }
    current_add_signal(SignalFlags::SIGILL);
}

pub(super) fn handle_user_breakpoint() {
    let user_token = current_user_token();
    let trap_cx = current_trap_cx();
    if let Some(next_pc) = arch::breakpoint_next_pc(user_token, trap_cx.sepc) {
        trap_cx.sepc = next_pc;
    }
}

pub(super) fn handle_user_unknown_trap(trap_type: arch::TrapType) {
    let trap_cx = current_trap_cx();
    warn!(
        "[kernel] trap_handler: unsupported trap {:?} sepc={:#x}",
        trap_type, trap_cx.sepc
    );
}
