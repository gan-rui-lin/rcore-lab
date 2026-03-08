#[cfg(target_arch = "riscv64")]
pub mod riscv64;

#[cfg(target_arch = "loongarch64")]
pub mod loongarch64;

#[cfg(target_arch = "riscv64")]
#[allow(unused_imports)]
pub use riscv64::{
	TrapContext, trap_handler, trap_return, trap_init, trap_enable_timer_interrupt, shutdown,
	TaskContext, __switch,
};

#[cfg(target_arch = "loongarch64")]
#[allow(unused_imports)]
pub use loongarch64::{
	TrapContext, trap_handler, trap_return, trap_init, trap_enable_timer_interrupt, shutdown,
	TaskContext, __switch, task_entry,
};
