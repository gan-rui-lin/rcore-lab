#![allow(missing_docs)]
use bitflags::*;

pub const MAX_SIG: usize = 64;

/// 信号编号常量（遵循 POSIX 标准）
pub const SIGHUP: usize = 1;
pub const SIGINT: usize = 2;
pub const SIGQUIT: usize = 3;
pub const SIGILL: usize = 4;
pub const SIGTRAP: usize = 5;
pub const SIGABRT: usize = 6;
pub const SIGBUS: usize = 7;
pub const SIGFPE: usize = 8;
pub const SIGKILL: usize = 9;
pub const SIGUSR1: usize = 10;
pub const SIGSEGV: usize = 11;
pub const SIGUSR2: usize = 12;
pub const SIGPIPE: usize = 13;
pub const SIGALRM: usize = 14;
pub const SIGTERM: usize = 15;
pub const SIGSTKFLT: usize = 16;
pub const SIGCHLD: usize = 17;
pub const SIGCONT: usize = 18;
pub const SIGSTOP: usize = 19;
pub const SIGTSTP: usize = 20;
pub const SIGTTIN: usize = 21;
pub const SIGTTOU: usize = 22;
pub const SIGURG: usize = 23;
pub const SIGXCPU: usize = 24;
pub const SIGXFSZ: usize = 25;
pub const SIGVTALRM: usize = 26;
pub const SIGPROF: usize = 27;
pub const SIGWINCH: usize = 28;
pub const SIGIO: usize = 29;
pub const SIGPWR: usize = 30;
pub const SIGSYS: usize = 31;

/// 保留 SigNumber 枚举用于退出码（避免破坏其他代码）
#[allow(dead_code)]
pub enum SigNumber {
    SigKill = 9,
}

bitflags! {
    /// SignalFlags: 每个 bit 对应一个信号
    /// bit N 表示 signum = N+1
    /// 例如：bit 0 = SIGHUP(1), bit 8 = SIGKILL(9), bit 32 = signum 33
    pub struct SignalFlags: u64 {
        const SIGHUP = 1u64 << 0;  // signum 1
        const SIGINT = 1u64 << 1;  // signum 2
        const SIGQUIT = 1u64 << 2; // signum 3
        const SIGILL = 1u64 << 3;  // signum 4
        const SIGTRAP = 1u64 << 4; // signum 5
        const SIGABRT = 1u64 << 5; // signum 6
        const SIGBUS = 1u64 << 6;  // signum 7
        const SIGFPE = 1u64 << 7;  // signum 8
        const SIGKILL = 1u64 << 8; // signum 9
        const SIGUSR1 = 1u64 << 9; // signum 10
        const SIGSEGV = 1u64 << 10; // signum 11
        const SIGUSR2 = 1u64 << 11; // signum 12
        const SIGPIPE = 1u64 << 12; // signum 13
        const SIGALRM = 1u64 << 13; // signum 14
        const SIGTERM = 1u64 << 14; // signum 15
        const SIGSTKFLT = 1u64 << 15; // signum 16
        const SIGCHLD = 1u64 << 16; // signum 17
        const SIGCONT = 1u64 << 17; // signum 18
        const SIGSTOP = 1u64 << 18; // signum 19
        const SIGTSTP = 1u64 << 19; // signum 20
        const SIGTTIN = 1u64 << 20; // signum 21
        const SIGTTOU = 1u64 << 21; // signum 22
        const SIGURG = 1u64 << 22;  // signum 23
        const SIGXCPU = 1u64 << 23; // signum 24
        const SIGXFSZ = 1u64 << 24; // signum 25
        const SIGVTALRM = 1u64 << 25; // signum 26
        const SIGPROF = 1u64 << 26; // signum 27
        const SIGWINCH = 1u64 << 27; // signum 28
        const SIGIO = 1u64 << 28;   // signum 29
        const SIGPWR = 1u64 << 29;  // signum 30
        const SIGSYS = 1u64 << 30;  // signum 31
        const SIG32 = 1u64 << 31;   // signum 32
        const SIG33 = 1u64 << 32;   // signum 33 (SIGCANCEL)
        const SIG34 = 1u64 << 33;   // signum 34
        const SIG35 = 1u64 << 34;   // signum 35
        const SIG36 = 1u64 << 35;   // signum 36
        const SIG37 = 1u64 << 36;   // signum 37
        const SIG38 = 1u64 << 37;   // signum 38
        const SIG39 = 1u64 << 38;   // signum 39
        const SIG40 = 1u64 << 39;   // signum 40
        const SIG41 = 1u64 << 40;   // signum 41
        const SIG42 = 1u64 << 41;   // signum 42
        const SIG43 = 1u64 << 42;   // signum 43
        const SIG44 = 1u64 << 43;   // signum 44
        const SIG45 = 1u64 << 44;   // signum 45
        const SIG46 = 1u64 << 45;   // signum 46
        const SIG47 = 1u64 << 46;   // signum 47
        const SIG48 = 1u64 << 47;   // signum 48
        const SIG49 = 1u64 << 48;   // signum 49
        const SIG50 = 1u64 << 49;   // signum 50
        const SIG51 = 1u64 << 50;   // signum 51
        const SIG52 = 1u64 << 51;   // signum 52
        const SIG53 = 1u64 << 52;   // signum 53
        const SIG54 = 1u64 << 53;   // signum 54
        const SIG55 = 1u64 << 54;   // signum 55
        const SIG56 = 1u64 << 55;   // signum 56
        const SIG57 = 1u64 << 56;   // signum 57
        const SIG58 = 1u64 << 57;   // signum 58
        const SIG59 = 1u64 << 58;   // signum 59
        const SIG60 = 1u64 << 59;   // signum 60
        const SIG61 = 1u64 << 60;   // signum 61
        const SIG62 = 1u64 << 61;   // signum 62
        const SIG63 = 1u64 << 62;   // signum 63
        const SIG64 = 1u64 << 63;   // signum 64
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

/// 将内核 SignalFlags 转换为用户态 sigset_t mask
/// SignalFlags 和 user_mask 的 bit 布局完全一致：bit N 表示 signum N+1
pub fn flags_to_user_mask(flags: SignalFlags) -> u64 {
    flags.bits()
}

/// 将用户态 sigset_t mask 转换为内核 SignalFlags
/// SignalFlags 和 user_mask 的 bit 布局完全一致：bit N 表示 signum N+1
pub fn user_mask_to_flags(user_mask: u64) -> SignalFlags {
    SignalFlags::from_bits_truncate(user_mask)
}
