use super::*;

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

pub(super) fn handle_user_supervisor_external() {
    crate::board::irq_handler();
}

pub(super) fn handle_user_page_fault(addr: usize) {
    // Try COW first, then demand paging (both under the process lock)
    {
        let process = current_process();
        let mut inner = process.inner_exclusive_access();
        if inner.memory_set.handle_cow_fault(addr) {
            return;
        }
        if inner.memory_set.handle_demand_fault(addr) {
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
    let sepc = trap_cx.sepc;
    let ra = trap_cx[TrapFrameArgs::RA];
    let sp = trap_cx[TrapFrameArgs::SP];
    let tp = trap_cx[TrapFrameArgs::TLS];
    let in_sigreturn_trampoline =
        (arch::SIG_RETURN_ADDR..arch::SIG_RETURN_ADDR + crate::config::PAGE_SIZE).contains(&sepc);

    if let Some(task) = current_task() {
        let pid = current_process().pid.0;
        let mut task_inner = task.inner_exclusive_access();
        let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
        let handling_sig = task_inner.handling_sig;
        if task_inner.illegal_last_sepc == sepc {
            task_inner.illegal_repeat_count = task_inner.illegal_repeat_count.saturating_add(1);
        } else {
            task_inner.illegal_last_sepc = sepc;
            task_inner.illegal_repeat_count = 1;
        }
        let repeat = task_inner.illegal_repeat_count;

        // Synchronous illegal-instruction trap should be delivered to the faulting task.
        task_inner.signal_mask.remove(SignalFlags::SIGILL);
        task_inner.signal_pending.insert(SignalFlags::SIGILL);
        task_inner.interrupted_by_signal = true;

        let fatal_sigreturn_loop = in_sigreturn_trampoline && handling_sig != -1;
        drop(task_inner);

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
    let step = riscv_insn_len_at(user_token, trap_cx.sepc);
    trap_cx.sepc = trap_cx.sepc.wrapping_add(step);
}

pub(super) fn handle_user_unknown_trap(trap_type: arch::TrapType) {
    let trap_cx = current_trap_cx();
    warn!("[kernel] trap_handler: unsupported trap {:?} sepc={:#x}", trap_type, trap_cx.sepc);
}
