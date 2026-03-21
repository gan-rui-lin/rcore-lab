use super::*;
use arch::{FpRegs, MContext};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UserContext {
    pub uc_flags: usize,
    pub uc_link: usize,
    pub uc_stack: StackT,
    pub uc_sigmask: [u64; LINUX_SIGSET_WORDS],
    pub uc_mcontext: MContext,
}

impl UserContext {
    pub fn from_trap(trap_cx: &TrapContext, sigmask: SignalFlags) -> Self {
        let mut gregs = [0usize; 32];
        trap_cx.write_ucontext_gregs(&mut gregs);
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
            uc_mcontext: MContext {
                gregs,
                fpregs: FpRegs {
                    f: [0u64; 32],
                    fcsr: 0,
                    _pad: 0,
                },
            },
        }
    }

    pub fn signal_mask_word0(&self) -> u64 {
        self.uc_sigmask[0]
    }

    pub fn user_pc(&self) -> usize {
        self.uc_mcontext.gregs[0]
    }

    pub fn restore_trap_context(&self, trap_cx: &mut TrapContext) {
        trap_cx.restore_from_ucontext_gregs(&self.uc_mcontext.gregs);
    }
}
