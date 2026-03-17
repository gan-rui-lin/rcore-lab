//! RISC-V trap handling -- architecture layer.
//!
//! This implementation uses `kernelvec/uservec` with `sscratch` stack
//! switching and does not depend on trampoline-page trap entry.

mod context;

use crate::api::ArchInterface;
use crate::TrapType;
use core::arch::global_asm;
use riscv::register::{
    mtvec::TrapMode,
    scause::{self, Exception, Interrupt, Trap},
    sie, sscratch, stval, stvec,
};

global_asm!(include_str!("trap.S"));

extern "C" {
    fn kernelvec();
    fn user_restore(context: *mut u8, user_satp: usize);
}

/// Initialize trap handling: point `stvec` at `kernelvec`.
pub fn init() {
    let kernelvec_high = if (kernelvec as usize) >= crate::VIRT_ADDR_START {
        kernelvec as usize
    } else {
        (kernelvec as usize) | crate::VIRT_ADDR_START
    };
    unsafe {
        stvec::write(kernelvec_high, TrapMode::Direct);
        sscratch::write(0);
    }
}

/// Enable the supervisor timer interrupt (sets `sie.STIE`).
pub fn enable_timer_interrupt() {
    unsafe {
        sie::set_stimer();
    }
}

/// Enter user mode and return once a trap from user mode occurs.
pub fn run_user_task(context: &mut context::TrapContext, user_satp: usize) -> TrapType {
    let restore_high = if (user_restore as usize) >= crate::VIRT_ADDR_START {
        user_restore as usize
    } else {
        (user_restore as usize) | crate::VIRT_ADDR_START
    };
    let restore_fn: extern "C" fn(*mut u8, usize) = unsafe { core::mem::transmute(restore_high) };
    restore_fn((context as *mut _) as *mut u8, user_satp);
    classify_current_trap()
}

/// Deprecated compatibility entry. RISC-V now returns to user through
/// `run_user_task` loop in kernel `task_entry`.
pub fn trap_return(_trap_cx_ptr: usize, _user_satp: usize) -> ! {
    panic!("trap_return() is obsolete on riscv64; use run_user_task loop")
}

/// Called from `trap.S` for traps taken while already in supervisor mode.
#[no_mangle]
extern "C" fn trap_from_kernel(_trap_cx: &context::KernelTrapContext) {
    let scause = scause::read();
    let stval = stval::read();

    let trap_type = match scause.cause() {
        Trap::Interrupt(Interrupt::SupervisorExternal) => TrapType::SupervisorExternal,
        Trap::Interrupt(Interrupt::SupervisorTimer) => TrapType::Time,
        Trap::Exception(Exception::Breakpoint) => TrapType::Breakpoint,
        _ => {
            error!(
                "[rv-ktrap] unsupported kernel trap: bits={:#x} cause={:?} stval={:#x} sepc={:#x}",
                scause.bits(),
                scause.cause(),
                stval,
                _trap_cx.sepc
            );
            panic!(
                "Unsupported trap from kernel: {:?}, stval = {:#x}!",
                scause.cause(),
                stval
            );
        }
    };

    ArchInterface::kernel_interrupt(trap_type);
}

fn classify_current_trap() -> TrapType {
    let scause = scause::read();
    let stval = stval::read();
    match scause.cause() {
        Trap::Exception(Exception::UserEnvCall) => TrapType::UserEnvCall,
        Trap::Exception(Exception::Breakpoint) => TrapType::Breakpoint,
        Trap::Interrupt(Interrupt::SupervisorTimer) => TrapType::Time,
        Trap::Interrupt(Interrupt::SupervisorExternal) => TrapType::SupervisorExternal,
        Trap::Exception(Exception::StoreFault) | Trap::Exception(Exception::StorePageFault) => {
            TrapType::StorePageFault(stval)
        }
        Trap::Exception(Exception::LoadFault) | Trap::Exception(Exception::LoadPageFault) => {
            TrapType::LoadPageFault(stval)
        }
        Trap::Exception(Exception::InstructionFault)
        | Trap::Exception(Exception::InstructionPageFault) => {
            TrapType::InstructionPageFault(stval)
        }
        Trap::Exception(Exception::IllegalInstruction) => TrapType::IllegalInstruction(stval),
        _ => TrapType::Unknown,
    }
}

pub use context::TrapContext;
