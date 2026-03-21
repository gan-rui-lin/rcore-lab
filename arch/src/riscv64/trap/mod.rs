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
    fn riscv_user_enter(context: *mut u8, user_satp: usize);
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
pub fn enter_user_and_trap(context: &mut context::TrapContext, user_satp: usize) -> TrapType {
    let enter_high = if (riscv_user_enter as usize) >= crate::VIRT_ADDR_START {
        riscv_user_enter as usize
    } else {
        (riscv_user_enter as usize) | crate::VIRT_ADDR_START
    };
    let enter_fn: extern "C" fn(*mut u8, usize) = unsafe { core::mem::transmute(enter_high) };
    enter_fn((context as *mut _) as *mut u8, user_satp);
    classify_current_trap()
}

/// Called from `trap.S` for traps taken while already in supervisor mode.
#[no_mangle]
extern "C" fn kernel_trap_dispatch(_trap_cx: &context::KernelTrapContext) {
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
