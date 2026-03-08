#![allow(missing_docs)]
#![allow(dead_code)]
#![allow(unused_imports)]

pub mod console;
pub mod consts;
pub mod context;
pub mod entry;
pub mod kcontext;
pub mod page_table;
pub mod sbi;
pub mod sigtrx;
pub mod signal;
pub mod timer;
pub mod trap;
pub mod unaligned;

#[derive(Debug, Clone, Copy)]
pub enum KContextArgs {
	KSP,
	KTP,
	KPC,
}

#[derive(Debug, Clone, Copy)]
pub enum TrapFrameArgs {
	SEPC,
	RA,
	SP,
	RET,
	ARG0,
	ARG1,
	ARG2,
	TLS,
	SYSCALL,
}

#[derive(Debug, Clone, Copy)]
pub enum TrapType {
	Breakpoint,
	UserEnvCall,
	Time,
	Unknown,
	SupervisorExternal,
	StorePageFault(usize),
	LoadPageFault(usize),
	InstructionPageFault(usize),
	IllegalInstruction(usize),
}

pub use consts::*;
pub use context::TrapFrame;
pub use kcontext::{context_switch, context_switch_pt, read_current_tp, KContext};
pub use page_table::*;
pub use sbi::shutdown;
pub use timer::{init_timer, Time};

pub use trap::{
	disable_irq, enable_external_irq, enable_irq, init_interrupt, run_user_task,
	set_kernel_trap, set_kernel_user_rw_trap, set_trap_vector_base, try_read_user,
	try_write_user,
};

pub use trap::{trap_enable_timer_interrupt, trap_handler, trap_init, trap_return};

pub type TrapContext = TrapFrame;
pub type TaskContext = KContext;
pub use kcontext::context_switch as __switch;
