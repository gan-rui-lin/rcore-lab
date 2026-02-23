#![allow(missing_docs)]
use bitflags::*;

pub const MAX_SIG: usize = 64;

pub enum SigNumber {
    SigHup = 1,
    SigInt = 2,
    SigQuit = 3,
    SigIll = 4,
    SigTrap = 5,
    SigAbrt = 6,
    SigBus = 7,
    SigFpe = 8,
    SigKill = 9,
    SigUsr1 = 10,
    SigSegv = 11,
    SigUsr2 = 12,
    SigPipe = 13,
    SigAlrm = 14,
    SigTerm = 15,
    SigStkflt = 16,
    SigChld = 17,
    SigCont = 18,
    SigStop = 19,
    SigTstp = 20,
    SigTtin = 21,
    SigTtou = 22,
    SigUrg = 23,
    SigXcpu = 24,
    SigXfsz = 25,
    SigVtalrm = 26,
    SigProf = 27,
    SigWinch = 28,
    SigIo = 29,
    SigPwr = 30,
    SigSys = 31,
}

bitflags! {
    pub struct SignalFlags: u64 {
        const SIGDEF = 1u64; // Default signal handling
        const SIGHUP = 1u64 << SigNumber::SigHup as u64; // Hangup
        const SIGINT = 1u64 << SigNumber::SigInt as u64; // Interrupt
        const SIGQUIT = 1u64 << SigNumber::SigQuit as u64; // Quit
        const SIGILL = 1u64 << SigNumber::SigIll as u64; // Illegal instruction
        const SIGTRAP = 1u64 << SigNumber::SigTrap as u64; // Trace/breakpoint trap
        const SIGABRT = 1u64 << SigNumber::SigAbrt as u64; // Abort
        const SIGBUS = 1u64 << SigNumber::SigBus as u64; // Bus error
        const SIGFPE = 1u64 << SigNumber::SigFpe as u64; // Floating-point exception
        const SIGKILL = 1u64 << SigNumber::SigKill as u64; // Kill
        const SIGUSR1 = 1u64 << SigNumber::SigUsr1 as u64; // User-defined signal 1
        const SIGSEGV = 1u64 << SigNumber::SigSegv as u64; // Segmentation fault
        const SIGUSR2 = 1u64 << SigNumber::SigUsr2 as u64; // User-defined signal 2
        const SIGPIPE = 1u64 << SigNumber::SigPipe as u64; // Broken pipe
        const SIGALRM = 1u64 << SigNumber::SigAlrm as u64; // Alarm clock
        const SIGTERM = 1u64 << SigNumber::SigTerm as u64; // Termination signal
        const SIGSTKFLT = 1u64 << SigNumber::SigStkflt as u64; // Stack fault
        const SIGCHLD = 1u64 << SigNumber::SigChld as u64; // Child stopped or terminated
        const SIGCONT = 1u64 << SigNumber::SigCont as u64; // Continue if stopped
        const SIGSTOP = 1u64 << SigNumber::SigStop as u64; // Stop process
        const SIGTSTP = 1u64 << SigNumber::SigTstp as u64; // Terminal stop signal
        const SIGTTIN = 1u64 << SigNumber::SigTtin as u64; // Background process attempting read
        const SIGTTOU = 1u64 << SigNumber::SigTtou as u64; // Background process attempting write
        const SIGURG = 1u64 << SigNumber::SigUrg as u64; // Urgent condition on socket
        const SIGXCPU = 1u64 << SigNumber::SigXcpu as u64; // CPU time limit exceeded
        const SIGXFSZ = 1u64 << SigNumber::SigXfsz as u64; // File size limit exceeded
        const SIGVTALRM = 1u64 << SigNumber::SigVtalrm as u64; // Virtual alarm clock
        const SIGPROF = 1u64 << SigNumber::SigProf as u64; // Profiling timer expired
        const SIGWINCH = 1u64 << SigNumber::SigWinch as u64; // Window size change
        const SIGIO = 1u64 << SigNumber::SigIo as u64; // I/O now possible
        const SIGPWR = 1u64 << SigNumber::SigPwr as u64; // Power failure
        const SIGSYS = 1u64 << SigNumber::SigSys as u64; // Bad system call
        const SIG32 = 1u64 << 32;
        const SIG33 = 1u64 << 33;
        const SIG34 = 1u64 << 34;
        const SIG35 = 1u64 << 35;
        const SIG36 = 1u64 << 36;
        const SIG37 = 1u64 << 37;
        const SIG38 = 1u64 << 38;
        const SIG39 = 1u64 << 39;
        const SIG40 = 1u64 << 40;
        const SIG41 = 1u64 << 41;
        const SIG42 = 1u64 << 42;
        const SIG43 = 1u64 << 43;
        const SIG44 = 1u64 << 44;
        const SIG45 = 1u64 << 45;
        const SIG46 = 1u64 << 46;
        const SIG47 = 1u64 << 47;
        const SIG48 = 1u64 << 48;
        const SIG49 = 1u64 << 49;
        const SIG50 = 1u64 << 50;
        const SIG51 = 1u64 << 51;
        const SIG52 = 1u64 << 52;
        const SIG53 = 1u64 << 53;
        const SIG54 = 1u64 << 54;
        const SIG55 = 1u64 << 55;
        const SIG56 = 1u64 << 56;
        const SIG57 = 1u64 << 57;
        const SIG58 = 1u64 << 58;
        const SIG59 = 1u64 << 59;
        const SIG60 = 1u64 << 60;
        const SIG61 = 1u64 << 61;
        const SIG62 = 1u64 << 62;
        const SIG63 = 1u64 << 63;
    }
}


impl SignalFlags {
    pub fn check_error(&self) -> Option<(i32, &'static str)> {
        if self.contains(Self::SIGINT) {
            Some((-2, "Killed, SIGINT=2"))
        } else if self.contains(Self::SIGILL) {
            Some((-4, "Illegal Instruction, SIGILL=4"))
        } else if self.contains(Self::SIGABRT) {
            Some((-6, "Aborted, SIGABRT=6"))
        } else if self.contains(Self::SIGFPE) {
            Some((-8, "Erroneous Arithmetic Operation, SIGFPE=8"))
        } else if self.contains(Self::SIGKILL) {
            Some((-9, "Killed, SIGKILL=9"))
        } else if self.contains(Self::SIGSEGV) {
            Some((-11, "Segmentation Fault, SIGSEGV=11"))
        } else {
            //println!("[K] signalflags check_error  {:?}", self);
            None
        }
    }
}

pub fn flags_to_user_mask(flags: SignalFlags) -> u64 {
    let mut user_mask = 0u64;
    for signum in 1..=MAX_SIG {
        let flag = match 1u64.checked_shl(signum as u32) {
            Some(bits) => SignalFlags::from_bits_truncate(bits),
            None => continue,
        };
        if flags.contains(flag) {
            user_mask |= 1u64 << (signum - 1);
        }
    }
    user_mask
}

pub fn user_mask_to_flags(user_mask: u64) -> SignalFlags {
    let mut flags = SignalFlags::empty();
    for signum in 1..=MAX_SIG {
        if (user_mask & (1u64 << (signum - 1))) == 0 {
            continue;
        }
        let flag = match 1u64.checked_shl(signum as u32) {
            Some(bits) => SignalFlags::from_bits_truncate(bits),
            None => continue,
        };
        flags |= flag;
    }
    flags
}
