//! Trap handling for the kernel.
//!
//! User-mode traps arrive here via the function pointer stored in
//! `TrapContext.trap_handler`.  Kernel-mode traps are dispatched
//! through `ArchInterface::kernel_interrupt` → `kernel_interrupt_dispatch`.

#![allow(missing_docs)]
#![allow(unused_imports)]

use crate::syscall::syscall;
use crate::task::{
    current_add_signal, current_process, current_task, current_trap_cx, current_trap_cx_user_va,
    current_user_token, handle_signals, suspend_current_and_run_next, SignalFlags,
};
use crate::mm::{PageTable, VirtAddr};
use crate::config::PAGE_SIZE;
use crate::timer::{check_timer, set_next_trigger};
use arch::TrapFrameArgs;
use core::sync::atomic::{AtomicU64, Ordering};
use alloc::string::String;

#[cfg(target_arch = "riscv64")]
use crate::fs::{open_file, OpenFlags};
#[cfg(target_arch = "riscv64")]
use xmas_elf::ElfFile;

#[cfg(target_arch = "riscv64")]
pub use trap_handler as user_trap_entry;
#[cfg(target_arch = "loongarch64")]
pub use task_entry as user_trap_entry;

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
            #[cfg(target_arch = "riscv64")]
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
// User-mode trap handler (RISC-V 64)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "riscv64")]
fn dump_user_stack(page_table: &PageTable, sp: usize, bytes: usize) {
    let end = sp.saturating_add(bytes);
    error!("  user stack dump: sp={:#x} len={}", sp, bytes);
    let mut addr = sp;
    while addr < end {
        let mut line = [0u8; 16];
        let mut any = false;
        let line_end = core::cmp::min(addr + 16, end);
        for i in 0..(line_end - addr) {
            let va = addr + i;
            if let Some(pa) = page_table.translate_va(VirtAddr::from(va)) {
                line[i] = *pa.get_ref::<u8>();
                any = true;
            }
        }
        if any {
            error!("    {:#x}: {:02x?}", addr, &line[..(line_end - addr)]);
        } else {
            error!("    {:#x}: <unmapped>", addr);
        }
        addr += 16;
    }
}

#[cfg(target_arch = "riscv64")]
fn dump_user_bytes(tag: &str, page_table: &PageTable, addr: usize, bytes: usize) {
    let end = addr.saturating_add(bytes);
    let mut cur = addr;
    while cur < end {
        let mut line = [0u8; 16];
        let mut any = false;
        let line_end = core::cmp::min(cur + 16, end);
        for i in 0..(line_end - cur) {
            let va = cur + i;
            if let Some(pa) = page_table.translate_va(VirtAddr::from(va)) {
                line[i] = *pa.get_ref::<u8>();
                any = true;
            }
        }
        if any {
            error!("  {} {:#x}: {:02x?}", tag, cur, &line[..(line_end - cur)]);
        } else {
            error!("  {} {:#x}: <unmapped>", tag, cur);
        }
        cur += 16;
    }
}

/// Main user-mode trap handler.
///
/// For RISC-V 64, this is called from `trap.S` via the `TrapContext.trap_handler`
/// function pointer.  It reads the hardware CSRs (scause/stval), dispatches the
/// event (syscall, page fault, timer, etc.), and then returns to user space.
#[cfg(target_arch = "riscv64")]
#[no_mangle]
pub fn trap_handler() -> ! {
    use riscv::register::{
        scause::{self, Exception, Interrupt, Trap},
        stval,
    };

    arch::trap_init(); // set_kernel_trap_entry
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            let mut cx = current_trap_cx();
            cx.sepc += 4;
            // Enable S-mode interrupts during syscall processing
            unsafe { riscv::register::sstatus::set_sie(); }
            let result = syscall(cx[TrapFrameArgs::SYSCALL], cx.args());
            cx = current_trap_cx();
            cx[TrapFrameArgs::RET] = result as usize;
        }
        Trap::Exception(Exception::StoreFault)
        | Trap::Exception(Exception::StorePageFault)
        | Trap::Exception(Exception::InstructionFault)
        | Trap::Exception(Exception::InstructionPageFault)
        | Trap::Exception(Exception::LoadFault)
        | Trap::Exception(Exception::LoadPageFault) => {
            let trap_cx = current_trap_cx();
            let proc = current_process();
            let proc_inner = proc.inner_exclusive_access();
            let pid = proc.pid.0;
            let name = proc_inner.name.clone();
            drop(proc_inner);
            error!("[kernel] trap_handler: {:?} in application", scause.cause());
            error!("  pid={} name={}", pid, name);
            error!("  bad addr (stval) = {:#x}", stval);
            error!("  bad instruction (sepc) = {:#x}", trap_cx.sepc);
            let token = current_user_token();
            let page_table = PageTable::from_token(token);
            let stval_va = VirtAddr::from(stval as usize);
            if let Some(pte) = page_table.translate(stval_va.floor()) {
                if pte.is_valid() {
                    let offset = stval_va.page_offset();
                    let end = core::cmp::min(offset + 8, PAGE_SIZE);
                    let bytes = &pte.ppn().get_bytes_array()[offset..end];
                    error!(
                        "  stval pte: ppn={:#x} flags={:?} bytes={:02x?}",
                        pte.ppn().0,
                        pte.flags(),
                        bytes
                    );
                } else {
                    error!("  stval pte: invalid flags={:?}", pte.flags());
                }
            } else {
                error!("  stval pte: unmapped");
            }
            let sepc_va = VirtAddr::from(trap_cx.sepc);
            if let Some(pte) = page_table.translate(sepc_va.floor()) {
                if pte.is_valid() {
                    let offset = sepc_va.page_offset();
                    let end = core::cmp::min(offset + 8, PAGE_SIZE);
                    let bytes = &pte.ppn().get_bytes_array()[offset..end];
                    error!(
                        "  sepc pte: ppn={:#x} flags={:?} bytes={:02x?}",
                        pte.ppn().0,
                        pte.flags(),
                        bytes
                    );
                } else {
                    error!("  sepc pte: invalid flags={:?}", pte.flags());
                }
            } else {
                error!("  sepc pte: unmapped");
            }
            let args = trap_cx.args();
            error!("  Registers:");
            error!("    ra (x1) = {:#x}", trap_cx[TrapFrameArgs::RA]);
            error!("    sp (x2) = {:#x}", trap_cx[TrapFrameArgs::SP]);
            error!("    gp (x3) = {:#x}", trap_cx.gp());
            error!("    tp (x4) = {:#x}", trap_cx[TrapFrameArgs::TLS]);
            error!("    t0 (x5) = {:#x}", trap_cx.t0());
            error!("    t1 (x6) = {:#x}", trap_cx.t1());
            error!("    a0 (x10) = {:#x}", args[0]);
            error!("    a1 (x11) = {:#x}", args[1]);
            error!("    a2 (x12) = {:#x}", args[2]);
            error!("    a3 (x13) = {:#x}", args[3]);
            error!("    a4 (x14) = {:#x}", args[4]);
            error!("    a5 (x15) = {:#x}", args[5]);
            dump_user_stack(&page_table, trap_cx[TrapFrameArgs::SP], 128);
            if name == "entry-static.exe" {
                let tp = trap_cx[TrapFrameArgs::TLS];
                error!("  tp dump: tp={:#x}", tp);
                let base = tp.saturating_sub(256);
                dump_user_bytes("tp-0x100", &page_table, base, 128);
                dump_user_bytes("tp-0x80", &page_table, tp.saturating_sub(128), 128);
                dump_user_bytes("tp+0x0", &page_table, tp, 64);
            }
            if name == "busybox" || name == "ld-linux-riscv64-lp64d.so.1" {
                let path = if name == "busybox" {
                    "/musl/busybox"
                } else {
                    "/lib/ld-linux-riscv64-lp64d.so.1"
                };
                if let Some(file) = open_file(path, OpenFlags::empty()) {
                    let data = file.read_all();
                    if let Ok(elf) = ElfFile::new(data.as_slice()) {
                        let elf_type = elf.header.pt2.type_().as_type();
                        let mut has_interp = false;
                        let ph_count = elf.header.pt2.ph_count();
                        for i in 0..ph_count {
                            let ph = elf.program_header(i).unwrap();
                            if ph.get_type().unwrap() == xmas_elf::program::Type::Interp {
                                has_interp = true;
                                break;
                            }
                        }
                        let load_base = if elf_type == xmas_elf::header::Type::SharedObject && !has_interp {
                            0x4000_0000usize
                        } else {
                            0
                        };
                        let mut found = false;
                        for i in 0..ph_count {
                            let ph = elf.program_header(i).unwrap();
                            if ph.get_type().unwrap() != xmas_elf::program::Type::Load {
                                continue;
                            }
                            let vaddr = load_base + ph.virtual_addr() as usize;
                            let memsz = ph.mem_size() as usize;
                            if trap_cx.sepc < vaddr || trap_cx.sepc >= vaddr.saturating_add(memsz) {
                                continue;
                            }
                            let filesz = ph.file_size() as usize;
                            let file_off = ph.offset() as usize + trap_cx.sepc.saturating_sub(vaddr);
                            let end = (file_off + 8).min(data.len());
                            if file_off < end && file_off < ph.offset() as usize + filesz {
                                error!("  file bytes @sepc={:02x?}", &data[file_off..end]);
                                error!("  file off @sepc={:#x}", file_off);
                            } else {
                                error!("  file bytes @sepc: out of file range");
                            }
                            found = true;
                            break;
                        }
                        if !found {
                            error!("  file bytes @sepc: sepc not in PT_LOAD");
                        }
                    } else {
                        error!("  file bytes @sepc: invalid ELF");
                    }
                } else {
                    error!("  file bytes @sepc: {} not found", path);
                }
            }
            current_add_signal(SignalFlags::SIGSEGV);
        }
        Trap::Exception(Exception::IllegalInstruction) => {
            let trap_cx = current_trap_cx();
            error!("[kernel] trap_handler: IllegalInstruction in application");
            error!("  bad addr (stval) = {:#x}", stval);
            error!("  bad instruction (sepc) = {:#x}", trap_cx.sepc);
            let token = current_user_token();
            let page_table = PageTable::from_token(token);
            let sepc_va = VirtAddr::from(trap_cx.sepc);
            if let Some(pte) = page_table.translate(sepc_va.floor()) {
                if pte.is_valid() {
                    let offset = sepc_va.page_offset();
                    let end = core::cmp::min(offset + 8, PAGE_SIZE);
                    let bytes = &pte.ppn().get_bytes_array()[offset..end];
                    error!(
                        "  sepc pte: ppn={:#x} flags={:?} bytes={:02x?}",
                        pte.ppn().0,
                        pte.flags(),
                        bytes
                    );
                } else {
                    error!("  sepc pte: invalid flags={:?}", pte.flags());
                }
            } else {
                error!("  sepc pte: unmapped");
            }
            let name = current_process().inner_exclusive_access().name.clone();
            if name == "busybox" || name == "ld-linux-riscv64-lp64d.so.1" {
                let path = if name == "busybox" {
                    "/musl/busybox"
                } else {
                    "/lib/ld-linux-riscv64-lp64d.so.1"
                };
                if let Some(file) = open_file(path, OpenFlags::empty()) {
                    let data = file.read_all();
                    if let Ok(elf) = ElfFile::new(data.as_slice()) {
                        let elf_type = elf.header.pt2.type_().as_type();
                        let mut has_interp = false;
                        let ph_count = elf.header.pt2.ph_count();
                        for i in 0..ph_count {
                            let ph = elf.program_header(i).unwrap();
                            if ph.get_type().unwrap() == xmas_elf::program::Type::Interp {
                                has_interp = true;
                                break;
                            }
                        }
                        let load_base = if elf_type == xmas_elf::header::Type::SharedObject && !has_interp {
                            0x4000_0000usize
                        } else {
                            0
                        };
                        let mut found = false;
                        for i in 0..ph_count {
                            let ph = elf.program_header(i).unwrap();
                            if ph.get_type().unwrap() != xmas_elf::program::Type::Load {
                                continue;
                            }
                            let vaddr = load_base + ph.virtual_addr() as usize;
                            let memsz = ph.mem_size() as usize;
                            if trap_cx.sepc < vaddr || trap_cx.sepc >= vaddr.saturating_add(memsz) {
                                continue;
                            }
                            let filesz = ph.file_size() as usize;
                            let file_off = ph.offset() as usize + trap_cx.sepc.saturating_sub(vaddr);
                            let end = (file_off + 8).min(data.len());
                            if file_off < end && file_off < ph.offset() as usize + filesz {
                                error!("  file bytes @sepc={:02x?}", &data[file_off..end]);
                                error!("  file off @sepc={:#x}", file_off);
                            } else {
                                error!("  file bytes @sepc: out of file range");
                            }
                            found = true;
                            break;
                        }
                        if !found {
                            error!("  file bytes @sepc: sepc not in PT_LOAD");
                        }
                    } else {
                        error!("  file bytes @sepc: invalid ELF");
                    }
                } else {
                    error!("  file bytes @sepc: {} not found", path);
                }
            }
            current_add_signal(SignalFlags::SIGILL);
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            set_next_trigger();
            check_timer();

            // Ensure pending signals are handled even if we preempt from user mode.
            if let Some(task) = current_task() {
                let task_inner = task.inner_exclusive_access();
                if task_inner.signal_pending.contains(SignalFlags::SIG33) {
                    let pid = current_process().pid.0;
                    let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
                    let tp = task_inner.get_trap_cx()[TrapFrameArgs::TLS];
                    info!(
                        "[trap-timer] pid={} tid={} sig33 pending tp={:#x} mask={:?}",
                        pid,
                        tid,
                        tp,
                        task_inner.signal_mask
                    );
                }
            }
            handle_signals();

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
                            "[sample] pid={} name={} sepc={:#x} sp={:#x} ra={:#x}",
                            pid, name, sepc, sp, ra
                        );
                    }
                }
            }
            suspend_current_and_run_next();
        }
        Trap::Interrupt(Interrupt::SupervisorExternal) => {
            crate::board::irq_handler();
        }
        _ => {
            panic!(
                "Unsupported trap {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
    // Before returning to user, check for pending signals.
    if let Some(task) = current_task() {
        let task_inner = task.inner_exclusive_access();
        if task_inner.signal_pending.contains(SignalFlags::SIG33) {
            let pid = current_process().pid.0;
            let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
            let tp = task_inner.get_trap_cx()[TrapFrameArgs::TLS];
            info!(
                "[trap] pid={} tid={} sig33 pending tp={:#x} mask={:?}",
                pid,
                tid,
                tp,
                task_inner.signal_mask
            );
        }
    }
    handle_signals();
    do_trap_return();
}

// ---------------------------------------------------------------------------
// User-mode trap entry loop (LoongArch64)
//
// On LoongArch64 the kernel enters user mode via a loop: restore user
// registers -> ertn -> (user runs) -> trap -> user_vec returns here.
// This replaces the RISC-V approach of calling `trap_return()` which
// jumps to the trampoline page.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "loongarch64")]
pub fn task_entry() {
    use arch::TrapType;
    loop {
        let user_token = current_user_token();
        arch::activate_page_table(user_token);
        // user_restore enters user mode; user_vec returns here after trap
        let trap_cx_ptr = current_trap_cx() as *mut arch::TrapContext;
        arch::user_restore(trap_cx_ptr);

        // Handle the trap
        let trap_cx = current_trap_cx();
        let estat = loongArch64::register::estat::read();
        let trap_type = match estat.cause() {
            loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::Syscall,
            ) => TrapType::UserEnvCall,
            loongArch64::register::estat::Trap::Interrupt(_) => {
                let irq_num = estat.is().trailing_zeros() as usize;
                if irq_num == 11 {
                    loongArch64::register::ticlr::clear_timer_interrupt();
                    TrapType::Time
                } else {
                    TrapType::Unknown
                }
            }
            loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::StorePageFault,
            )
            | loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::PagePrivilegeIllegal,
            )
            | loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::PageModifyFault,
            ) => TrapType::StorePageFault(loongArch64::register::badv::read().raw()),
            loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::LoadPageFault,
            )
            | loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::FetchPageFault,
            ) => TrapType::LoadPageFault(loongArch64::register::badv::read().raw()),
            loongArch64::register::estat::Trap::Exception(
                loongArch64::register::estat::Exception::InstructionNotExist,
            ) => TrapType::IllegalInstruction(loongArch64::register::badv::read().raw()),
            _ => TrapType::Unknown,
        };

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
                current_add_signal(SignalFlags::SIGSEGV);
                // ! 暂时直接 shutdown，后续可以考虑杀死进程
                use arch::shutdown;
                shutdown();

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
            _ => {
                warn!("[kernel] trap_handler: unknown trap at sepc={:#x}", trap_cx.sepc);
            }
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

/// Return to user space via the architecture-specific `trap_return`.
pub fn do_trap_return() -> ! {
    let trap_cx_ptr = current_trap_cx_user_va();
    let user_satp = current_user_token();
    arch::trap_return(trap_cx_ptr, user_satp);
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
