use super::*;

pub(super) fn handle_user_supervisor_external() {
    let trap_cx = current_trap_cx();
    warn!("[kernel] trap_handler: unknown trap at sepc={:#x}", trap_cx.sepc);
}

pub(super) fn handle_user_page_fault(addr: usize) {
    // Try COW first
    {
        let process = current_process();
        let mut inner = process.inner_exclusive_access();
        if inner.memory_set.handle_cow_fault(addr) {
            return;
        }
    }

    // Not COW-able: log and SIGSEGV
    let token = current_user_token();
    let page_table = PageTable::from_token(token);
    let fault_va = VirtAddr::from(addr);
    let fault_vpn = fault_va.floor();

    let trap_cx = current_trap_cx();
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
        addr, pid, tid, name, trap_cx.sepc
    );
    error!(
        "[kernel] trap_handler: ra={:#x} sp={:#x} tp={:#x} syscall={:#x} args={:x?}",
        trap_cx[TrapFrameArgs::RA],
        trap_cx[TrapFrameArgs::SP],
        trap_cx[TrapFrameArgs::TLS],
        trap_cx[TrapFrameArgs::SYSCALL],
        args
    );
    if let Some(pte) = page_table.translate(fault_vpn) {
        if pte.is_valid() {
            let offset = fault_va.page_offset();
            let end = core::cmp::min(offset + 8, PAGE_SIZE);
            let bytes = &pte.ppn().get_bytes_array()[offset..end];
            error!(
                "[kernel] trap_handler: fault pte ppn={:#x} flags={:?} bytes={:02x?}",
                pte.ppn().0, pte.flags(), bytes
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
                pte.ppn().0, pte.flags(), bytes
            );
        } else {
            error!("[kernel] trap_handler: sepc pte invalid flags={:?}", pte.flags());
        }
    } else {
        error!("[kernel] trap_handler: sepc pte unmapped");
    }
    if let Some(task) = current_task() {
        // Synchronous faults should be delivered to the faulting thread.
        // If SIGSEGV stays masked, user mode can loop on the same fault forever.
        let mut task_inner = task.inner_exclusive_access();
        task_inner.signal_mask.remove(SignalFlags::SIGSEGV);
        task_inner.signal_pending.insert(SignalFlags::SIGSEGV);
        task_inner.interrupted_by_signal = true;
    } else {
        current_add_signal(SignalFlags::SIGSEGV);
    }
}

pub(super) fn handle_user_illegal_instruction(addr: usize) {
    let trap_cx = current_trap_cx();
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

pub(super) fn handle_user_breakpoint() {
    let trap_cx = current_trap_cx();
    warn!("[kernel] trap_handler: unknown trap at sepc={:#x}", trap_cx.sepc);
}

pub(super) fn handle_user_unknown_trap(_trap_type: arch::TrapType) {
    let trap_cx = current_trap_cx();
    warn!("[kernel] trap_handler: unknown trap at sepc={:#x}", trap_cx.sepc);
}
