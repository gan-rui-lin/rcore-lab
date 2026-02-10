#![allow(missing_docs)]
use bitflags::*;

pub const MAX_SIG: usize = 31;

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
    pub struct SignalFlags: u32 {
        const SIGDEF = 1; // Default signal handling
        const SIGHUP = 1 << SigNumber::SigHup as u32; // Hangup
        const SIGINT = 1 << SigNumber::SigInt as u32; // Interrupt
        const SIGQUIT = 1 << SigNumber::SigQuit as u32; // Quit
        const SIGILL = 1 << SigNumber::SigIll as u32; // Illegal instruction
        const SIGTRAP = 1 << SigNumber::SigTrap as u32; // Trace/breakpoint trap
        const SIGABRT = 1 << SigNumber::SigAbrt as u32; // Abort
        const SIGBUS = 1 << SigNumber::SigBus as u32; // Bus error
        const SIGFPE = 1 << SigNumber::SigFpe as u32; // Floating-point exception
        const SIGKILL = 1 << SigNumber::SigKill as u32; // Kill
        const SIGUSR1 = 1 << SigNumber::SigUsr1 as u32; // User-defined signal 1
        const SIGSEGV = 1 << SigNumber::SigSegv as u32; // Segmentation fault
        const SIGUSR2 = 1 << SigNumber::SigUsr2 as u32; // User-defined signal 2
        const SIGPIPE = 1 << SigNumber::SigPipe as u32; // Broken pipe
        const SIGALRM = 1 << SigNumber::SigAlrm as u32; // Alarm clock
        const SIGTERM = 1 << SigNumber::SigTerm as u32; // Termination signal
        const SIGSTKFLT = 1 << SigNumber::SigStkflt as u32; // Stack fault
        const SIGCHLD = 1 << SigNumber::SigChld as u32; // Child stopped or terminated
        const SIGCONT = 1 << SigNumber::SigCont as u32; // Continue if stopped
        const SIGSTOP = 1 << SigNumber::SigStop as u32; // Stop process
        const SIGTSTP = 1 << SigNumber::SigTstp as u32; // Terminal stop signal
        const SIGTTIN = 1 << SigNumber::SigTtin as u32; // Background process attempting read
        const SIGTTOU = 1 << SigNumber::SigTtou as u32; // Background process attempting write
        const SIGURG = 1 << SigNumber::SigUrg as u32; // Urgent condition on socket
        const SIGXCPU = 1 << SigNumber::SigXcpu as u32; // CPU time limit exceeded
        const SIGXFSZ = 1 << SigNumber::SigXfsz as u32; // File size limit exceeded
        const SIGVTALRM = 1 << SigNumber::SigVtalrm as u32; // Virtual alarm clock
        const SIGPROF = 1 << SigNumber::SigProf as u32; // Profiling timer expired
        const SIGWINCH = 1 << SigNumber::SigWinch as u32; // Window size change
        const SIGIO = 1 << SigNumber::SigIo as u32; // I/O now possible
        const SIGPWR = 1 << SigNumber::SigPwr as u32; // Power failure
        const SIGSYS = 1 << SigNumber::SigSys as u32; // Bad system call
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
