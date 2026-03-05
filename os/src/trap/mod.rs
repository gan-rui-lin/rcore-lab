//! Trap handling functionality
//!
//! For rCore, we have a single trap entry point, namely `__alltraps`. At
//! initialization in [`init()`], we set the `stvec` CSR to point to it.
//!
//! All traps go through `__alltraps`, which is defined in `trap.S`. The
//! assembly language code does just enough work restore the kernel space
//! context, ensuring that Rust code safely runs, and transfers control to
//! [`trap_handler()`].
//!
//! It then calls different functionality based on what exactly the exception
//! was. For example, timer interrupts trigger task preemption, and syscalls go
//! to [`syscall()`].

mod context;

use crate::config::TRAMPOLINE;
use crate::syscall::syscall;
use crate::task::{
    current_add_signal, current_process, current_task, current_trap_cx, current_trap_cx_user_va,
    current_user_token, handle_signals, suspend_current_and_run_next, SignalFlags,
};
use crate::mm::{PageTable, VirtAddr};
use crate::config::PAGE_SIZE;
use crate::fs::{open_file, OpenFlags};
use xmas_elf::ElfFile;
use crate::timer::{check_timer, set_next_trigger};
use core::arch::{asm, global_asm};
use core::sync::atomic::{AtomicU64, Ordering};
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, sscratch, sstatus, stval, stvec,
};

const TIMER_SAMPLE_INTERVAL: u64 = 200;
static TIMER_SAMPLE_COUNTER: AtomicU64 = AtomicU64::new(0);

global_asm!(include_str!("trap.S"));

/// Initialize trap handling
pub fn init() {
    set_kernel_trap_entry();
}

fn set_kernel_trap_entry() {
    extern "C" {
        fn __alltraps();
        fn __alltraps_k();
    }
    let __alltraps_k_va = __alltraps_k as usize - __alltraps as usize + TRAMPOLINE;
    unsafe {
        stvec::write(__alltraps_k_va, TrapMode::Direct);
        sscratch::write(trap_from_kernel as usize);
    }
}

fn set_user_trap_entry() {
    unsafe {
        stvec::write(TRAMPOLINE as usize, TrapMode::Direct);
    }
}

/// enable timer interrupt in supervisor mode
pub fn enable_timer_interrupt() {
    unsafe {
        sie::set_stimer();
    }
}

fn enable_supervisor_interrupt() {
    unsafe {
        sstatus::set_sie();
    }
}

fn disable_supervisor_interrupt() {
    unsafe {
        sstatus::clear_sie();
    }
}

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

/// trap handler
#[no_mangle]
pub fn trap_handler() -> ! {
    set_kernel_trap_entry();
    let scause = scause::read();
    let stval = stval::read();
    // trace!("into {:?}", scause.cause());
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => {
            // jump to next instruction anyway
            let mut cx = current_trap_cx();
            cx.sepc += 4;
            enable_supervisor_interrupt();
            // get system call return value
            let result = syscall(
                cx.x[17],
                [cx.x[10], cx.x[11], cx.x[12], cx.x[13], cx.x[14], cx.x[15]],
            );
            // cx is changed during sys_exec, so we have to call it again
            cx = current_trap_cx();
            cx.x[10] = result as usize;
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
            error!("  Registers:");
            error!("    ra (x1) = {:#x}", trap_cx.x[1]);
            error!("    sp (x2) = {:#x}", trap_cx.x[2]);
            error!("    gp (x3) = {:#x}", trap_cx.x[3]);
            error!("    tp (x4) = {:#x}", trap_cx.x[4]);
            error!("    t0 (x5) = {:#x}", trap_cx.x[5]);
            error!("    t1 (x6) = {:#x}", trap_cx.x[6]);
            error!("    a0 (x10) = {:#x}", trap_cx.x[10]);
            error!("    a1 (x11) = {:#x}", trap_cx.x[11]);
            error!("    a2 (x12) = {:#x}", trap_cx.x[12]);
            error!("    a3 (x13) = {:#x}", trap_cx.x[13]);
            error!("    a4 (x14) = {:#x}", trap_cx.x[14]);
            error!("    a5 (x15) = {:#x}", trap_cx.x[15]);
            dump_user_stack(&page_table, trap_cx.x[2], 128);
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
            // for debug
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
            // Without this, pure user-space loops may never process SIGCANCEL.
            if let Some(task) = current_task() {
                let task_inner = task.inner_exclusive_access();
                if task_inner.signal_pending.contains(SignalFlags::SIG33) {
                    let pid = current_process().pid.0;
                    let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
                    let tp = task_inner.get_trap_cx().x[4];
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
                            (trap_cx.sepc, trap_cx.x[2], trap_cx.x[1])
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
    //println!("before trap_return");
    if let Some(task) = current_task() {
        let task_inner = task.inner_exclusive_access();
        if task_inner.signal_pending.contains(SignalFlags::SIG33) {
            let pid = current_process().pid.0;
            let tid = task_inner.res.as_ref().map(|r| r.tid).unwrap_or(0);
            let tp = task_inner.get_trap_cx().x[4];
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
    trap_return();
}

#[no_mangle]
/// return to user space
/// set the new addr of __restore asm function in TRAMPOLINE page,
/// set the reg a0 = trap_cx_ptr, reg a1 = phy addr of usr page table,
/// finally, jump to new addr of __restore asm function
pub fn trap_return() -> ! {
    disable_supervisor_interrupt();
    set_user_trap_entry();
    let trap_cx_ptr = current_trap_cx_user_va();
    let user_satp = current_user_token();
    extern "C" {
        fn __alltraps();
        fn __restore();
    }
    let restore_va = __restore as usize - __alltraps as usize + TRAMPOLINE;
    // trace!("[kernel] trap_return: ..before return");
    unsafe {
        asm!(
            "fence.i",
            "jr {restore_va}",
            restore_va = in(reg) restore_va,
            in("a0") trap_cx_ptr,
            in("a1") user_satp,
            options(noreturn)
        );
    }
}

/// Debug helper for gdb: translate a user virtual address to physical address.
/// Returns 0 if unmapped.
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

#[no_mangle]
/// handle trap from kernel
/// Unimplement: traps/interrupts/exceptions from kernel mode
/// Todo: Chapter 9: I/O device
fn trap_from_kernel(_trap_cx: &context::KernelTrapContext) {
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Interrupt(Interrupt::SupervisorExternal) => {
            crate::board::irq_handler();
        }
        Trap::Interrupt(Interrupt::SupervisorTimer) => {
            // 内核里可能正持有锁、在临界区或使用内核栈/中断屏蔽状态不一致，此时直接调度切换任务风险很大
            // 先设置下次触发时间，等到返回用户态后再检查定时器并切换任务
            set_next_trigger();
            check_timer();

            // SIGCANCEL 循环检测已在 handle_signals()/sys_sigreturn() 中处理

            let count = TIMER_SAMPLE_COUNTER.fetch_add(1, Ordering::Relaxed);
            if count % TIMER_SAMPLE_INTERVAL == 0 {
                if let Some(task) = current_task() {
                    if let Some(process) = task.process.upgrade() {
                        let pid = process.pid.0;
                        let name = process.inner_exclusive_access().name.clone();
                        let (sepc, sp, ra) = {
                            let task_inner = task.inner_exclusive_access();
                            let trap_cx = task_inner.get_trap_cx();
                            (trap_cx.sepc, trap_cx.x[2], trap_cx.x[1])
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
            panic!(
                "Unsupported trap from kernel: {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    }
}

pub use context::TrapContext;
