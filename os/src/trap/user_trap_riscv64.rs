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
    current_add_signal(SignalFlags::SIGSEGV);
}

pub(super) fn handle_user_illegal_instruction(addr: usize) {
    let trap_cx = current_trap_cx();
    error!(
        "[kernel] trap_handler: illegal instruction addr={:#x} sepc={:#x}",
        addr,
        trap_cx.sepc
    );
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
