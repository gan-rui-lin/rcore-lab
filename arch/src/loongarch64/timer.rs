#![allow(missing_docs)]

use loongArch64::register::ecfg::{self, LineBasedInterrupt};
use loongArch64::register::tcfg;
use loongArch64::time::{get_timer_freq, Time as LaTime};
use spin::Lazy;

static FREQ: Lazy<usize> = Lazy::new(|| get_timer_freq());

#[derive(Clone, Copy, Debug)]
pub struct Time(pub usize);

impl Time {
    #[inline]
    pub fn get_freq() -> usize {
        *FREQ
    }

    #[inline]
    pub fn now() -> Self {
        Self(LaTime::read())
    }

    #[inline]
    pub fn to_sec(&self) -> usize {
        self.0 / Self::get_freq()
    }

    #[inline]
    pub fn to_msec(&self) -> usize {
        self.0 * 1_000 / Self::get_freq()
    }

    #[inline]
    pub fn to_usec(&self) -> usize {
        self.0 * 1_000_000 / Self::get_freq()
    }

    #[inline]
    pub fn to_nsec(&self) -> usize {
        self.0 * 1_000_000_000 / Self::get_freq()
    }

    #[inline]
    pub fn raw(&self) -> usize {
        self.0
    }

    #[inline]
    pub fn from_raw(raw: usize) -> Self {
        Self(raw)
    }
}

pub fn init_timer() {
    let ticks = ((*FREQ / 100) + 3) & !3;
    tcfg::set_periodic(true);
    tcfg::set_init_val(ticks);
    tcfg::set_en(true);

    let inter = LineBasedInterrupt::TIMER
        | LineBasedInterrupt::SWI0
        | LineBasedInterrupt::SWI1
        | LineBasedInterrupt::HWI0;
    ecfg::set_lie(inter);
}

// ---------------------------------------------------------------------------
// Unified timer API wrappers (matching the arch-agnostic interface)
// ---------------------------------------------------------------------------

/// Get current time as a raw counter value.
#[inline]
pub fn get_time() -> usize {
    Time::now().raw()
}

/// Get current time in milliseconds.
#[inline]
pub fn get_time_ms() -> usize {
    Time::now().to_msec()
}

/// Get current time in microseconds.
#[inline]
pub fn get_time_us() -> usize {
    Time::now().to_usec()
}

/// Re-arm the timer for the next tick.
///
/// On LoongArch the timer is periodic (configured once in `init_timer`),
/// so there is nothing to do here.
#[inline]
pub fn set_next_trigger() {
    // LoongArch timer is periodic -- no explicit re-arm needed.
}
