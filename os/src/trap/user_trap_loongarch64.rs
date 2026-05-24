use super::*;

pub(super) fn handle_user_supervisor_external() {
    let trap_cx = current_trap_cx();
    warn!(
        "[kernel] trap_handler: unknown trap at sepc={:#x}",
        trap_cx.sepc
    );
}

pub(super) fn handle_user_page_fault(addr: usize) {
    // Try COW first, then demand paging (both under the process lock)
    {
        let process = current_process();
        if process.handle_cow_or_demand_fault(addr) {
            return;
        }
    }

    // Not COW-able: deliver SIGSEGV to user.
    let trap_cx = current_trap_cx();
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        debug!(
            "[trap] page fault pid={} addr={:#x} sepc={:#x} ra={:#x} sp={:#x}",
            pid,
            addr,
            trap_cx.sepc,
            trap_cx[TrapFrameArgs::RA],
            trap_cx[TrapFrameArgs::SP]
        );
    }
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
    let args = trap_cx.args();
    error!(
        "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x}",
        addr, trap_cx.sepc
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

pub(super) fn handle_user_breakpoint() {
    let trap_cx = current_trap_cx();
    warn!(
        "[kernel] trap_handler: unknown trap at sepc={:#x}",
        trap_cx.sepc
    );
}

pub(super) fn handle_user_unknown_trap(_trap_type: arch::TrapType) {
    let trap_cx = current_trap_cx();
    warn!(
        "[kernel] trap_handler: unknown trap at sepc={:#x}",
        trap_cx.sepc
    );
}
