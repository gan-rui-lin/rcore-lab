#![allow(missing_docs)]

pub const MAX_SIG: usize = 64;

/// Standard signal numbers (POSIX / Linux)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
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

/// Linux-compatible signal set (64 signals, 1-indexed).
/// Bit 0 = signal 1 (SIGHUP), bit 63 = signal 64.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SigSet(pub u64);

impl SigSet {
    pub const fn empty() -> Self {
        SigSet(0)
    }

    pub const fn from_raw(bits: u64) -> Self {
        SigSet(bits)
    }

    pub const fn raw(&self) -> u64 {
        self.0
    }

    pub const fn is_empty(&self) -> bool {
        self.0 == 0
    }

    /// Check if signal `signum` (1-indexed) is in the set.
    pub const fn contains_sig(&self, signum: usize) -> bool {
        if signum == 0 || signum > 64 {
            return false;
        }
        self.0 & (1u64 << (signum - 1)) != 0
    }

    /// Add signal `signum` (1-indexed) to the set.
    pub fn add_sig(&mut self, signum: usize) {
        if signum >= 1 && signum <= 64 {
            self.0 |= 1u64 << (signum - 1);
        }
    }

    /// Remove signal `signum` (1-indexed) from the set.
    pub fn remove_sig(&mut self, signum: usize) {
        if signum >= 1 && signum <= 64 {
            self.0 &= !(1u64 << (signum - 1));
        }
    }

    /// Return the lowest signal number in the set, or None if empty.
    pub const fn lowest_signal(&self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some(self.0.trailing_zeros() as usize + 1)
        }
    }

    /// Union: self |= other
    pub fn union(&mut self, other: SigSet) {
        self.0 |= other.0;
    }

    /// Subtract: self &= !other
    pub fn subtract(&mut self, other: SigSet) {
        self.0 &= !other.0;
    }

    /// Intersection (returns new set)
    pub const fn intersect(&self, other: SigSet) -> SigSet {
        SigSet(self.0 & other.0)
    }

    /// Complement of other applied as mask: self & !other
    pub const fn and_not(&self, other: SigSet) -> SigSet {
        SigSet(self.0 & !other.0)
    }

    /// Union (returns new set without mutation)
    pub const fn or(&self, other: SigSet) -> SigSet {
        SigSet(self.0 | other.0)
    }

    /// Convenience: SIGKILL (9) and SIGSTOP (19) — never maskable
    pub const fn unmaskable() -> SigSet {
        SigSet((1u64 << (9 - 1)) | (1u64 << (19 - 1)))
    }

    /// Remove SIGKILL and SIGSTOP from the set (used when setting signal mask).
    pub fn sanitize_mask(&mut self) {
        self.remove_sig(9);  // SIGKILL
        self.remove_sig(19); // SIGSTOP
    }
}

impl Default for SigSet {
    fn default() -> Self {
        Self::empty()
    }
}

/// Signal default action categories
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SigDefaultAction {
    Term,
    Core,
    Stop,
    Cont,
    Ignore,
}

/// Get the default action for a signal number (1-indexed).
pub fn sig_default_action(signum: usize) -> SigDefaultAction {
    match signum {
        // Term: terminate process
        1 | 2 | 3 | 14 | 15 | 24 | 25 | 26 | 27 | 29 | 30 | 31 => SigDefaultAction::Term,
        // Core: terminate + core dump
        4 | 5 | 6 | 7 | 8 | 11 => SigDefaultAction::Core,
        // Stop
        19 | 20 | 21 | 22 => SigDefaultAction::Stop,
        // Cont
        18 => SigDefaultAction::Cont,
        // Ignore
        10 | 12 | 13 | 16 | 17 | 23 | 28 => SigDefaultAction::Ignore,
        // SIGKILL = 9, handled specially before this
        9 => SigDefaultAction::Term,
        // Unknown / real-time: default is Term
        _ => SigDefaultAction::Term,
    }
}

