use super::*;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct LoongArchMContext {
    pub pc: usize,
    pub gregs: [usize; 32],
    pub flags: u32,
    pub _pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    pub uc_flags: usize,
    pub uc_link: usize,
    pub uc_stack: StackT,
    pub uc_sigmask: [u64; LINUX_SIGSET_WORDS],
    pub uc_mcontext: LoongArchMContext,
}

impl UserContext {
    pub fn from_trap(trap_cx: &TrapContext, sigmask: SignalFlags) -> Self {
        let mut user_sigset = [0u64; LINUX_SIGSET_WORDS];
        user_sigset[0] = signal::flags_to_user_mask(sigmask);
        Self {
            uc_flags: 0,
            uc_link: 0,
            uc_stack: StackT {
                ss_sp: 0,
                ss_flags: 0,
                _pad: 0,
                ss_size: 0,
            },
            uc_sigmask: user_sigset,
            uc_mcontext: LoongArchMContext {
                pc: trap_cx.sepc,
                gregs: trap_cx.x,
                flags: 0,
                _pad: 0,
            },
        }
    }

    pub fn signal_mask_word0(&self) -> u64 {
        self.uc_sigmask[0]
    }

    pub fn user_pc(&self) -> usize {
        self.uc_mcontext.pc
    }

    pub fn restore_trap_context(&self, trap_cx: &mut TrapContext) {
        let mut restored_pc = self.uc_mcontext.pc;
        let gpr0_pc = self.uc_mcontext.gregs[0];
        if gpr0_pc != 0 && gpr0_pc != restored_pc {
            trace!(
                "[sigreturn-compat] loongarch use gregs[0] as pc: pc={:#x} gpr0={:#x}",
                restored_pc,
                gpr0_pc
            );
            restored_pc = gpr0_pc;
        }
        trap_cx.sepc = restored_pc;
        trap_cx.x = self.uc_mcontext.gregs;
        trap_cx.x[0] = 0;
    }
}
