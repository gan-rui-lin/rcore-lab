//! Process management syscalls
//!
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use bitflags::bitflags;
use lazy_static::lazy_static;

#[cfg(not(target_arch = "loongarch64"))]
use crate::task::SA_RESTORER;
use crate::{
    fs::{open_file, path_exists, path_is_dir, File, OpenFlags},
    mm::{
        translated_ref, translated_refmut, translated_str_checked, MapAreaType, MapPermission,
        MmapMeta, PageTable, ProtectError, VirtAddr,
    },
    task::{
        add_task, current_process, current_task, current_trap_cx, current_user_token,
        exit_current_and_run_next, flags_to_user_mask, futex_remove_waiter,
        futex_remove_waiter_any, futex_requeue, futex_wait, futex_wait_bitset, futex_wake,
        futex_wake_bitset, pid2process, pid2process_snapshot, remove_from_pid2process,
        suspend_current_and_run_next, user_mask_to_flags, ChildWaitEvent, FutexKey,
        IntervalTimerState, RLimit, SignalAction, SignalFlags, TaskControlBlock, TaskStatus,
        UserContext, MAX_SIG, RLIMIT_NLIMITS, SIGCHLD, SIGCONT, SIGKILL, SIGSEGV, SIGSTOP,
    },
    timer::{add_timer, get_time, get_time_ms, get_time_us, remove_timer},
};

use arch::TrapFrameArgs;

use super::errno::*;
use super::user_mem::{self, UserReadPolicy, UserWritePolicy};
use crate::config::USER_STACK_TOP as USER_ADDR_MAX;
use crate::config::{CLOCK_FREQ, PAGE_SIZE};
use crate::sync::UPIntrFreeCell;

lazy_static! {
    static ref EXEC_IMAGE_CACHE: UPIntrFreeCell<BTreeMap<String, Arc<[u8]>>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
    static ref UMASK_STATE: UPIntrFreeCell<usize> = unsafe { UPIntrFreeCell::new(0o022) };
    static ref REALTIME_OFFSET_US: UPIntrFreeCell<i64> = unsafe { UPIntrFreeCell::new(0) };
    static ref UTS_STATE: UPIntrFreeCell<(String, String)> =
        unsafe { UPIntrFreeCell::new((String::from("rcore"), String::from("ruos"))) };
    static ref SCHED_POLICIES: UPIntrFreeCell<BTreeMap<usize, i32>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
    static ref PERSONALITY_STATE: UPIntrFreeCell<BTreeMap<usize, usize>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
    static ref POSIX_TIMERS: UPIntrFreeCell<BTreeMap<(usize, usize), PosixTimer>> =
        unsafe { UPIntrFreeCell::new(BTreeMap::new()) };
    static ref POSIX_TIMER_NEXT_ID: UPIntrFreeCell<usize> = unsafe { UPIntrFreeCell::new(0) };
    static ref TIMEX_STATE: UPIntrFreeCell<TimexState> =
        unsafe { UPIntrFreeCell::new(TimexState::default()) };
}

#[allow(dead_code)]
fn dump_user_bytes(tag: &str, token: usize, addr: usize, len: usize) {
    let page_table = PageTable::from_token(token);
    let end = addr.saturating_add(len);
    let mut cur = addr;
    while cur < end {
        let mut line = [0u8; 16];
        let mut any = false;
        let line_end = core::cmp::min(cur + 16, end);
        for i in 0..(line_end - cur) {
            let va = cur + i;
            if let Some(pa) = page_table.translate_va(VirtAddr::from(va)) {
                line[i] = *pa.get_ref::<u8>();
                any = true;
            }
        }
        if any {
            info!(
                "[clone-tls] {} {:#x}: {:02x?}",
                tag,
                cur,
                &line[..(line_end - cur)]
            );
        } else {
            info!("[clone-tls] {} {:#x}: <unmapped>", tag, cur);
        }
        cur += 16;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct ITimerVal {
    pub it_interval: TimeVal,
    pub it_value: TimeVal,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeSpec {
    pub tv_sec: usize,
    pub tv_nsec: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Tms {
    pub tms_utime: i64,
    pub tms_stime: i64,
    pub tms_cutime: i64,
    pub tms_cstime: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RUsageTimeVal {
    pub tv_sec: i64,
    pub tv_usec: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RUsage {
    pub ru_utime: RUsageTimeVal,
    pub ru_stime: RUsageTimeVal,
    pub ru_maxrss: i64,
    pub ru_ixrss: i64,
    pub ru_idrss: i64,
    pub ru_isrss: i64,
    pub ru_minflt: i64,
    pub ru_majflt: i64,
    pub ru_nswap: i64,
    pub ru_inblock: i64,
    pub ru_oublock: i64,
    pub ru_msgsnd: i64,
    pub ru_msgrcv: i64,
    pub ru_nsignals: i64,
    pub ru_nvcsw: i64,
    pub ru_nivcsw: i64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SysInfo {
    pub uptime: i64,
    pub loads: [u64; 3],
    pub totalram: u64,
    pub freeram: u64,
    pub sharedram: u64,
    pub bufferram: u64,
    pub totalswap: u64,
    pub freeswap: u64,
    pub procs: u16,
    pub pad: u16,
    pub totalhigh: u64,
    pub freehigh: u64,
    pub mem_unit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct UtsName {
    pub sysname: [u8; 65],
    pub nodename: [u8; 65],
    pub release: [u8; 65],
    pub version: [u8; 65],
    pub machine: [u8; 65],
    pub domainname: [u8; 65],
}

fn timeval_to_us(tv: TimeVal) -> usize {
    tv.sec
        .saturating_mul(1_000_000)
        .saturating_add(tv.usec.min(999_999))
}

fn us_to_timeval(us: usize) -> TimeVal {
    TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    }
}

fn itimer_state_to_user(state: IntervalTimerState) -> ITimerVal {
    ITimerVal {
        it_interval: us_to_timeval(state.interval_us),
        it_value: us_to_timeval(state.remaining_us),
    }
}

fn itimer_state_from_user(timer: ITimerVal) -> IntervalTimerState {
    IntervalTimerState {
        interval_us: timeval_to_us(timer.it_interval),
        remaining_us: timeval_to_us(timer.it_value),
    }
}

bitflags! {
    struct FutexCmd: u32 {
        const FUTEX_WAIT = 0;
        const FUTEX_WAKE = 1;
        const FUTEX_REQUEUE = 3;
        const FUTEX_CMP_REQUEUE = 4;
        const FUTEX_WAIT_BITSET = 9;
        const FUTEX_WAKE_BITSET = 10;
    }
}

bitflags! {
    struct FutexOpt: u32 {
        const FUTEX_PRIVATE_FLAG = 0x80;
        const FUTEX_CLOCK_REALTIME = 0x100;
    }
}

bitflags! {
    pub struct CloneFlags: u64 {
        const VM = 0x0000_0100;
        const FS = 0x0000_0200;
        const FILES = 0x0000_0400;
        const SIGHAND = 0x0000_0800;
        const PARENT = 0x0000_8000;
        const VFORK = 0x0000_4000;
        const THREAD = 0x0001_0000;
        const SYSVSEM = 0x0004_0000;
        const SETTLS = 0x0008_0000;
        const PARENT_SETTID = 0x0010_0000;
        const CHILD_CLEARTID = 0x0020_0000;
        const CHILD_SETTID = 0x0100_0000;
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct CloneArgsUser {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

impl Default for UtsName {
    fn default() -> Self {
        Self {
            sysname: [0; 65],
            nodename: [0; 65],
            release: [0; 65],
            version: [0; 65],
            machine: [0; 65],
            domainname: [0; 65],
        }
    }
}

fn copy_to_user(token: usize, dst: *mut u8, data: &[u8]) -> Result<(), isize> {
    if dst.is_null() {
        return Err(errno(EFAULT));
    }
    let start = dst as usize;
    let end = start.checked_add(data.len()).ok_or_else(|| errno(EFAULT))?;
    if start >= USER_ADDR_MAX || end > USER_ADDR_MAX {
        return Err(errno(EFAULT));
    }
    user_mem::copy_to_user(token, dst, data, UserWritePolicy::RelaxedReadableMapping)
}

fn read_from_user<T: Copy>(token: usize, src: *const T) -> Result<T, isize> {
    user_mem::read_from_user(token, src, UserReadPolicy::StrictChecked)
}

pub fn sys_futex(
    uaddr1: *mut i32,
    futex_op: u32,
    val: i32,
    timeout: *const TimeSpec,
    uaddr2: *mut i32,
    val3: i32,
) -> isize {
    let pid_now = current_process().pid.0;
    if uaddr1.is_null() || (uaddr1 as usize) % core::mem::size_of::<i32>() != 0 {
        return errno(EINVAL);
    }
    let cmd = FutexCmd::from_bits_truncate(futex_op & 0x7f);
    let opt = FutexOpt::from_bits_truncate(futex_op);
    let token = current_user_token();
    let uaddr1_va = uaddr1 as usize;
    if !user_mem::ensure_user_readable(
        token,
        uaddr1 as *const u8,
        core::mem::size_of::<i32>(),
        UserReadPolicy::DemandPaged,
    ) {
        return errno(EFAULT);
    }
    let page_table = PageTable::from_token(token);
    let pa = match page_table.translate_va(VirtAddr::from(uaddr1_va)) {
        Some(pa) => pa,
        None => return errno(EFAULT),
    };
    let private = opt.contains(FutexOpt::FUTEX_PRIVATE_FLAG);
    let pid = if private { current_process().pid.0 } else { 0 };
    // For private futexes, key by user virtual address within the process.
    // Physical page can change due to COW after fork/threads, which would
    // otherwise break wakeup matching for the same uaddr.
    let key_addr = if private {
        crate::mm::PhysAddr::from(uaddr1 as usize)
    } else {
        pa
    };
    let key = FutexKey::new(key_addr, pid);
    let name = current_process().name();
    if matches!(cmd, FutexCmd::FUTEX_WAIT | FutexCmd::FUTEX_WAIT_BITSET)
        && name == "entry-static.exe"
    {
        let tid = current_task().map(|task| task.tid()).unwrap_or(0);
        info!(
            "[sys_futex] pid={} tid={} name={} cmd={:?} uaddr1={:#x} pa={:#x} private={} val={}",
            pid_now, tid, name, cmd, uaddr1 as usize, pa.0, private, val
        );
    }
    trace!(
        "[sys_futex] pid={} op={:#x} cmd={:?} private={} uaddr1={:#x} pa={:#x} val={} uaddr2={:#x} val3={} timeout={}",
        pid_now,
        futex_op,
        cmd,
        private,
        uaddr1 as usize,
        pa.0,
        val,
        uaddr2 as usize,
        val3,
        !timeout.is_null()
    );

    match cmd {
        FutexCmd::FUTEX_WAIT => {
            let futex_word = translated_ref(token, uaddr1);
            if *futex_word != val {
                trace!(
                    "[sys_futex] pid={} wait mismatch word={} val={}",
                    pid_now,
                    *futex_word,
                    val
                );
                return errno(EAGAIN);
            }
            let has_timeout = !timeout.is_null();
            if has_timeout {
                let spec = match read_from_user(token, timeout) {
                    Ok(value) => value,
                    Err(err) => return err,
                };
                if spec.tv_nsec >= 1_000_000_000 {
                    return errno(EINVAL);
                }
                let add_ms = spec
                    .tv_sec
                    .saturating_mul(1000)
                    .saturating_add(spec.tv_nsec / 1_000_000);
                let expire_ms = get_time_ms().saturating_add(add_ms);
                add_timer(expire_ms, current_task().unwrap());
            }
            trace!(
                "[sys_futex] pid={} wait sleep word={} timeout={}",
                pid_now,
                *futex_word,
                has_timeout
            );
            let ret = futex_wait(key.clone());
            if has_timeout {
                let task = current_task().unwrap();
                let timed_out = futex_remove_waiter(&key, &task);
                remove_timer(task);
                if timed_out {
                    trace!("[sys_futex] pid={} wait timed out", pid_now);
                    return errno(ETIMEDOUT);
                }
            }
            if ret == errno(EINTR) {
                let process = current_process();
                let process_pending = process.pending_signal();
                let (tid, mask, task_pending, handling_sig, interrupted_by_signal, sepc, ra, tp) =
                    match current_task() {
                        Some(task) => {
                            let (mask, task_pending, handling_sig, interrupted_by_signal) = task
                                .with_signals(|signals| {
                                    (
                                        signals.signal_mask,
                                        signals.signal_pending,
                                        signals.handling_sig,
                                        signals.interrupted_by_signal,
                                    )
                                });
                            let (sepc, ra, tp) = task.with_trap_cx_mut(|trap_cx| {
                                (
                                    trap_cx.sepc,
                                    trap_cx[TrapFrameArgs::RA],
                                    trap_cx[TrapFrameArgs::TLS],
                                )
                            });
                            (
                                task.tid(),
                                mask,
                                task_pending,
                                handling_sig,
                                interrupted_by_signal,
                                sepc,
                                ra,
                                tp,
                            )
                        }
                        None => (
                            0,
                            SignalFlags::empty(),
                            SignalFlags::empty(),
                            -1,
                            false,
                            0,
                            0,
                            0,
                        ),
                    };
                let pending_unmasked = (process_pending | task_pending) & !mask;
                info!(
                    "[sys_futex] wait EINTR pid={} tid={} uaddr1={:#x} pa={:#x} val={} handling_sig={} interrupted={} sepc={:#x} ra={:#x} tp={:#x} mask={:?} task_pending={:?} proc_pending={:?} unmasked={:?} sig33_pending(task={},proc={})",
                    pid_now,
                    tid,
                    uaddr1 as usize,
                    pa.0,
                    val,
                    handling_sig,
                    interrupted_by_signal,
                    sepc,
                    ra,
                    tp,
                    mask,
                    task_pending,
                    process_pending,
                    pending_unmasked,
                    task_pending.contains(SignalFlags::SIG33),
                    process_pending.contains(SignalFlags::SIG33),
                );
            }
            ret
        }
        FutexCmd::FUTEX_WAKE => {
            let woke = futex_wake(key, val as usize) as isize;
            trace!("[sys_futex] pid={} wake n={}", pid_now, woke);
            woke
        }
        FutexCmd::FUTEX_REQUEUE => {
            if uaddr2.is_null() {
                return errno(EINVAL);
            }
            let pa2 = match page_table.translate_va(VirtAddr::from(uaddr2 as usize)) {
                Some(pa) => pa,
                None => return errno(EFAULT),
            };
            let new_key_addr = if private {
                crate::mm::PhysAddr::from(uaddr2 as usize)
            } else {
                pa2
            };
            let new_key = FutexKey::new(new_key_addr, pid);
            let requeued = futex_requeue(key, val, new_key, val3) as isize;
            trace!(
                "[sys_futex] pid={} requeue n={} uaddr2={:#x} pa2={:#x}",
                pid_now,
                requeued,
                uaddr2 as usize,
                pa2.0
            );
            requeued
        }
        FutexCmd::FUTEX_CMP_REQUEUE => {
            // val3 contains expected value for comparison
            // val = max_wake, timeout (as usize) = max_requeue
            if uaddr2.is_null() {
                return errno(EINVAL);
            }
            let futex_word = translated_ref(token, uaddr1);
            if *futex_word != val3 {
                trace!(
                    "[sys_futex] pid={} cmp_requeue mismatch word={} expected={}",
                    pid_now,
                    *futex_word,
                    val3
                );
                return errno(EAGAIN);
            }
            let pa2 = match page_table.translate_va(VirtAddr::from(uaddr2 as usize)) {
                Some(pa) => pa,
                None => return errno(EFAULT),
            };
            let new_key_addr = if private {
                crate::mm::PhysAddr::from(uaddr2 as usize)
            } else {
                pa2
            };
            let new_key = FutexKey::new(new_key_addr, pid);
            // For CMP_REQUEUE: val = max_wake, timeout (cast to usize) = max_requeue
            let max_requeue = timeout as usize as i32;
            let requeued = futex_requeue(key, val, new_key, max_requeue) as isize;
            trace!(
                "[sys_futex] pid={} cmp_requeue n={} uaddr2={:#x} pa2={:#x} max_wake={} max_requeue={}",
                pid_now,
                requeued,
                uaddr2 as usize,
                pa2.0,
                val,
                max_requeue
            );
            requeued
        }
        FutexCmd::FUTEX_WAIT_BITSET => {
            if val3 == 0 {
                return errno(EINVAL);
            }
            let futex_word = translated_ref(token, uaddr1);
            if *futex_word != val {
                trace!(
                    "[sys_futex] pid={} wait_bitset mismatch word={} val={}",
                    pid_now,
                    *futex_word,
                    val
                );
                return errno(EAGAIN);
            }
            let has_timeout = !timeout.is_null();
            if has_timeout {
                let spec = match read_from_user(token, timeout) {
                    Ok(value) => value,
                    Err(err) => return err,
                };
                if spec.tv_nsec >= 1_000_000_000 {
                    return errno(EINVAL);
                }
                let add_ms = spec
                    .tv_sec
                    .saturating_mul(1000)
                    .saturating_add(spec.tv_nsec / 1_000_000);
                let expire_ms = get_time_ms().saturating_add(add_ms);
                add_timer(expire_ms, current_task().unwrap());
            }
            trace!(
                "[sys_futex] pid={} wait_bitset sleep word={} bitset={:#x} timeout={}",
                pid_now,
                *futex_word,
                val3,
                has_timeout
            );
            let ret = futex_wait_bitset(key.clone(), val3);
            if has_timeout {
                let task = current_task().unwrap();
                let timed_out = futex_remove_waiter(&key, &task);
                remove_timer(task);
                if timed_out {
                    trace!("[sys_futex] pid={} wait_bitset timed out", pid_now);
                    return errno(ETIMEDOUT);
                }
            }
            if ret == errno(EINTR) {
                let process = current_process();
                let process_pending = process.pending_signal();
                let (tid, mask, task_pending, handling_sig, interrupted_by_signal, sepc, ra, tp) =
                    match current_task() {
                        Some(task) => {
                            let (mask, task_pending, handling_sig, interrupted_by_signal) = task
                                .with_signals(|signals| {
                                    (
                                        signals.signal_mask,
                                        signals.signal_pending,
                                        signals.handling_sig,
                                        signals.interrupted_by_signal,
                                    )
                                });
                            let (sepc, ra, tp) = task.with_trap_cx_mut(|trap_cx| {
                                (
                                    trap_cx.sepc,
                                    trap_cx[TrapFrameArgs::RA],
                                    trap_cx[TrapFrameArgs::TLS],
                                )
                            });
                            (
                                task.tid(),
                                mask,
                                task_pending,
                                handling_sig,
                                interrupted_by_signal,
                                sepc,
                                ra,
                                tp,
                            )
                        }
                        None => (
                            0,
                            SignalFlags::empty(),
                            SignalFlags::empty(),
                            -1,
                            false,
                            0,
                            0,
                            0,
                        ),
                    };
                let pending_unmasked = (process_pending | task_pending) & !mask;
                info!(
                    "[sys_futex] wait_bitset EINTR pid={} tid={} uaddr1={:#x} pa={:#x} val={} bitset={:#x} handling_sig={} interrupted={} sepc={:#x} ra={:#x} tp={:#x} mask={:?} task_pending={:?} proc_pending={:?} unmasked={:?} sig33_pending(task={},proc={})",
                    pid_now,
                    tid,
                    uaddr1 as usize,
                    pa.0,
                    val,
                    val3,
                    handling_sig,
                    interrupted_by_signal,
                    sepc,
                    ra,
                    tp,
                    mask,
                    task_pending,
                    process_pending,
                    pending_unmasked,
                    task_pending.contains(SignalFlags::SIG33),
                    process_pending.contains(SignalFlags::SIG33),
                );
            }
            ret
        }
        FutexCmd::FUTEX_WAKE_BITSET => {
            if val3 == 0 {
                return errno(EINVAL);
            }
            let woke = futex_wake_bitset(key, val as usize, val3) as isize;
            trace!(
                "[sys_futex] pid={} wake_bitset n={} bitset={:#x}",
                pid_now,
                woke,
                val3
            );
            woke
        }
        _ => errno(ENOSYS),
    }
}

pub fn sys_exit(exit_code: i32) -> ! {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_exit", pid);
    }
    let name = current_process().name();
    if pid == 4 || name == "sh" {
        trace!(
            "[sys_exit] pid={} name={} code={} sepc={:#x}",
            pid,
            name,
            exit_code,
            current_trap_cx().sepc
        );
    }
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_process().pid.0);
    current_process().pid.0 as isize
}

pub fn sys_getppid() -> isize {
    let process = current_process();
    if let Some(parent) = process.parent() {
        parent.pid.0 as isize
    } else {
        0
    }
}

pub fn sys_setsid() -> isize {
    let process = current_process();
    let pid = process.pid.0;
    process.with_identity_mut(|identity| {
        identity.session_id = pid;
        identity.pgid = pid;
    });
    pid as isize
}

pub fn sys_setpgid(pid: isize, pgid: isize) -> isize {
    if pid < 0 || pgid < 0 {
        return errno(EINVAL);
    }
    let process = current_process();
    let current_pid = process.pid.0;
    let target_pid = if pid == 0 { current_pid } else { pid as usize };
    let target_pgid = if pgid == 0 { target_pid } else { pgid as usize };
    let Some(target) = pid2process(target_pid) else {
        return errno(ESRCH);
    };

    if target_pid != current_pid {
        let parent_pid = target.parent().map(|p| p.pid.0);
        if parent_pid != Some(current_pid) {
            return errno(ESRCH);
        }
    }

    let current_session = process.session_id();
    let target_session = target.session_id();
    if current_session != target_session {
        return errno(EPERM);
    }
    if target_pgid != target_pid {
        let group_exists = pid2process(target_pgid)
            .map(|leader| leader.session_id() == current_session && leader.pgid() == target_pgid)
            .unwrap_or(false);
        if !group_exists {
            return errno(EPERM);
        }
    }
    target.set_pgid(target_pgid);
    0
}

pub fn sys_getpgid(pid: isize) -> isize {
    let process = current_process();
    if pid == 0 || pid as usize == process.pid.0 {
        process.pgid() as isize
    } else if pid < 0 {
        // Negative pid: no process can have a negative pid → ESRCH.
        errno(ESRCH)
    } else {
        // Look up target process; return ESRCH if it doesn't exist.
        match pid2process(pid as usize) {
            Some(target) => target.pgid() as isize,
            None => errno(ESRCH),
        }
    }
}

pub fn sys_getsid(pid: isize) -> isize {
    let process = current_process();
    if pid == 0 || pid as usize == process.pid.0 {
        process.session_id() as isize
    } else if pid < 0 {
        errno(EINVAL)
    } else {
        // Look up target process; return ESRCH if it doesn't exist.
        match pid2process(pid as usize) {
            Some(target) => target.session_id() as isize,
            None => errno(ESRCH),
        }
    }
}
pub fn sys_fork() -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_fork", pid);
    }
    let current_process = current_process();
    let new_process = current_process.fork();
    let new_pid = new_process.pid.0;
    let new_task = new_process.get_task(0);
    let parent_cx = *current_trap_cx();
    let clone_stack = parent_cx[TrapFrameArgs::ARG1];
    new_task.with_trap_cx_mut(|trap_cx| {
        *trap_cx = parent_cx;
        trap_cx[TrapFrameArgs::RET] = 0;
        #[cfg(target_arch = "loongarch64")]
        if trap_cx[TrapFrameArgs::TLS] == 0 {
            trap_cx[TrapFrameArgs::TLS] = 0x7000_1000;
        }
        if clone_stack != 0 {
            trap_cx[TrapFrameArgs::SP] = clone_stack;
        }
    });
    new_pid as isize
}

pub fn sys_clone3(args_ptr: *const u8, size: usize) -> isize {
    if args_ptr.is_null() {
        return errno(EFAULT);
    }
    if size < core::mem::size_of::<CloneArgsUser>() {
        return errno(EINVAL);
    }

    let token = current_user_token();
    let args = match read_from_user(token, args_ptr as *const CloneArgsUser) {
        Ok(v) => v,
        Err(e) => return e,
    };

    // clone3 extensions we don't support yet.
    if args.pidfd != 0 || args.set_tid != 0 || args.set_tid_size != 0 || args.cgroup != 0 {
        return errno(EINVAL);
    }

    let flags = args.flags;
    if flags & !CloneFlags::all().bits() != 0 {
        return errno(EINVAL);
    }

    let exit_signal = args.exit_signal as i32;
    if args.exit_signal > 0xff
        || (exit_signal != 0 && (exit_signal <= 0 || exit_signal > MAX_SIG as i32))
    {
        return errno(EINVAL);
    }

    let stack = args.stack as *const u8;
    let ptid = args.parent_tid as *mut i32;
    let tls = args.tls as *mut i32;
    let ctid = args.child_tid as *mut i32;
    let clone_flags = flags | args.exit_signal;
    sys_clone(clone_flags as usize, stack, ptid, tls, ctid)
}

pub fn sys_clone(
    flags: usize,
    stack: *const u8,
    ptid: *mut i32,
    tls: *mut i32,
    ctid: *mut i32,
) -> isize {
    const CLONE_NEWPID: usize = 0x2000_0000;
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!(
            "kernel:pid[{}] sys_clone flags={:#x} stack={:#x} ptid={:#x} tls={:#x} ctid={:#x}",
            pid,
            flags,
            stack as usize,
            ptid as usize,
            tls as usize,
            ctid as usize
        );
    }
    // We do not support PID namespaces yet. Silently truncating this flag is
    // dangerous: tests like pidns10 may run container-only logic (kill(-1))
    // against the global process table and terminate unrelated workloads.
    if (flags & CLONE_NEWPID) != 0 {
        return errno(EINVAL);
    }
    let exit_signal = (flags & 0xff) as i32;
    if exit_signal != 0 && (exit_signal <= 0 || exit_signal > MAX_SIG as i32) {
        return errno(EINVAL);
    }
    let clone_flags = CloneFlags::from_bits_truncate((flags as u64) & !0xff);
    info!(
        "[clone] flags={:#x} clone_flags={:#x} stack={:#x} ptid={:#x} tls={:#x} ctid={:#x}",
        flags,
        clone_flags.bits(),
        stack as usize,
        ptid as usize,
        tls as usize,
        ctid as usize
    );
    let parent_cx = *current_trap_cx();

    if !clone_flags.contains(CloneFlags::THREAD) {
        let has_extended_clone_semantics = clone_flags.intersects(
            CloneFlags::PARENT
                | CloneFlags::VFORK
                | CloneFlags::PARENT_SETTID
                | CloneFlags::CHILD_CLEARTID
                | CloneFlags::CHILD_SETTID
                | CloneFlags::SETTLS
                | CloneFlags::VM
                | CloneFlags::FS
                | CloneFlags::FILES
                | CloneFlags::SIGHAND
                | CloneFlags::SYSVSEM,
        ) || !stack.is_null();
        if !has_extended_clone_semantics {
            return sys_fork();
        }

        let current = current_process();
        let new_process = current.fork();
        let new_pid = new_process.pid.0;
        if clone_flags.contains(CloneFlags::VM) && clone_flags.contains(CloneFlags::VFORK) {
            new_process.set_vfork_parent(Some(Arc::downgrade(&current)));
        }

        let new_task = new_process.get_task(0);
        new_task.with_trap_cx_mut(|trap_cx| {
            *trap_cx = parent_cx;
            trap_cx[TrapFrameArgs::RET] = 0;
            if !stack.is_null() {
                trap_cx[TrapFrameArgs::SP] = stack as usize;
            }
        });

        if clone_flags.contains(CloneFlags::CHILD_CLEARTID) && !ctid.is_null() {
            new_task.set_clear_child_tid(ctid as usize);
        }

        if clone_flags.contains(CloneFlags::PARENT_SETTID) && !ptid.is_null() {
            let parent_token = current_user_token();
            *translated_refmut(parent_token, ptid) = new_pid as i32;
            // Make sure child can observe the same value as Linux clone semantics.
            let child_token = new_process.get_user_token();
            *translated_refmut(child_token, ptid) = new_pid as i32;
        }
        if clone_flags.contains(CloneFlags::CHILD_SETTID) && !ctid.is_null() {
            let child_token = new_process.get_user_token();
            *translated_refmut(child_token, ctid) = new_pid as i32;
        }

        if clone_flags.contains(CloneFlags::PARENT) {
            let parent_weak = current.parent_weak();
            current.remove_child_by_pid(new_pid);
            new_process.set_parent(parent_weak.clone());
            if let Some(parent) = parent_weak.and_then(|p| p.upgrade()) {
                parent.push_child(Arc::clone(&new_process));
            }
        }

        if clone_flags.contains(CloneFlags::VFORK) {
            loop {
                if new_process.is_zombie() {
                    break;
                }
                suspend_current_and_run_next();
            }
        }

        return new_pid as isize;
    }

    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();

    let ustack_base = task.ustack_base();
    let parent_signal_mask = task.signal_mask();

    let alloc_user_stack = stack.is_null();
    let new_task = Arc::new(TaskControlBlock::new(
        Arc::clone(&process),
        ustack_base,
        alloc_user_stack,
    ));
    // 新线程继承父线程的信号掩码（Linux 语义：clone(CLONE_THREAD) 继承 signal_mask）
    new_task.set_signal_mask(parent_signal_mask);
    let new_task_tid = new_task.tid();
    process.insert_task(new_task_tid, Arc::clone(&new_task));

    new_task.with_trap_cx_mut(|new_trap_cx| {
        *new_trap_cx = parent_cx;
        new_trap_cx[TrapFrameArgs::RET] = 0;
        if !stack.is_null() {
            new_trap_cx[TrapFrameArgs::SP] = stack as usize;
        }
        if clone_flags.contains(CloneFlags::SETTLS) && !tls.is_null() {
            new_trap_cx[TrapFrameArgs::TLS] = tls as usize;
            let name = current_process().name();
            if name == "entry-static.exe" {
                let token = current_user_token();
                let tls_addr = tls as usize;
                info!(
                    "[clone-tls] pid={} tid={} tls={:#x} stack={:#x}",
                    pid, new_task_tid, tls_addr, stack as usize
                );
                let base = tls_addr.saturating_sub(256);
                dump_user_bytes("tp-0x100", token, base, 128);
                dump_user_bytes("tp-0x80", token, tls_addr.saturating_sub(128), 128);
                dump_user_bytes("tp+0x0", token, tls_addr, 64);
            }
        }
    });
    if clone_flags.contains(CloneFlags::CHILD_CLEARTID) && !ctid.is_null() {
        new_task.set_clear_child_tid(ctid as usize);
        info!(
            "[clone] pid={} tid={} child_cleartid={:#x}",
            pid, new_task_tid, ctid as usize
        );
    }

    let system_tid = super::thread::to_user_tid(pid, new_task_tid) as i32;
    if clone_flags.contains(CloneFlags::PARENT_SETTID) && !ptid.is_null() {
        let token = current_user_token();
        info!(
            "[clone] PARENT_SETTID ptid={:#x} system_tid={}",
            ptid as usize, system_tid
        );
        *translated_refmut(token, ptid) = system_tid;
    }
    if clone_flags.contains(CloneFlags::CHILD_SETTID) && !ctid.is_null() {
        let token = new_task.get_user_token();
        info!(
            "[clone] CHILD_SETTID ctid={:#x} system_tid={}",
            ctid as usize, system_tid
        );
        *translated_refmut(token, ctid) = system_tid;
    }

    // Queue the child only after its trap context/TLS/tid pointers are fully initialized.
    add_task(Arc::clone(&new_task));

    system_tid as isize
}

/// Maximum depth for shebang recursion to prevent infinite loops
const MAX_SHEBANG_DEPTH: usize = 4;

/// Parse shebang line and return (interpreter_path, optional_arg)
fn parse_shebang(data: &[u8]) -> Option<(String, Option<String>)> {
    // Check for shebang marker
    if data.len() < 2 || data[0] != b'#' || data[1] != b'!' {
        return None;
    }

    // Find the end of first line
    let line_end = data
        .iter()
        .position(|&b| b == b'\n' || b == b'\r')
        .unwrap_or(data.len());
    if line_end <= 2 {
        return None;
    }

    // Extract the shebang line (skip #!)
    let shebang_line = &data[2..line_end];

    // Convert to string and trim whitespace
    let shebang_str = core::str::from_utf8(shebang_line).ok()?.trim();
    if shebang_str.is_empty() {
        return None;
    }

    let mut parts = shebang_str.split_whitespace();
    let interpreter = String::from(parts.next()?);
    let arg = parts.next().map(String::from);
    Some((interpreter, arg))
}

fn resolve_relative_path(path: &str) -> String {
    let cwd = current_process().cwd();
    if cwd == "/" {
        format!("/{}", path)
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), path)
    }
}

const EXEC_PATH_MAX: usize = 4096;
const EXEC_NAME_MAX: usize = 255;

fn validate_exec_path(path: &str) -> Result<(), isize> {
    if path.is_empty() {
        return Err(errno(ENOENT));
    }
    if path.len() >= EXEC_PATH_MAX {
        return Err(errno(ENAMETOOLONG));
    }
    if path
        .split('/')
        .any(|part| !part.is_empty() && part.len() > EXEC_NAME_MAX)
    {
        return Err(errno(ENAMETOOLONG));
    }
    Ok(())
}

fn has_non_dir_prefix(path: &str) -> bool {
    let comps: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    if comps.len() < 2 {
        return false;
    }
    let mut prefix = String::from("/");
    for (idx, comp) in comps.iter().enumerate() {
        if idx > 0 && !prefix.ends_with('/') {
            prefix.push('/');
        }
        prefix.push_str(comp);
        if idx + 1 < comps.len() && path_exists(prefix.as_str()) && !path_is_dir(prefix.as_str()) {
            return true;
        }
    }
    false
}

fn build_exec_candidates(exec_path: &str, envs: &[String]) -> Vec<String> {
    // 绝对路径直接返回
    if exec_path.starts_with('/') {
        return vec![String::from(exec_path)];
    }
    // 相对路径需要基于当前工作目录拼接
    if exec_path.contains('/') {
        return vec![resolve_relative_path(exec_path)];
    }
    let mut candidates = Vec::new();
    // 根据 PATH 环境变量拼接候选路径，PATH 以冒号分隔多个目录，空目录表示当前目录
    if let Some(path_env) = envs.iter().find(|env| env.starts_with("PATH=")) {
        let path_value = &path_env[5..];
        for dir in path_value.split(':') {
            if dir.is_empty() {
                continue;
            }
            let candidate = if dir == "/" {
                format!("/{}", exec_path)
            } else {
                format!("{}/{}", dir.trim_end_matches('/'), exec_path)
            };
            candidates.push(candidate);
        }
    }
    if candidates.is_empty() {
        candidates.push(resolve_relative_path(exec_path));
    }
    candidates
}

fn exec_image_cache_key(_path: &str) -> Option<&'static str> {
    // Strict mode: disable cross-path cache aliasing to avoid mixing musl/glibc images.
    None
}

fn read_exec_image(path: &str, file: &Arc<dyn File>) -> Arc<[u8]> {
    // TODO: Replace eager read_all() in exec with a file-backed Reader + lazy page loading.
    // This keeps current behavior stable first; page-fault-driven loading can be added later.
    if let Some(inode) = file.inode() {
        let file_size = inode.size();
        if file_size >= 8 * 1024 * 1024 {
            info!(
                "[exec-image] large file path={} size={} bytes",
                path, file_size
            );
        }
    }
    if let Some(key) = exec_image_cache_key(path) {
        if let Some(cached) = EXEC_IMAGE_CACHE.exclusive_access().get(key).cloned() {
            return cached;
        }
        let data = Arc::<[u8]>::from(file.read_all().into_boxed_slice());
        let mut cache = EXEC_IMAGE_CACHE.exclusive_access();
        let entry = cache
            .entry(String::from(key))
            .or_insert_with(|| data.clone());
        return entry.clone();
    }
    Arc::<[u8]>::from(file.read_all().into_boxed_slice())
}

fn trace_exec_resolution(name: &str, exec_path: &str, exec_path_resolved: &str, args: &[String]) {
    if name == "busybox"
        && (exec_path.contains("run-all.sh")
            || exec_path.contains("/basic/")
            || exec_path.starts_with("./"))
    {
        let argv0 = args.get(0).cloned().unwrap_or_default();
        let argv1 = args.get(1).cloned().unwrap_or_default();
        trace!(
            "[sys_exec] pid={} name={} raw={} resolved={} argv0={} argv1={}",
            current_process().pid.0,
            name,
            exec_path,
            exec_path_resolved,
            argv0,
            argv1
        );
    }
}

fn trace_entry_bytes(exec_path_resolved: &str, all_data: &[u8], app: &Arc<dyn File>) {
    if exec_path_resolved != "/bin/sh"
        && exec_path_resolved != "/musl/busybox"
        && exec_path_resolved != "/glibc/busybox"
    {
        return;
    }
    if let Ok(elf) = xmas_elf::ElfFile::new(all_data) {
        let entry = elf.header.pt2.entry_point() as usize;
        let ph_count = elf.header.pt2.ph_count() as usize;
        for idx in 0..ph_count {
            if let Ok(ph) = elf.program_header(idx as u16) {
                if ph.get_type().ok() != Some(xmas_elf::program::Type::Load) {
                    continue;
                }
                let vaddr = ph.virtual_addr() as usize;
                let filesz = ph.file_size() as usize;
                if entry < vaddr || entry >= vaddr.saturating_add(filesz) {
                    continue;
                }
                let offset = ph.offset() as usize;
                let file_off = offset + (entry - vaddr);
                let end = (file_off + 8).min(all_data.len());
                if end > file_off {
                    let label = if exec_path_resolved == "/bin/sh" {
                        "/bin/sh"
                    } else {
                        "/musl/busybox"
                    };
                    let entry_bytes = &all_data[file_off..end];
                    trace!("[sys_exec] {} entry bytes={:02x?}", label, entry_bytes);
                    if exec_path_resolved == "/musl/busybox" && entry_bytes.iter().all(|b| *b == 0)
                    {
                        let head_len = all_data.len().min(16);
                        trace!(
                            "[sys_exec] busybox read_all len={} head={:02x?}",
                            all_data.len(),
                            &all_data[..head_len]
                        );
                        if let Some(inode) = app.inode() {
                            let mut buf = [0u8; 8];
                            let n = inode.read_at(file_off, &mut buf);
                            trace!(
                                "[sys_exec] busybox inode.read_at off={:#x} n={} bytes={:02x?}",
                                file_off,
                                n,
                                &buf[..n]
                            );
                            trace!("[sys_exec] busybox inode.size={}", inode.size());
                        } else {
                            trace!("[sys_exec] busybox inode missing");
                        }
                    }
                }
                break;
            }
        }
    }
}

fn trace_run_all_head(name: &str, exec_path: &str, all_data: &[u8]) {
    if name == "busybox" && exec_path.contains("run-all.sh") {
        let head_len = all_data.len().min(16);
        let head = &all_data[..head_len];
        trace!(
            "[sys_exec] run-all.sh head={:02x?} len={}",
            head,
            all_data.len()
        );
    }
}

fn sys_exec_internal(
    path: *const u8,
    argv: *const usize,
    envp: *const usize,
    depth: usize,
) -> isize {
    if path.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let mut exec_path = match translated_str_checked(token, path) {
        Some(s) => s,
        None => return errno(EFAULT),
    };
    if let Err(e) = validate_exec_path(exec_path.as_str()) {
        return e;
    }
    let mut args: Vec<String> = Vec::new();
    if !argv.is_null() {
        let mut argv_cur = argv;
        loop {
            let arg_ptr = match read_from_user::<usize>(token, argv_cur) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if arg_ptr == 0 {
                break;
            }
            let arg = match translated_str_checked(token, arg_ptr as *const u8) {
                Some(s) => s,
                None => return errno(EFAULT),
            };
            if arg.len() >= EXEC_PATH_MAX {
                return errno(ENAMETOOLONG);
            }
            args.push(arg);
            unsafe {
                argv_cur = argv_cur.add(1);
            }
        }
    }
    if args.is_empty() {
        args.push(exec_path.clone());
    }
    let mut envs: Vec<String> = Vec::new();
    if !envp.is_null() {
        let mut envp_cur = envp;
        loop {
            let env_ptr = match read_from_user::<usize>(token, envp_cur) {
                Ok(v) => v,
                Err(e) => return e,
            };
            if env_ptr == 0 {
                break;
            }
            let env = match translated_str_checked(token, env_ptr as *const u8) {
                Some(s) => s,
                None => return errno(EFAULT),
            };
            if env.len() >= EXEC_PATH_MAX {
                return errno(ENAMETOOLONG);
            }
            envs.push(env);
            unsafe {
                envp_cur = envp_cur.add(1);
            }
        }
    }

    let mut depth = depth;
    loop {
        if exec_path == "/bin/sh" {
            // 应该已经提前做了硬链接
            if open_file("/bin/sh", OpenFlags::empty()).is_none() {
                if open_file("/musl/busybox", OpenFlags::empty()).is_some() {
                    info!(
                        "[sys_exec] pid={} exec /bin/sh fallback to /musl/busybox",
                        current_process().pid.0
                    );
                    exec_path = String::from("/musl/busybox");
                } else {
                    error!(
                        "[sys_exec] pid={} exec /bin/sh fallback busybox also not found",
                        current_process().pid.0
                    );
                    return errno(ENOENT);
                }
            } else {
                trace!("[sys_exec] /bin/sh ready");
            }
        }
        let mut resolved_path = None;
        let mut app = None;
        // 根据 exec_path 和 PATH 环境变量构建候选路径列表，并尝试打开找到第一个存在的文件
        let candidates = build_exec_candidates(exec_path.as_str(), &envs);
        for candidate in &candidates {
            if let Some(found) = open_file(candidate.as_str(), OpenFlags::empty()) {
                resolved_path = Some(candidate.clone());
                app = Some(found);
                break;
            }
        }
        // 如果没有找到任何候选文件，返回 ENOENT
        let Some(app) = app else {
            let name = current_process().name();
            if name == "busybox" && exec_path.starts_with("./") {
                trace!(
                    "[sys_exec] pid={} name={} raw={} -> ENOENT (no candidate)",
                    current_process().pid.0,
                    name,
                    exec_path
                );
            }
            if (exec_path.starts_with('/') || exec_path.contains('/'))
                && has_non_dir_prefix(candidates[0].as_str())
            {
                return errno(ENOTDIR);
            }
            return errno(ENOENT);
        };
        // 如果找到的候选文件路径和原始 exec_path 不同，说明是通过 PATH 环境变量解析得到的，打印解析信息
        let exec_path_resolved = resolved_path.unwrap_or_else(|| exec_path.clone());
        let name = current_process().name();
        trace_exec_resolution(&name, &exec_path, &exec_path_resolved, &args);
        let all_data = read_exec_image(exec_path_resolved.as_str(), &app);
        trace_entry_bytes(&exec_path_resolved, all_data.as_ref(), &app);
        trace_run_all_head(&name, &exec_path, all_data.as_ref());
        // Prefer ELF execution; only try shebang (or /bin/sh fallback) for non-ELF.
        let is_elf = all_data.len() >= 4 && &all_data[..4] == b"\x7fELF";
        if !is_elf {
            if exec_path_resolved.starts_with("/musl/ltp/testcases/bin/") {
                let head_len = all_data.len().min(32);
                warn!(
                    "[sys_exec] non-ELF sample path={} len={} head={:02x?}",
                    exec_path_resolved,
                    all_data.len(),
                    &all_data[..head_len]
                );
                if let Some(inode) = app.inode() {
                    let mut sample = [0u8; 32];
                    let n = inode.read_at(0, &mut sample);
                    warn!(
                        "[sys_exec] inode kind={:?} size={} read0_n={} read0={:02x?}",
                        inode.kind(),
                        inode.size(),
                        n,
                        &sample[..n.min(sample.len())]
                    );
                }
            }
            // Check for shebang scripts first.
            if let Some((interpreter, opt_arg)) = parse_shebang(all_data.as_ref()) {
                trace!(
                    "[sys_exec] shebang interp={} opt_arg={:?} script={}",
                    interpreter,
                    opt_arg,
                    exec_path_resolved
                );
                if depth >= MAX_SHEBANG_DEPTH {
                    return errno(ELOOP);
                }
                depth += 1;
                trace!("[sys_exec] shebang interpreter ready: {}", interpreter);
                let interp_basename = interpreter
                    .rsplit('/')
                    .find(|part| !part.is_empty())
                    .unwrap_or(interpreter.as_str());
                let mut new_args: Vec<String> = Vec::new();
                if interp_basename == "sh" {
                    new_args.push(String::from("sh"));
                } else {
                    new_args.push(interpreter.clone());
                }
                if let Some(arg) = opt_arg {
                    new_args.push(arg);
                }
                new_args.push(exec_path_resolved.clone());
                if args.len() > 1 {
                    new_args.extend(args.into_iter().skip(1));
                }
                args = new_args;
                exec_path = interpreter;
                continue;
            }
            // Non-ELF without shebang: fall back to /bin/sh (execlp-like behavior).
            // // Guard against recursive /bin/sh -> /bin/sh fallback loops.
            // if exec_path == "/bin/sh" || exec_path_resolved == "/bin/sh" {
            //     warn!(
            //         "[sys_exec] non-ELF /bin/sh without shebang, stop recursive fallback for {}",
            //         exec_path_resolved
            //     );
            //     return errno(ENOEXEC);
            // }
            // if args.get(0).map(|s| s.as_str()) == Some("sh")
            //     && args.get(1).map(|s| s.as_str()) == Some("/bin/sh")
            // {
            //     warn!(
            //         "[sys_exec] detected recursive sh argv chain for {}, abort fallback",
            //         exec_path_resolved
            //     );
            //     return errno(ENOEXEC);
            // }
            if depth >= MAX_SHEBANG_DEPTH {
                return errno(ELOOP);
            }
            depth += 1;
            let mut new_args: Vec<String> = Vec::new();
            new_args.push(String::from("sh"));
            new_args.push(exec_path_resolved.clone());
            if args.len() > 1 {
                new_args.extend(args.into_iter().skip(1));
            }
            args = new_args;
            exec_path = String::from("/bin/sh");
            warn!(
                "[sys_exec] non-ELF without shebang, fallback to /bin/sh for {} with args {:?}. BE CAREFUL OF IT.for run-static.sh/run-dynamic.sh, it's fine",
                exec_path_resolved, args
            );
            continue;
        }
        let mut interp_data: Option<Arc<[u8]>> = None;
        if let Ok(elf) = xmas_elf::ElfFile::new(all_data.as_ref()) {
            let mut interp: Option<String> = None;
            for i in 0..elf.header.pt2.ph_count() {
                let ph = elf.program_header(i).unwrap();
                if ph.get_type().unwrap() == xmas_elf::program::Type::Interp {
                    let interp_start = ph.offset() as usize;
                    let interp_end = interp_start + ph.file_size() as usize;
                    if interp_end <= all_data.len() {
                        if let Ok(interp_str) =
                            core::str::from_utf8(&all_data[interp_start..interp_end])
                        {
                            interp = Some(String::from(interp_str.trim_end_matches('\0')));
                        }
                    }
                    break;
                }
            }
            if let Some(mut interp_path) = interp {
                if open_file(interp_path.as_str(), OpenFlags::empty()).is_none() {
                    // Fallback: try musl/glibc loader for known interpreter paths
                    let musl_loader = "/musl/lib/libc.so";
                    let glibc_loader = "/glibc/lib/ld-linux-riscv64-lp64d.so.1";
                    if interp_path == "/lib/ld-linux-riscv64-lp64d.so.1"
                        || interp_path.contains("ld-musl-riscv64")
                        || interp_path == "/lib64/ld-linux-loongarch-lp64d.so.1"
                        || interp_path.contains("ld-musl-loongarch")
                    {
                        if open_file(musl_loader, OpenFlags::empty()).is_some() {
                            info!(
                                "[sys_exec] interp {} not found, fallback to musl loader: {}",
                                interp_path, musl_loader
                            );
                            interp_path = String::from(musl_loader);
                        } else if open_file(glibc_loader, OpenFlags::empty()).is_some() {
                            info!(
                                "[sys_exec] interp {} not found, fallback to glibc loader: {}",
                                interp_path, glibc_loader
                            );
                            interp_path = String::from(glibc_loader);
                        }
                    }
                }
                if open_file(interp_path.as_str(), OpenFlags::empty()).is_none() {
                    trace!("[sys_exec] interp not found: {}", interp_path);
                    return errno(ENOENT);
                }
                if let Some(interp_file) = open_file(interp_path.as_str(), OpenFlags::empty()) {
                    interp_data = Some(read_exec_image(interp_path.as_str(), &interp_file));
                } else {
                    error!("[sys_exec] interp open failed: {}", interp_path);
                    return errno(ENOENT);
                }
            }
        }
        let process = current_process();
        {
            let name = exec_path
                .rsplit('/')
                .find(|part| !part.is_empty())
                .unwrap_or(exec_path.as_str());
            process.set_name(String::from(name));
            trace!(
                "[sys_exec] set process name to {} for path {}",
                name,
                exec_path
            );
        }
        // if exec_path == "/bin/sh" {
        //     let argv0 = args.get(0).cloned().unwrap_or_default();
        //     let argv1 = args.get(1).cloned().unwrap_or_default();
        //     trace!("[sys_exec] /bin/sh argv0={} argv1={}", argv0, argv1);
        // }
        process.exec_with_interp(all_data.as_ref(), interp_data.as_deref(), args, envs);
        let after_name = current_process().name();
        trace!("[sys_exec] after exec name={}", after_name);
        return 0;
    }
}

pub fn sys_exec(path: *const u8, argv: *const usize, envp: *const usize) -> isize {
    sys_exec_internal(path, argv, envp, 0)
}

fn encode_wait_status(exit_code: i32) -> i32 {
    if exit_code >= 0 {
        (exit_code & 0xff) << 8
    } else {
        let signum = (-exit_code) & 0x7f;
        let core = match signum {
            3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31 => 0x80,
            _ => 0,
        };
        signum | core
    }
}

const P_ALL: usize = 0;
const P_PID: usize = 1;
const P_PGID: usize = 2;

const WAITID_WNOHANG: i32 = 0x00000001;
const WAITID_WSTOPPED: i32 = 0x00000002;
const WAITID_WEXITED: i32 = 0x00000004;
const WAITID_WCONTINUED: i32 = 0x00000008;
const WAITID_WNOWAIT: i32 = 0x01000000;

const WAITID_CLD_EXITED: i32 = 1;
const WAITID_CLD_KILLED: i32 = 2;
const WAITID_CLD_DUMPED: i32 = 3;
const WAITID_CLD_STOPPED: i32 = 5;
const WAITID_CLD_CONTINUED: i32 = 6;

#[repr(C)]
#[derive(Clone, Copy)]
struct LinuxSigInfo {
    si_signo: i32,
    si_errno: i32,
    si_code: i32,
    _align_pad: i32,
    _pad: [u8; 112],
}

impl LinuxSigInfo {
    fn zeroed() -> Self {
        Self {
            si_signo: 0,
            si_errno: 0,
            si_code: 0,
            _align_pad: 0,
            _pad: [0; 112],
        }
    }

    fn for_sigchld(pid: i32, status: i32, code: i32) -> Self {
        let mut info = Self {
            si_signo: SIGCHLD as i32,
            si_errno: 0,
            si_code: code,
            _align_pad: 0,
            _pad: [0; 112],
        };
        info._pad[0..4].copy_from_slice(&pid.to_ne_bytes());
        info._pad[8..12].copy_from_slice(&status.to_ne_bytes());
        info
    }

    fn for_signal(signum: i32, sender_pid: i32, code: i32) -> Self {
        let mut info = Self {
            si_signo: signum,
            si_errno: 0,
            si_code: code,
            _align_pad: 0,
            _pad: [0; 112],
        };
        info._pad[0..4].copy_from_slice(&sender_pid.to_ne_bytes());
        info
    }
}

fn write_waitid_siginfo(info_ptr: *mut u8, info: LinuxSigInfo) -> Result<(), isize> {
    let token = current_user_token();
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&info as *const LinuxSigInfo) as *const u8,
            core::mem::size_of::<LinuxSigInfo>(),
        )
    };
    copy_to_user(token, info_ptr, bytes)
}

fn waitid_options_valid(options: i32) -> bool {
    let known =
        WAITID_WNOHANG | WAITID_WSTOPPED | WAITID_WEXITED | WAITID_WCONTINUED | WAITID_WNOWAIT;
    (options & !known) == 0
        && (options & (WAITID_WSTOPPED | WAITID_WEXITED | WAITID_WCONTINUED)) != 0
}

fn exit_code_to_waitid(exit_code: i32) -> (i32, i32) {
    if exit_code >= 0 {
        return (exit_code & 0xff, WAITID_CLD_EXITED);
    }
    let signum = (-exit_code) & 0x7f;
    let code = match signum {
        3 | 4 | 5 | 6 | 7 | 8 | 11 | 24 | 25 | 31 => WAITID_CLD_DUMPED,
        _ => WAITID_CLD_KILLED,
    };
    (signum, code)
}

pub(crate) fn has_unmasked_user_signal_without_restart() -> bool {
    let process = current_process();
    let task = current_task().unwrap();
    let (task_pending, signal_mask) =
        task.with_signals(|signals| (signals.signal_pending, signals.signal_mask));
    let unmasked = (process.pending_signal() | task_pending) & !signal_mask;
    if unmasked.is_empty() {
        return false;
    }
    use crate::task::SA_RESTART;
    let raw = unmasked.bits();
    for bit in 0..64u32 {
        if raw & (1u64 << bit) == 0 {
            continue;
        }
        let signum = bit as usize + 1;
        if signum >= MAX_SIG {
            continue;
        }
        let action = process.with_signal_action(signum, |action| action);
        if action.handler == 1 {
            continue; // SIG_IGN
        }
        if action.handler == 0 {
            // Default dispositions should still break wait loops unless the
            // signal is one of the Linux default-ignored ones (e.g. SIGCHLD).
            if signal_default_ignored(signum) {
                continue;
            }
            return true;
        }
        if (action.flags & SA_RESTART) != 0 {
            continue;
        }
        return true;
    }
    false
}

#[inline]
fn signal_default_ignored(signum: usize) -> bool {
    // Linux default ignored signals relevant for EINTR decisions in wait loops.
    matches!(signum, SIGCHLD | 23 | 28)
}

pub fn sys_waitid(idtype: usize, id: usize, infop: *mut u8, options: i32, _ru: *mut u8) -> isize {
    if infop.is_null() {
        return errno(EFAULT);
    }
    if !waitid_options_valid(options) {
        return errno(EINVAL);
    }
    if idtype != P_ALL && idtype != P_PID && idtype != P_PGID {
        return errno(EINVAL);
    }
    let want_exited = (options & WAITID_WEXITED) != 0;
    let want_stopped = (options & WAITID_WSTOPPED) != 0;
    let want_continued = (options & WAITID_WCONTINUED) != 0;
    let nohang = (options & WAITID_WNOHANG) != 0;
    let nowait = (options & WAITID_WNOWAIT) != 0;
    let caller_pgid = current_process().pgid();
    let target_pgid = if idtype == P_PGID && id == 0 {
        caller_pgid
    } else {
        id
    };

    loop {
        let process = current_process();
        let children = process.children_snapshot();
        let mut has_matching_child = false;

        // 1) report non-exit events first (WSTOPPED/WCONTINUED)
        for child in children.iter() {
            let child_pid = child.getpid();
            let matched = match idtype {
                P_ALL => true,
                P_PID => child_pid == id,
                P_PGID => child.pgid() == target_pgid,
                _ => false,
            };
            if !matched {
                continue;
            }
            has_matching_child = true;
            if let Some(event) = child.child_wait_event() {
                match event {
                    ChildWaitEvent::Stopped(sig) if want_stopped => {
                        let info =
                            LinuxSigInfo::for_sigchld(child_pid as i32, sig, WAITID_CLD_STOPPED);
                        if !nowait {
                            child.take_child_wait_event();
                        }
                        if write_waitid_siginfo(infop, info).is_err() {
                            return errno(EFAULT);
                        }
                        return 0;
                    }
                    ChildWaitEvent::Continued(sig) if want_continued => {
                        let info =
                            LinuxSigInfo::for_sigchld(child_pid as i32, sig, WAITID_CLD_CONTINUED);
                        if !nowait {
                            child.take_child_wait_event();
                        }
                        if write_waitid_siginfo(infop, info).is_err() {
                            return errno(EFAULT);
                        }
                        return 0;
                    }
                    _ => {}
                }
            }
        }

        // 2) report exit events
        if want_exited {
            let pair = children.iter().find(|child| {
                let child_pid = child.getpid();
                let matched = match idtype {
                    P_ALL => true,
                    P_PID => child_pid == id,
                    P_PGID => child.pgid() == target_pgid,
                    _ => false,
                };
                if matched {
                    has_matching_child = true;
                }
                matched && child.is_zombie()
            });
            if let Some(child) = pair {
                let (found_pid, exit_code, reaped) = if nowait {
                    let found_pid = child.getpid();
                    let exit_code = child.exit_code();
                    (found_pid, exit_code, false)
                } else {
                    let found_pid = child.getpid();
                    let exit_code = child.exit_code();
                    process.remove_child_by_pid(found_pid);
                    (found_pid, exit_code, true)
                };
                if reaped {
                    remove_from_pid2process(found_pid);
                }
                let (status, code) = exit_code_to_waitid(exit_code);
                let info = LinuxSigInfo::for_sigchld(found_pid as i32, status, code);
                if write_waitid_siginfo(infop, info).is_err() {
                    return errno(EFAULT);
                }
                return 0;
            }
        }

        if !has_matching_child {
            return errno(ECHILD);
        }

        // No reportable event.
        if nohang {
            let info = LinuxSigInfo::zeroed();
            if write_waitid_siginfo(infop, info).is_err() {
                return errno(EFAULT);
            }
            return 0;
        }
        suspend_current_and_run_next();
        if has_unmasked_user_signal_without_restart() {
            return errno(EINTR);
        }
    }
}

/// wait4 syscall: wait for child process state changes.
/// options: WNOHANG (1) = return immediately if no zombie child.
/// Returns child pid on success, 0 if WNOHANG and no zombie, -ECHILD if no matching child.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32, options: i32) -> isize {
    const WNOHANG: i32 = 1;
    const WUNTRACED: i32 = 2;
    let my_pid = current_process().getpid();
    let my_pgid = current_process().pgid();
    #[allow(dead_code)]
    const SIG_DFL: usize = 0;
    #[allow(dead_code)]
    const SIG_IGN: usize = 1;

    // Helper: check if child matches the pid criteria
    let matches_pid = |child_pid: usize, child_pgid: usize| -> bool {
        match pid {
            -1 => true,                            // Any child
            0 => child_pgid == my_pgid,            // Same process group
            p if p > 0 => child_pid == p as usize, // Specific PID
            p => child_pgid == (-p) as usize,      // Specific process group (pid < -1)
        }
    };

    loop {
        let process = current_process();
        let children = process.children_snapshot();

        // Check if any child matches the criteria
        let has_matching_child = children.iter().any(|p| matches_pid(p.getpid(), p.pgid()));

        if !has_matching_child {
            return errno(ECHILD);
        }

        // First, check for ptrace-stopped children (higher priority than zombies)
        let ptrace_child = children
            .iter()
            .find(|p| p.ptrace_stop_signal().is_some() && matches_pid(p.getpid(), p.pgid()));
        if let Some(child) = ptrace_child {
            let found_pid = child.getpid();

            if let Some(signum) = child.take_ptrace_stop_signal() {
                if !exit_code_ptr.is_null() {
                    // WIFSTOPPED encoding: (signal << 8) | 0x7f
                    let status = ((signum & 0xff) << 8) | 0x7f;
                    *translated_refmut(current_user_token(), exit_code_ptr) = status;
                }
                trace!(
                    "[sys_waitpid] pid={} reports ptrace-stop of child pid={} signal={}",
                    my_pid,
                    found_pid,
                    signum
                );
                return found_pid as isize;
            }
        }

        // Then, if caller requests WUNTRACED, report group-stopped children.
        if (options & WUNTRACED) != 0 {
            let stopped_pair = children.iter().find_map(|p| {
                if !matches_pid(p.getpid(), p.pgid()) {
                    return None;
                }
                let stop_sig = match p.take_child_wait_event() {
                    Some(ChildWaitEvent::Stopped(sig)) => Some(sig),
                    Some(other) => {
                        // Keep non-stopped event for waitid()/other wait semantics.
                        p.set_child_wait_event(Some(other));
                        None
                    }
                    None => None,
                };
                stop_sig.map(|sig| (p.clone(), sig))
            });
            if let Some((child, signum)) = stopped_pair {
                let found_pid = child.getpid();
                if !exit_code_ptr.is_null() {
                    let stop_sig = if (1..=(MAX_SIG as i32)).contains(&signum) {
                        signum
                    } else {
                        SIGSTOP as i32
                    };
                    // WIFSTOPPED encoding.
                    let status = ((stop_sig & 0xff) << 8) | 0x7f;
                    *translated_refmut(current_user_token(), exit_code_ptr) = status;
                }
                trace!(
                    "[sys_waitpid] pid={} reports group-stop of child pid={} signal={}",
                    my_pid,
                    found_pid,
                    signum
                );
                return found_pid as isize;
            }
        }

        // Then check for zombie children
        let pair = children
            .iter()
            .find(|p| p.is_zombie() && matches_pid(p.getpid(), p.pgid()));
        if let Some(child) = pair {
            let found_pid = child.getpid();
            let exit_code = child.exit_code();
            process.remove_child_by_pid(found_pid);
            remove_from_pid2process(found_pid);
            if !exit_code_ptr.is_null() {
                let status = encode_wait_status(exit_code);
                *translated_refmut(current_user_token(), exit_code_ptr) = status;
            }
            trace!(
                "[sys_waitpid] pid={} reaped child pid={} exit_code={}",
                my_pid,
                found_pid,
                exit_code
            );
            return found_pid as isize;
        }

        // WNOHANG: return 0 immediately if no zombie child
        if (options & WNOHANG) != 0 {
            return 0;
        }

        // Yield and retry (busy-wait until child exits)
        suspend_current_and_run_next();

        // Check for pending signals that have a user handler -> return EINTR.
        // Signals with SIG_DFL (default-ignore, like SIGCHLD) should NOT cause EINTR.
        if has_unmasked_user_signal_without_restart() {
            return errno(EINTR);
        }
    }
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(_ts: *mut TimeVal, _tz: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_get_time", pid);
    }
    // Validate tz pointer: non-null tz must be a valid user address.
    // Linux returns EFAULT for invalid (e.g., -1) tz even though timezone is deprecated.
    if _tz != 0 && _tz >= crate::config::USER_STACK_TOP {
        return errno(EFAULT);
    }
    if _ts.is_null() {
        return errno(EFAULT);
    }
    let us = get_time_us();
    let tv = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&tv as *const TimeVal) as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, _ts as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

fn realtime_now_us() -> i64 {
    (get_time_us() as i64).saturating_add(*REALTIME_OFFSET_US.exclusive_access())
}

pub fn sys_clock_settime(clock_id: usize, ts: *const TimeSpec) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_clock_settime", pid);
    }
    if clock_id != 0 {
        return errno(EINVAL);
    }
    if ts.is_null() {
        return errno(EFAULT);
    }
    let process = current_process();
    let euid = process.effective_uid();
    if euid != 0 {
        return errno(EPERM);
    }
    let token = current_user_token();
    let spec = match read_from_user(token, ts) {
        Ok(spec) => spec,
        Err(err) => return err,
    };
    if spec.tv_nsec >= 1_000_000_000 {
        return errno(EINVAL);
    }
    let target_us = match spec
        .tv_sec
        .checked_mul(1_000_000)
        .and_then(|v| v.checked_add(spec.tv_nsec / 1_000))
        .and_then(|v| i64::try_from(v).ok())
    {
        Some(v) => v,
        None => return errno(EINVAL),
    };
    let now_us = get_time_us() as i64;
    *REALTIME_OFFSET_US.exclusive_access() = target_us.saturating_sub(now_us);
    0
}

pub fn sys_clock_gettime(clock_id: usize, ts: *mut TimeSpec) -> isize {
    if ts.is_null() {
        return errno(EFAULT);
    }
    match clock_id {
        // CLOCK_REALTIME/CLOCK_MONOTONIC and common glibc probes.
        // We currently map them to the same wall-clock source.
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 11 => {
            let us = if matches!(clock_id, 0 | 5 | 8) {
                realtime_now_us().max(0) as usize
            } else {
                get_time_us()
            };
            let spec = TimeSpec {
                tv_sec: us / 1_000_000,
                tv_nsec: (us % 1_000_000) * 1_000,
            };
            let token = current_user_token();
            match user_mem::write_value_to_user(
                token,
                ts,
                spec,
                UserWritePolicy::RelaxedReadableMapping,
            ) {
                Ok(_) => 0,
                Err(err) => err,
            }
        }
        _ => errno(EINVAL),
    }
}

pub fn sys_clock_getres(clock_id: usize, res: *mut TimeSpec) -> isize {
    match clock_id {
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 11 => {
            if res.is_null() {
                return 0;
            }
            let spec = TimeSpec {
                tv_sec: 0,
                tv_nsec: 1000, // 1µs resolution
            };
            let token = current_user_token();
            match user_mem::write_value_to_user(
                token,
                res,
                spec,
                UserWritePolicy::RelaxedReadableMapping,
            ) {
                Ok(_) => 0,
                Err(err) => err,
            }
        }
        _ => errno(EINVAL),
    }
}

pub fn sys_nanosleep(req: *const TimeSpec, rem: *mut TimeSpec) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_nanosleep", pid);
    }
    let token = current_user_token();
    let req = match read_from_user::<TimeSpec>(token, req) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if req.tv_nsec >= 1_000_000_000 {
        return errno(EINVAL);
    }
    let sleep_us = req
        .tv_sec
        .saturating_mul(1_000_000)
        .saturating_add(req.tv_nsec / 1_000);
    let target = get_time_us().saturating_add(sleep_us);

    let unmasked_pending_signal = || -> Option<(SignalFlags, SignalFlags, SignalFlags)> {
        let process = current_process();
        let task = match current_task() {
            Some(task) => task,
            None => return None,
        };
        let process_pending = process.pending_signal();
        let (task_pending, signal_mask) =
            task.with_signals(|signals| (signals.signal_pending, signals.signal_mask));
        let pending = (process_pending | task_pending) & !signal_mask;
        if pending.is_empty() {
            None
        } else {
            Some((task_pending, process_pending, signal_mask))
        }
    };

    while get_time_us() < target {
        if let Some((task_pending, process_pending, signal_mask)) = unmasked_pending_signal() {
            let unmasked = (task_pending | process_pending) & !signal_mask;
            let cancel_related = unmasked & (SignalFlags::SIG32 | SignalFlags::SIG33);
            if !cancel_related.is_empty() {
                info!(
                    "[sys_nanosleep] pid={} EINTR by pending signal: unmasked={:?} task_pending={:?} proc_pending={:?} mask={:?}",
                    pid,
                    unmasked,
                    task_pending,
                    process_pending,
                    signal_mask
                );
            } else {
                trace!(
                    "[sys_nanosleep] pid={} EINTR by pending signal: unmasked={:?}",
                    pid,
                    unmasked
                );
            }
            if !rem.is_null() {
                let now = get_time_us();
                let remain_us = target.saturating_sub(now);
                let remain = TimeSpec {
                    tv_sec: remain_us / 1_000_000,
                    tv_nsec: (remain_us % 1_000_000) * 1_000,
                };
                let bytes = unsafe {
                    core::slice::from_raw_parts(
                        (&remain as *const TimeSpec) as *const u8,
                        core::mem::size_of::<TimeSpec>(),
                    )
                };
                if let Err(err) = copy_to_user(token, rem as *mut u8, bytes) {
                    return err;
                }
            }
            return errno(EINTR);
        }
        suspend_current_and_run_next();
    }
    if !rem.is_null() {
        let zero = TimeSpec::default();
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&zero as *const TimeSpec) as *const u8,
                core::mem::size_of::<TimeSpec>(),
            )
        };
        let _ = copy_to_user(token, rem as *mut u8, bytes);
    }
    0
}

pub fn sys_getitimer(which: isize, curr_value: *mut ITimerVal) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_getitimer which={}", pid, which);
    }
    if !(0..=2).contains(&which) {
        return errno(EINVAL);
    }
    if curr_value.is_null() {
        return errno(EFAULT);
    }
    let timer = {
        let process = current_process();
        process.with_timers(|timers| {
            if which == 0 {
                // ITIMER_REAL: compute actual remaining time from expire deadline
                let expire_ms = timers.itimer_real_expire_ms;
                let remaining_us = if expire_ms == 0 {
                    0usize
                } else {
                    let now_ms = get_time_ms();
                    if expire_ms > now_ms {
                        (expire_ms - now_ms) * 1000
                    } else {
                        0
                    }
                };
                ITimerVal {
                    it_interval: us_to_timeval(timers.itimer_real_interval_ms * 1000),
                    it_value: us_to_timeval(remaining_us),
                }
            } else {
                itimer_state_to_user(timers.itimers[which as usize])
            }
        })
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&timer as *const ITimerVal) as *const u8,
            core::mem::size_of::<ITimerVal>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, curr_value as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_setitimer(
    which: isize,
    new_value: *const ITimerVal,
    old_value: *mut ITimerVal,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_setitimer which={}", pid, which);
    }
    if !(0..=2).contains(&which) {
        return errno(EINVAL);
    }
    let token = current_user_token();
    let new_timer = match read_from_user::<ITimerVal>(token, new_value) {
        Ok(timer) => timer,
        Err(err) => return err,
    };
    let new_state = itimer_state_from_user(new_timer);
    let old_timer = {
        let process = current_process();
        process.with_timers_mut(|timers| {
            let old_timer = if which == 0 {
                // ITIMER_REAL: compute actual remaining time
                let expire_ms = timers.itimer_real_expire_ms;
                let remaining_us = if expire_ms == 0 {
                    0usize
                } else {
                    let now_ms = get_time_ms();
                    if expire_ms > now_ms {
                        (expire_ms - now_ms) * 1000
                    } else {
                        0
                    }
                };
                // Keep user-visible interval precision from the canonical itimer state,
                // instead of the millisecond scheduling cache field.
                let interval_us = timers.itimers[0].interval_us;
                ITimerVal {
                    it_interval: us_to_timeval(interval_us),
                    it_value: us_to_timeval(remaining_us),
                }
            } else {
                itimer_state_to_user(timers.itimers[which as usize])
            };
            timers.itimers[which as usize] = new_state;
            // Update itimer_real_expire_ms for ITIMER_REAL
            if which == 0 {
                let now_ms = get_time_ms();
                let remaining_us = new_state.remaining_us;
                if remaining_us == 0 {
                    timers.itimer_real_expire_ms = 0;
                } else {
                    timers.itimer_real_expire_ms = now_ms + remaining_us / 1000;
                }
                timers.itimer_real_interval_ms = new_state.interval_us / 1000;
            }
            old_timer
        })
    };
    if !old_value.is_null() {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old_timer as *const ITimerVal) as *const u8,
                core::mem::size_of::<ITimerVal>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_value as *mut u8, bytes) {
            return err;
        }
    }
    0
}

pub fn sys_sched_getaffinity(pid: isize, cpusetsize: usize, mask: *mut u8) -> isize {
    let pid_now = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid_now) {
        syscall!(
            "kernel:pid[{}] sys_sched_getaffinity pid={} cpusetsize={}",
            pid_now,
            pid,
            cpusetsize
        );
    }
    if cpusetsize == 0 {
        return errno(EINVAL);
    }
    if mask.is_null() {
        return errno(EFAULT);
    }
    if pid > 0 && pid2process(pid as usize).is_none() {
        return errno(ESRCH);
    }
    let mut out = vec![0u8; cpusetsize];
    out[0] = 1;
    let token = current_user_token();
    match copy_to_user(token, mask, &out) {
        Ok(_) => cpusetsize as isize,
        Err(err) => err,
    }
}

pub fn sys_clock_nanosleep(
    clock_id: usize,
    flags: usize,
    req: *const TimeSpec,
    rem: *mut TimeSpec,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_clock_nanosleep", pid);
    }
    // Support common clocks; reject thread CPU clock like Linux.
    match clock_id {
        0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 11 => {}
        _ => return errno(EINVAL),
    }
    if clock_id == 3 {
        return errno(ENOTSUP);
    }
    // Only relative sleep is supported for now.
    if flags != 0 {
        return errno(EINVAL);
    }
    sys_nanosleep(req, rem)
}

pub fn sys_times(tms: *mut Tms) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_times", pid);
    }
    let ticks = (get_time() * 100 / CLOCK_FREQ) as i64;
    if !tms.is_null() {
        let tms_val = Tms {
            tms_utime: ticks,
            tms_stime: 0,
            tms_cutime: 0,
            tms_cstime: 0,
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&tms_val as *const Tms) as *const u8,
                core::mem::size_of::<Tms>(),
            )
        };
        let token = current_user_token();
        if let Err(err) = copy_to_user(token, tms as *mut u8, bytes) {
            return err;
        }
    }
    ticks as isize
}

pub fn sys_getrusage(who: isize, usage: *mut RUsage) -> isize {
    if usage.is_null() {
        return errno(EFAULT);
    }
    // Linux accepts RUSAGE_SELF(0), RUSAGE_CHILDREN(-1), RUSAGE_THREAD(1).
    if who != 0 && who != -1 && who != 1 {
        return errno(EINVAL);
    }
    let us = get_time_us() as i64;
    let mut ru = RUsage::default();
    ru.ru_utime = RUsageTimeVal {
        tv_sec: us / 1_000_000,
        tv_usec: us % 1_000_000,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&ru as *const RUsage) as *const u8,
            core::mem::size_of::<RUsage>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, usage as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_uname(uts: *mut UtsName) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_uname", pid);
    }
    if uts.is_null() {
        return errno(EFAULT);
    }
    let mut uname = UtsName::default();
    fn fill(dst: &mut [u8], s: &str) {
        let bytes = s.as_bytes();
        let len = bytes.len().min(dst.len() - 1);
        dst[..len].copy_from_slice(&bytes[..len]);
        dst[len] = 0;
    }
    fill(&mut uname.sysname, "Linux");
    let uts_state = UTS_STATE.exclusive_access();
    fill(&mut uname.nodename, uts_state.0.as_str());
    fill(&mut uname.release, "5.10.0");
    fill(&mut uname.version, "rcore");
    fill(&mut uname.machine, arch::machine_name());
    fill(&mut uname.domainname, uts_state.1.as_str());
    drop(uts_state);
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&uname as *const UtsName) as *const u8,
            core::mem::size_of::<UtsName>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, uts as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

fn sys_set_uts_name(name: *const u8, len: usize, is_hostname: bool) -> isize {
    const UTS_NAME_MAX: usize = 64;
    if len > UTS_NAME_MAX {
        return errno(EINVAL);
    }
    if name.is_null() {
        return errno(EFAULT);
    }

    let process = current_process();
    if process.effective_uid() != 0 {
        return errno(EPERM);
    }

    let token = current_user_token();
    let mut bytes = [0u8; UTS_NAME_MAX];
    if len > 0
        && user_mem::copy_from_user(
            token,
            name,
            &mut bytes[..len],
            UserReadPolicy::StrictChecked,
        )
        .is_err()
    {
        return errno(EFAULT);
    }

    let new_name = core::str::from_utf8(&bytes[..len]).unwrap_or("").into();
    let mut uts_state = UTS_STATE.exclusive_access();
    if is_hostname {
        uts_state.0 = new_name;
    } else {
        uts_state.1 = new_name;
    }
    0
}

pub fn sys_sethostname(name: *const u8, len: usize) -> isize {
    sys_set_uts_name(name, len, true)
}

pub fn sys_setdomainname(name: *const u8, len: usize) -> isize {
    sys_set_uts_name(name, len, false)
}

fn user_range_in_area(start: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let process = current_process();
    process.with_memory_set(|memory_set| memory_set.is_user_range_mapped_area(start, len))
}

fn user_page_resident_bitmap(start: usize, pages: usize) -> Vec<u8> {
    let process = current_process();
    process.with_memory_set(|memory_set| {
        let mut out = Vec::new();
        for i in 0..pages {
            let va = start.saturating_add(i * PAGE_SIZE);
            out.push(
                if memory_set.translate(VirtAddr::from(va).floor()).is_some() {
                    1
                } else {
                    0
                },
            );
        }
        out
    })
}

pub fn sys_mlock(addr: usize, len: usize) -> isize {
    const RLIMIT_MEMLOCK: usize = 8;
    if len == 0 {
        return 0;
    }
    let Some(_end) = addr.checked_add(len) else {
        return errno(ENOMEM);
    };
    let process = current_process();
    let euid = process.effective_uid();
    let limit = process.with_limits(|limits| limits.rlimits[RLIMIT_MEMLOCK].rlim_cur as usize);
    if euid != 0 {
        if limit == 0 {
            return errno(EPERM);
        }
        if len > limit {
            return errno(ENOMEM);
        }
    }
    if !user_range_in_area(addr, len) {
        return errno(ENOMEM);
    }
    let readable = user_mem::ensure_user_readable(
        current_user_token(),
        addr as *const u8,
        len,
        UserReadPolicy::DemandPaged,
    );
    let writable = readable
        || user_mem::ensure_user_writable(
            current_user_token(),
            addr as *const u8,
            len,
            UserWritePolicy::DemandCowWithForkFallback,
        );
    if !writable {
        return errno(ENOMEM);
    }
    0
}

pub fn sys_munlock(_addr: usize, _len: usize) -> isize {
    0
}

pub fn sys_mlockall(_flags: usize) -> isize {
    0
}

pub fn sys_munlockall() -> isize {
    0
}

pub fn sys_mincore(addr: usize, len: usize, vec_ptr: *mut u8) -> isize {
    if (addr & (PAGE_SIZE - 1)) != 0 {
        return errno(EINVAL);
    }
    if len == 0 {
        return 0;
    }
    let Some(end) = addr.checked_add(len) else {
        return errno(ENOMEM);
    };
    if addr >= USER_ADDR_MAX || end > USER_ADDR_MAX {
        return errno(ENOMEM);
    }
    if !user_range_in_area(addr, len) {
        return errno(ENOMEM);
    }
    if vec_ptr.is_null() {
        return errno(EFAULT);
    }
    let Some(pages) = len
        .checked_add(PAGE_SIZE - 1)
        .map(|rounded| rounded / PAGE_SIZE)
    else {
        return errno(ENOMEM);
    };
    if !user_mem::ensure_user_writable(
        current_user_token(),
        vec_ptr,
        pages,
        UserWritePolicy::DemandCowWithForkFallback,
    ) {
        return errno(EFAULT);
    }
    let present = user_page_resident_bitmap(addr, pages);
    match copy_to_user(current_user_token(), vec_ptr, present.as_slice()) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_syslog(_log_type: usize, buf: *mut u8, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_syslog", pid);
    }
    let log_type = _log_type as isize;
    match log_type {
        0..=10 => {}
        _ => return errno(EINVAL),
    }
    if current_process().effective_uid() != 0 {
        return errno(EPERM);
    }
    if log_type == 8 && len > 8 {
        return errno(EINVAL);
    }
    if len > isize::MAX as usize {
        return errno(EINVAL);
    }
    if buf.is_null() {
        return errno(EINVAL);
    }
    if len == 0 {
        return 0;
    }
    let token = current_user_token();
    let out = [0u8; 1];
    match copy_to_user(token, buf, &out) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_sysinfo(info: *mut SysInfo) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sysinfo", pid);
    }
    if info.is_null() {
        return errno(EFAULT);
    }
    let us = get_time_us();
    let data = SysInfo {
        uptime: (us / 1_000_000) as i64,
        loads: [0, 0, 0],
        totalram: 0,
        freeram: 0,
        sharedram: 0,
        bufferram: 0,
        totalswap: 0,
        freeswap: 0,
        procs: 1,
        pad: 0,
        totalhigh: 0,
        freehigh: 0,
        mem_unit: 1,
    };
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&data as *const SysInfo) as *const u8,
            core::mem::size_of::<SysInfo>(),
        )
    };
    let token = current_user_token();
    match copy_to_user(token, info as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_msync(start: usize, len: usize, _flags: usize) -> isize {
    const MS_ASYNC: usize = 0x1;
    const MS_INVALIDATE: usize = 0x2;
    const MS_SYNC: usize = 0x4;

    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_msync start={:#x} len={:#x}",
            pid,
            start,
            len
        );
    }
    if start == 0 || len == 0 {
        return errno(EINVAL);
    }
    if (start & (PAGE_SIZE - 1)) != 0 {
        return errno(EINVAL);
    }
    if _flags & !(MS_ASYNC | MS_INVALIDATE | MS_SYNC) != 0 {
        return errno(EINVAL);
    }
    if (_flags & MS_ASYNC) != 0 && (_flags & MS_SYNC) != 0 {
        return errno(EINVAL);
    }
    let Some(end) = start.checked_add(len) else {
        return errno(EINVAL);
    };
    let ms_invalidate = (_flags & MS_INVALIDATE) != 0;
    let ms_writeback = (_flags & (MS_SYNC | MS_ASYNC)) != 0;
    let process = current_process();
    match process.with_memory_set_mut(|memory_set| {
        memory_set.msync_file_range(
            VirtAddr::from(start),
            VirtAddr::from(end),
            ms_invalidate,
            ms_writeback,
        )
    }) {
        Ok(_) => 0,
        Err(crate::mm::MsyncError::Busy) => errno(EBUSY),
        Err(crate::mm::MsyncError::Unmapped) => errno(ENOMEM),
    }
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(
    start: usize,
    len: usize,
    prot: usize,
    flags: usize,
    fd: usize,
    offset: usize,
) -> isize {
    const PROT_READ: usize = 0x1;
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;
    const MAP_SHARED: usize = 0x01;
    const MAP_PRIVATE: usize = 0x02;
    const MAP_FIXED: usize = 0x10;
    const MAP_ANON: usize = 0x20;
    const MAP_LOCKED: usize = 0x2000;
    const MAP_TYPE_MASK: usize = MAP_SHARED | MAP_PRIVATE;

    let pid = current_process().pid.0;
    let proc_name = current_process().name();
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_mmap", pid);
    }
    if proc_name == "entry-dynamic.exe" && (flags & MAP_ANON) == 0 && fd != usize::MAX {
        info!(
            "[mmap-file] pid={} start={:#x} len={:#x} prot={:#x} flags={:#x} fd={} off={:#x}",
            pid, start, len, prot, flags, fd, offset
        );
    }

    let mut len = len;
    if len == 0 {
        warn!(
            "kernel:pid[{}] sys_mmap with zero length, treating as 1 page",
            pid
        );
        //// 针对 musl busybox 的问题排查代码
        // let cx = current_trap_cx();
        // let sepc = cx.sepc;
        // let ra = cx[TrapFrameArgs::RA];
        // let gp = cx.gp();
        // let process = current_process();
        // let name = process.name();
        // let (heap_bottom, program_brk, mmap_base) =
        //     process.with_memory(|memory| (memory.heap_bottom, memory.program_brk, memory.mmap_base));
        // let token = current_user_token();
        // let heap_struct = gp.wrapping_add(0x688);
        // let heap_align = *translated_ref(token, (heap_struct + 0x38) as *const usize);
        // let heap_min = *translated_ref(token, (heap_struct + 0x10) as *const usize);
        // let malloc_shift = *translated_ref(token, (gp + 0x140) as *const u32);
        if (flags & MAP_ANON) != 0 && (flags & MAP_PRIVATE) != 0 {
            // trace!(
            //     "[sys_mmap] pid={} name={} sepc={:#x} ra={:#x} gp={:#x} req={:#x} len=0 prot={:#x} flags={:#x} fd={} off={:#x} hb={:#x} brk={:#x} mmap_base={:#x} heap_align={:#x} heap_min={:#x} malloc_shift={} -> compat map 1 page",
            //     pid,
            //     name,
            //     sepc,
            //     ra,
            //     gp,
            //     start,
            //     prot,
            //     flags,
            //     fd,
            //     offset,
            //     heap_bottom,
            //     program_brk,
            //     mmap_base,
            //     heap_align,
            //     heap_min,
            //     malloc_shift
            // );
            len = PAGE_SIZE;
        } else {
            // trace!(
            //     "[sys_mmap] pid={} name={} sepc={:#x} ra={:#x} gp={:#x} req={:#x} len=0 prot={:#x} flags={:#x} fd={} off={:#x} hb={:#x} brk={:#x} mmap_base={:#x} heap_align={:#x} heap_min={:#x} malloc_shift={} -> EINVAL",
            //     pid,
            //     name,
            //     sepc,
            //     ra,
            //     gp,
            //     start,
            //     prot,
            //     flags,
            //     fd,
            //     offset,
            //     heap_bottom,
            //     program_brk,
            //     mmap_base,
            //     heap_align,
            //     heap_min,
            //     malloc_shift
            // );
            return errno(EINVAL);
        }
    }

    if start % PAGE_SIZE != 0 && (flags & MAP_FIXED) != 0 {
        error!(
            "start addr should be page aligned when MAP_FIXED is set, but got {:#x} in pid {}",
            pid, start
        );
        return errno(EINVAL);
    }
    let mut map_perm = MapPermission::U;
    if (prot & PROT_READ) != 0 {
        map_perm |= MapPermission::R;
    }
    if (prot & PROT_WRITE) != 0 {
        map_perm |= MapPermission::W;
    }
    if (prot & PROT_EXEC) != 0 {
        map_perm |= MapPermission::X;
    }
    // 获取页对齐的长度，如果 len 已经是页大小的整数倍，则保持不变；否则向上调整到下一个页边界。
    let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    if (flags & MAP_TYPE_MASK) == 0 || (flags & MAP_TYPE_MASK) == MAP_TYPE_MASK {
        return errno(EINVAL);
    }
    if (prot & !(PROT_READ | PROT_WRITE | PROT_EXEC)) != 0 {
        return errno(EINVAL);
    }

    let is_shared = (flags & MAP_SHARED) != 0;
    let is_anon = (flags & MAP_ANON) != 0;
    let process = current_process();

    let file_for_mapping = if !is_anon {
        if offset % PAGE_SIZE != 0 {
            return errno(EINVAL);
        }
        let Some(file) = process.get_file(fd) else {
            return errno(EBADF);
        };
        if map_perm.contains(MapPermission::R) && !file.readable() {
            return errno(EACCES);
        }
        if is_shared && map_perm.contains(MapPermission::W) && !file.writable() {
            return errno(EACCES);
        }
        if is_shared
            && map_perm.contains(MapPermission::W)
            && file.get_seals().is_some_and(|seals| (seals & 0x0008) != 0)
        {
            return errno(EPERM);
        }
        Some(file)
    } else {
        None
    };
    let file_writable = file_for_mapping
        .as_ref()
        .map(|file| file.writable())
        .unwrap_or(true);

    // 计算 mmap 起始地址：
    // - MAP_FIXED: 使用调用方给定地址
    // - 非 MAP_FIXED: 从 hint/mmap_base 出发，向上跳过所有重叠区间后再放置
    let req_start = start;
    let is_fixed = (flags & MAP_FIXED) != 0 && req_start != 0;
    let mmap_result = process.with_memory_mut(|memory| -> Result<usize, isize> {
        let mut start = if is_fixed {
            req_start
        } else if req_start != 0 {
            (req_start + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
        } else {
            (memory.mmap_base + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
        };

        if !is_fixed {
            loop {
                let end = match start.checked_add(len) {
                    Some(v) => v,
                    None => return Err(errno(ENOMEM)),
                };
                let overlaps = memory
                    .memory_set
                    .overlap_ranges(VirtAddr(start), VirtAddr(end));
                if overlaps.is_empty() {
                    break;
                }
                let jump_to = overlaps
                    .iter()
                    .map(|(_, r_end)| r_end.0)
                    .max()
                    .unwrap_or(start);
                let next = (jump_to + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
                if next <= start {
                    return Err(errno(ENOMEM));
                }
                start = next;
            }
            let end = match start.checked_add(len) {
                Some(v) => v,
                None => return Err(errno(ENOMEM)),
            };
            if end > memory.mmap_base {
                memory.mmap_base = end;
            }
        }

        // TODO(grl): harden mmap end-range validation in one place for both MAP_FIXED
        // and non-fixed paths:
        // 1) compute/check `end = start.checked_add(len)` once;
        // 2) enforce `end <= USER_ADDR_MAX`;
        // 3) stop using raw `start + len` in overlap/unmap/insert calls.
        let mmap_proc_name = proc_name.as_str();
        if mmap_proc_name == "busybox" || mmap_proc_name == "ld-linux-riscv64-lp64d.so.1" {
            let overlap = memory
                .memory_set
                .overlap_count(VirtAddr(start), VirtAddr(start + len));
            trace!(
                "[sys_mmap] pid={} name={} req={:#x} len={:#x} prot={:#x} flags={:#x} -> start={:#x} overlap={} fixed={}",
                pid,
                mmap_proc_name,
                req_start,
                len,
                prot,
                flags,
                start,
                overlap,
                is_fixed
            );
            if is_fixed && overlap > 0 {
                let ranges = memory
                    .memory_set
                    .overlap_ranges(VirtAddr(start), VirtAddr(start + len));
                for (idx, (r_start, r_end)) in ranges.into_iter().enumerate() {
                    trace!(
                        "[sys_mmap] pid={} fixed overlap[{}]=[{:#x},{:#x})",
                        pid,
                        idx,
                        r_start.0,
                        r_end.0
                    );
                }
            }
        }
        trace!(
            "[sys_mmap] pid={} req={:#x} len={:#x} flags={:#x} -> start={:#x}",
            pid,
            req_start,
            len,
            flags,
            start
        );

        let overlap = memory
            .memory_set
            .overlap_count(VirtAddr(start), VirtAddr(start + len));
        if overlap > 0 {
            if is_fixed {
                // MAP_FIXED: unmap overlapping pages in the target range.
                memory
                    .memory_set
                    .unmap_range(VirtAddr(start), VirtAddr(start + len));
            } else {
                // Non-MAP_FIXED mappings must not overlap existing VMAs.
                return Err(errno(ENOMEM));
            }
        }

        // Lazy VMA insertion: register VMA, allocate pages on fault.
        // Preserve MapAreaType tracking from dev for bookkeeping.
        let meta = MmapMeta {
            shared: is_shared,
            file_backed: !is_anon && fd != usize::MAX,
            file_writable,
            map_locked: (flags & MAP_LOCKED) != 0,
        };
        if is_anon || fd == usize::MAX {
            if is_shared {
                // MAP_SHARED anonymous: eagerly allocate so that all forked processes
                // share the same physical frames. Lazy allocation creates per-process
                // pages on fault, breaking MAP_SHARED semantics (e.g. getpid02 test).
                memory.memory_set.insert_mmap_area(
                    VirtAddr(start),
                    VirtAddr(start + len),
                    map_perm,
                    meta,
                    MapAreaType::MmapAnon,
                );
            } else {
                memory.memory_set.insert_lazy_anon_area(
                    VirtAddr(start),
                    VirtAddr(start + len),
                    map_perm,
                    meta,
                );
            }
        } else {
            match file_for_mapping.clone() {
                Some(file) => {
                    if is_shared {
                        // MAP_SHARED file-backed mappings must materialize shared
                        // frames before fork(). Keeping them lazy can cause parent
                        // and child to fault different private pages, corrupting
                        // shared userspace state (e.g. LTP summary counters).
                        memory.memory_set.insert_shared_file_mmap_area(
                            VirtAddr(start),
                            VirtAddr(start + len),
                            map_perm,
                            file,
                            offset as u64,
                            meta,
                        );
                    } else {
                        memory.memory_set.insert_lazy_file_area(
                            VirtAddr(start),
                            VirtAddr(start + len),
                            map_perm,
                            file,
                            offset as u64,
                            meta,
                        );
                    }
                }
                None => return Err(errno(EBADF)),
            }
        }
        Ok(start)
    });

    match mmap_result {
        Ok(start) => start as isize,
        Err(err) => err,
    }
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_munmap", pid);
    }
    if start % PAGE_SIZE != 0 || len == 0 {
        return errno(EINVAL);
    }
    let len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let process = current_process();
    // if inner.name == "busybox" || inner.name == "ld-linux-riscv64-lp64d.so.1" {
    //     let before = inner
    //         .memory_set
    //         .overlap_count(VirtAddr(start), VirtAddr(start + len));
    //     trace!(
    //         "[sys_munmap] pid={} name={} start={:#x} len={:#x} overlap_before={}",
    //         pid,
    //         inner.name,
    //         start,
    //         len,
    //         before
    //     );
    //     if before > 0 {
    //         let ranges = inner
    //             .memory_set
    //             .overlap_ranges(VirtAddr(start), VirtAddr(start + len));
    //         for (idx, (r_start, r_end)) in ranges.into_iter().enumerate() {
    //             trace!(
    //                 "[sys_munmap] pid={} overlap[{}]=[{:#x},{:#x})",
    //                 pid,
    //                 idx,
    //                 r_start.0,
    //                 r_end.0
    //             );
    //         }
    //     }
    // }
    let aligned_len = (len + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    process.with_memory_set_mut(|memory_set| {
        memory_set.unmap_range(VirtAddr(start), VirtAddr(start + aligned_len));
    });
    0
}

/// change data segment size
pub fn sys_sbrk(arg: isize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sbrk", pid);
    }
    let sepc = current_trap_cx().sepc;
    let name = current_process().name();
    let process = current_process();
    let (current_brk, heap_bottom) =
        process.with_memory(|memory| (memory.program_brk, memory.heap_bottom));
    if arg == 0 {
        // if name == "busybox" {
        //     trace!(
        //         "[sys_sbrk] pid={} name={} sepc={:#x} arg=0 cur={:#x} heap_bottom={:#x}",
        //         pid,
        //         name,
        //         sepc,
        //         current_brk,
        //         heap_bottom
        //     );
        // }
        return current_brk as isize;
    }
    let is_abs = (arg as usize) >= heap_bottom;
    let delta = if is_abs {
        arg - current_brk as isize
    } else {
        // sbrk(delta) is equivalent to brk(current_brk + delta)
        arg
    };
    let Some(new_brk_signed) = (current_brk as isize).checked_add(delta) else {
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} overflow -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom
        );
        return errno(ENOMEM);
    };
    let new_brk = new_brk_signed as usize;
    if new_brk < heap_bottom {
        // new_brk is below the heap bottom, which is invalid
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
        return errno(ENOMEM);
    }
    if new_brk >= USER_ADDR_MAX {
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} exceeds USER_ADDR_MAX={:#x} -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk,
            USER_ADDR_MAX
        );
        return errno(ENOMEM);
    }
    // delta == 0 is a no-op (brk query or same address)
    let result = process.with_memory_mut(|memory| {
        if delta == 0 {
            true
        } else if delta < 0 {
            // Shrink heap using type-based lookup
            memory.memory_set.shrink_heap_to(VirtAddr(new_brk))
        } else {
            // Expand heap using type-based lookup
            memory
                .memory_set
                .append_heap_to(VirtAddr(new_brk), VirtAddr(heap_bottom))
        }
    });
    if result {
        process.set_brk(new_brk);
        trace!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} ok",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
        if is_abs {
            current_brk as isize + delta
        } else {
            current_brk as isize
        }
    } else {
        error!(
            "[sys_sbrk] pid={} name={} sepc={:#x} arg={} cur={:#x} heap_bottom={:#x} new={:#x} -> ENOMEM",
            pid,
            name,
            sepc,
            arg,
            current_brk,
            heap_bottom,
            new_brk
        );
        errno(ENOMEM)
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(_path: *const u8) -> isize {
    trace!(
        "kernel:pid[{}] sys_spawn NOT IMPLEMENTED",
        current_process().pid.0
    );
    errno(ENOSYS)
}

const PRIO_PROCESS: isize = 0;
const PRIO_PGRP: isize = 1;
const PRIO_USER: isize = 2;
const PTRACE_TRACEME: usize = 0;
const PTRACE_CONT: usize = 7;
const PTRACE_KILL: usize = 8;

fn clamp_nice(prio: isize) -> i32 {
    prio.clamp(-20, 19) as i32
}

fn collect_priority_target_pids(which: isize, who: isize) -> Result<Vec<usize>, isize> {
    let current = current_process();
    let (self_pid, self_pgid, self_euid) =
        { (current.getpid(), current.pgid(), current.effective_uid()) };
    match which {
        PRIO_PROCESS => {
            if who < 0 {
                return Err(errno(ESRCH));
            }
            let target_pid = if who == 0 { self_pid } else { who as usize };
            if pid2process(target_pid).is_some() {
                Ok(vec![target_pid])
            } else {
                Err(errno(ESRCH))
            }
        }
        PRIO_PGRP => {
            if who < 0 {
                return Err(errno(ESRCH));
            }
            let target_pgid = if who == 0 { self_pgid } else { who as usize };
            let mut pids = Vec::new();
            for (pid, process) in pid2process_snapshot() {
                if process.pgid() == target_pgid {
                    pids.push(pid);
                }
            }
            if pids.is_empty() {
                Err(errno(ESRCH))
            } else {
                Ok(pids)
            }
        }
        PRIO_USER => {
            if who < 0 {
                return Err(errno(ESRCH));
            }
            let target_uid = if who == 0 { self_euid } else { who as u32 };
            let mut pids = Vec::new();
            for (pid, process) in pid2process_snapshot() {
                if process.effective_uid() == target_uid {
                    pids.push(pid);
                }
            }
            if pids.is_empty() {
                Err(errno(ESRCH))
            } else {
                Ok(pids)
            }
        }
        _ => Err(errno(EINVAL)),
    }
}

/// setpriority(which, who, prio)
pub fn sys_set_priority(which: isize, who: isize, prio: isize) -> isize {
    let caller = current_process();
    let caller_euid = caller.effective_uid();
    let is_privileged = caller_euid == 0;
    let target_pids = match collect_priority_target_pids(which, who) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let new_nice = clamp_nice(prio);

    for pid in target_pids.iter().copied() {
        let Some(target) = pid2process(pid) else {
            continue;
        };
        let target_identity =
            target.with_identity(|identity| (identity.effective_uid, identity.nice));
        if !is_privileged {
            // Unprivileged caller can only modify its own processes.
            if target_identity.0 != caller_euid {
                return errno(EPERM);
            }
            // Unprivileged caller cannot lower nice value (raise priority).
            if new_nice < target_identity.1 {
                return errno(EACCES);
            }
        }
    }

    let mut applied = false;
    for pid in target_pids {
        if let Some(target) = pid2process(pid) {
            target.with_identity_mut(|identity| identity.nice = new_nice);
            applied = true;
        }
    }
    if applied {
        0
    } else {
        errno(ESRCH)
    }
}

/// getpriority(which, who)
///
/// Linux raw syscall returns `20 - nice` in range [1, 40].
/// libc wrapper converts this back to the public nice value [-20, 19].
pub fn sys_get_priority(which: isize, who: isize) -> isize {
    let target_pids = match collect_priority_target_pids(which, who) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut best_nice: Option<i32> = None;
    for pid in target_pids {
        let Some(target) = pid2process(pid) else {
            continue;
        };
        let nice = target.with_identity(|identity| identity.nice);
        best_nice = Some(best_nice.map(|old| old.min(nice)).unwrap_or(nice));
    }
    let Some(nice) = best_nice else {
        return errno(ESRCH);
    };
    (20 - nice) as isize
}

/// ptrace(request, pid, addr, data)
///
/// Minimal support for LTP ptrace01/ptrace05:
/// - PTRACE_TRACEME
/// - PTRACE_CONT
/// - PTRACE_KILL
pub fn sys_ptrace(request: usize, pid: isize, _addr: usize, _data: usize) -> isize {
    match request {
        PTRACE_TRACEME => {
            let process = current_process();
            if process.ptrace_traceme() {
                return errno(EPERM);
            }
            process.set_ptrace_traceme(true);
            process.set_ptrace_stop_signal(None);
            0
        }
        PTRACE_CONT | PTRACE_KILL => {
            if pid <= 0 {
                return errno(ESRCH);
            }
            let pid = pid as usize;
            let Some(target) = pid2process(pid) else {
                return errno(ESRCH);
            };
            let current_pid = current_process().getpid();
            let traced_by_me = {
                if !target.ptrace_traceme() {
                    false
                } else {
                    match target.parent() {
                        Some(parent) => parent.getpid() == current_pid,
                        None => false,
                    }
                }
            };
            if !traced_by_me {
                return errno(EPERM);
            }
            if request == PTRACE_CONT {
                target.set_ptrace_stop_signal(None);
                let tasks = target.tasks_snapshot();
                for task in tasks {
                    if task.status() == TaskStatus::Blocked {
                        task.set_status(TaskStatus::Ready);
                        add_task(task);
                    }
                }
                0
            } else {
                // Keep behavior consistent with signal path: traced task gets SIGKILL.
                sys_kill(pid as isize, SIGKILL as i32)
            }
        }
        _ => errno(ENOSYS),
    }
}

pub fn sys_kill(pid: isize, signum: i32) -> isize {
    let pid_now = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid_now) {
        syscall!(
            "kernel:pid[{}] sys_kill pid={} signum={}",
            pid_now,
            pid,
            signum
        );
    }
    let flag = if signum == 0 {
        None
    } else {
        Some(match signal_flag_from_signum(signum) {
            Ok(flag) => flag,
            Err(err) => return err,
        })
    };

    if pid > 0 {
        kill_single(pid as usize, signum, flag, pid_now)
    } else if pid == 0 {
        let caller_pgid = current_process().pgid();
        kill_group(Some(caller_pgid), signum, flag, pid_now)
    } else if pid == -1 {
        kill_group(None, signum, flag, pid_now)
    } else {
        let target_pgid = (-pid) as usize;
        kill_group(Some(target_pgid), signum, flag, pid_now)
    }
}

fn kill_single(pid: usize, signum: i32, flag: Option<SignalFlags>, sender_pid: usize) -> isize {
    let process = match pid2process(pid) {
        Some(p) => p,
        None => return errno(ESRCH),
    };
    if process.getpid() == 1 && sender_pid != 1 {
        return errno(EPERM);
    }
    if signum == 0 {
        return 0;
    }
    let flag = flag.unwrap();
    let mut yielded_after_continue = false;
    if flag == SignalFlags::SIGCONT && process.group_stopped() {
        process.set_child_wait_event(Some(ChildWaitEvent::Continued(SIGCONT as i32)));
        process.set_group_stopped(false);
        yielded_after_continue = true;
    }
    process.insert_process_signal(flag, sender_pid as i32, 0);
    for task in process.tasks_snapshot() {
        wake_task_for_signal(&task, flag);
    }
    if yielded_after_continue {
        suspend_current_and_run_next();
    }
    0
}

fn kill_group(
    target_pgid: Option<usize>,
    signum: i32,
    flag: Option<SignalFlags>,
    sender_pid: usize,
) -> isize {
    let mut found = false;
    for (p, process) in pid2process_snapshot() {
        if p == 1 {
            continue;
        }
        if target_pgid.is_none() && p == sender_pid {
            continue;
        }
        let matches = if let Some(pgid) = target_pgid {
            process.pgid() == pgid
        } else {
            true
        };
        if !matches {
            continue;
        }
        found = true;
        if signum == 0 {
            continue;
        }
        let flag = flag.unwrap();
        if flag == SignalFlags::SIGCONT && process.group_stopped() {
            process.set_child_wait_event(Some(ChildWaitEvent::Continued(SIGCONT as i32)));
            process.set_group_stopped(false);
        }
        process.insert_process_signal(flag, sender_pid as i32, 0);
        for task in process.tasks_snapshot() {
            wake_task_for_signal(&task, flag);
        }
    }
    if found {
        0
    } else {
        errno(ESRCH)
    }
}

fn signal_flag_from_signum(signum: i32) -> Result<SignalFlags, isize> {
    if signum <= 0 || signum > MAX_SIG as i32 {
        return Err(errno(EINVAL));
    }
    let flag = match 1u64.checked_shl((signum - 1) as u32) {
        Some(bits) => SignalFlags::from_bits_truncate(bits),
        None => return Err(errno(EINVAL)),
    };
    if flag.is_empty() {
        return Err(errno(EINVAL));
    }
    Ok(flag)
}

fn wake_task_for_signal(task: &Arc<TaskControlBlock>, flag: SignalFlags) {
    let signal_unmasked = !task.signal_mask().contains(flag);
    let force_wake = flag == SignalFlags::SIGKILL || flag == SignalFlags::SIGCONT;
    if task.status() == TaskStatus::Blocked && (signal_unmasked || force_wake) {
        futex_remove_waiter_any(task);
        task.mark_interrupted();
        task.set_status(TaskStatus::Ready);
        add_task(task.clone());
    }
}

fn task_matches_linux_tid(
    process_pid: usize,
    task: &Arc<TaskControlBlock>,
    target_tid: usize,
) -> bool {
    super::thread::match_user_tid(process_pid, task.tid(), target_tid)
}

fn send_signal_to_task_from_list(
    target_tid: usize,
    process_pid: usize,
    tasks: &[Arc<TaskControlBlock>],
    flag: SignalFlags,
) -> isize {
    let target = tasks
        .iter()
        .find(|task| task_matches_linux_tid(process_pid, task, target_tid))
        .cloned();
    let Some(task) = target else {
        return errno(ESRCH);
    };
    task.insert_pending_signal(flag);
    wake_task_for_signal(&task, flag);
    0
}

pub fn sys_tkill(tid: isize, signum: i32) -> isize {
    let pid_now = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid_now) {
        syscall!(
            "kernel:pid[{}] sys_tkill tid={} signum={}",
            pid_now,
            tid,
            signum
        );
    }
    if tid <= 0 {
        return errno(EINVAL);
    }
    let flag = match signal_flag_from_signum(signum) {
        Ok(flag) => flag,
        Err(err) => return err,
    };
    let tid = tid as usize;
    // First search current process (most common case: pthread_cancel sends
    // tkill to a thread in the same process). Only fall back to pid2process
    // for cross-process tkill.
    let process = current_process();
    let process_pid = process.getpid();
    let tasks = process.tasks_snapshot();
    let ret = send_signal_to_task_from_list(tid, process_pid, &tasks, flag);
    if ret == 0 {
        return 0;
    }
    // Fallback: try as a global PID (for cross-process tkill)
    if let Some(process) = pid2process(tid) {
        let process_pid = process.getpid();
        if process_pid == 1 && pid_now != 1 {
            return errno(EPERM);
        }
        let tasks = process.tasks_snapshot();
        let ret = send_signal_to_task_from_list(tid, process_pid, &tasks, flag);
        return ret;
    }
    ret
}

pub fn sys_tgkill(tgid: isize, tid: isize, signum: i32) -> isize {
    let pid_now = current_process().pid.0 as isize;
    if crate::syscall::should_trace_syscall(pid_now as usize) {
        syscall!(
            "kernel:pid[{}] sys_tgkill tgid={} tid={} signum={}",
            pid_now,
            tgid,
            tid,
            signum
        );
    }
    if tgid <= 0 || tid <= 0 {
        return errno(EINVAL);
    }
    if tgid != pid_now {
        return errno(ESRCH);
    }
    sys_tkill(tid, signum)
}

pub fn sys_sigaction(
    signum: i32,
    action: *const SignalAction,
    old_action: *mut SignalAction,
    sigsetsize: usize,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_sigaction signum={} size={}",
            pid,
            signum,
            sigsetsize
        );
    }
    if sigsetsize != core::mem::size_of::<usize>() {
        return errno(EINVAL);
    }
    if signum <= 0
        || signum > MAX_SIG as i32
        || signum == SIGKILL as i32
        || signum == SIGSTOP as i32
    {
        return errno(EINVAL);
    }
    let process = current_process();
    let idx = signum as usize;
    let token = current_user_token();
    if !old_action.is_null() {
        let old = process.with_signal_action(idx, |action| action);
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old as *const SignalAction) as *const u8,
                core::mem::size_of::<SignalAction>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_action as *mut u8, bytes) {
            return err;
        }
    }
    if !action.is_null() {
        let new_action = match read_from_user::<SignalAction>(token, action) {
            Ok(v) => v,
            Err(err) => return err,
        };
        #[cfg(not(target_arch = "loongarch64"))]
        let mut new_action = new_action;

        #[cfg(not(target_arch = "loongarch64"))]
        {
            // Linux only uses sa_restorer when SA_RESTORER is set.
            // glibc may leave a non-zero value in sa_restorer even when SA_RESTORER
            // is clear; that value is semantically irrelevant and must not be used.
            if (new_action.flags & SA_RESTORER) == 0 {
                if new_action.restorer != 0 {
                    debug!(
                        "[sigaction] pid={} signum={} clear restorer={:#x} because SA_RESTORER is not set",
                        pid, signum, new_action.restorer
                    );
                    new_action.restorer = 0;
                }
            } else if new_action.restorer == 0 {
                // Keep stored action self-consistent to avoid confusing downstream logic.
                new_action.flags &= !SA_RESTORER;
            }
        }

        if signum == 32 || signum == 33 || signum == crate::task::SIGCHLD as i32 {
            info!(
                "[sigaction] pid={} signum={} handler={:#x} flags={:#x} restorer={:#x} mask={:?}",
                pid,
                signum,
                new_action.handler,
                new_action.flags,
                new_action.restorer(),
                new_action.mask
            );
            if new_action.handler >= USER_ADDR_MAX && new_action.handler > 1 {
                warn!(
                    "[sigaction] pid={} signum={} handler out of range: {:#x}",
                    pid, signum, new_action.handler
                );
            }
        }
        process.update_signal_action(idx, |slot| *slot = new_action);
    }
    0
}

pub fn sys_sigprocmask(
    how: usize,
    set: *const usize,
    oldset: *mut usize,
    sigsetsize: usize,
) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        trace!(
            "kernel:pid[{}] sys_sigprocmask how=0x{:x} set={:?} oldset={:?} size={}",
            pid,
            how,
            set,
            oldset,
            sigsetsize
        );
    }
    if sigsetsize != core::mem::size_of::<usize>() {
        return errno(EINVAL);
    }
    let token = current_user_token();
    let task = current_task().unwrap();

    if !oldset.is_null() {
        // 使用 flags_to_user_mask 简化转换
        let user_mask = flags_to_user_mask(task.signal_mask()) as usize;
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&user_mask as *const usize) as *const u8,
                core::mem::size_of::<usize>(),
            )
        };
        if let Err(err) = copy_to_user(token, oldset as *mut u8, bytes) {
            return err;
        }
    }

    if !set.is_null() {
        if !matches!(how, 0 | 1 | 2) {
            return errno(EINVAL);
        }
        let user_mask = match read_from_user::<usize>(token, set) {
            Ok(v) => v,
            Err(err) => return err,
        };
        let mut new_flags = user_mask_to_flags(user_mask as u64);
        new_flags.remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        task.with_signals_mut(|signals| {
            match how {
                0 => signals.signal_mask |= new_flags,  // SIG_BLOCK
                1 => signals.signal_mask &= !new_flags, // SIG_UNBLOCK
                2 => signals.signal_mask = new_flags,   // SIG_SETMASK
                _ => {}
            }
            signals
                .signal_mask
                .remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        });
    }
    0
}

pub fn sys_rt_sigpending(set: *mut usize, sigsetsize: usize) -> isize {
    if sigsetsize != core::mem::size_of::<usize>() {
        return errno(EINVAL);
    }
    let token = current_user_token();
    let task = current_task().unwrap();
    // Get pending signals as a user mask
    let pending = task.with_signals(|signals| flags_to_user_mask(signals.signal_pending));
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&(pending as usize) as *const usize) as *const u8,
            core::mem::size_of::<usize>(),
        )
    };
    if let Err(err) = copy_to_user(token, set as *mut u8, bytes) {
        return err;
    }
    0
}

pub fn sys_rt_sigqueueinfo(pid: isize, sig: i32, _info: *const u8) -> isize {
    // Simplified: just deliver the signal like kill
    sys_kill(pid, sig)
}

pub fn sys_sigreturn() -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_sigreturn", pid);
    }
    let task = current_task().unwrap();
    let trap_sepc = task.with_trap_cx_mut(|trap_cx| trap_cx.sepc);
    let in_sigreturn_trampoline = arch::is_sigreturn_trampoline_pc(trap_sepc);

    let saved = match task.take_signal_frame() {
        Some(cx) => cx,
        None => {
            if in_sigreturn_trampoline {
                error!(
                    "[sigreturn] missing signal frame in trampoline context, pid={} sepc={:#x}, force SIGSEGV",
                    pid, trap_sepc
                );
                task.with_signals_mut(|signals| {
                    signals.handling_sig = -1;
                    signals.signal_ucontext_ptr = 0;
                    signals.signal_canary_ptr = 0;
                });
                exit_current_and_run_next(-(SIGSEGV as i32));
                panic!("Unreachable after fatal sigreturn frame error");
            }
            return errno(EINVAL);
        }
    };

    // 检查信号帧 canary（使用投递时记录的精确地址，避免依赖当前 SP）
    let current_sp = task.with_trap_cx_mut(|trap_cx| trap_cx[TrapFrameArgs::SP]);
    let canary_ptr = task.with_signals_mut(|signals| {
        let canary_ptr = signals.signal_canary_ptr;
        signals.signal_canary_ptr = 0;
        canary_ptr
    });
    let token = current_user_token();
    if canary_ptr != 0 {
        match read_from_user::<usize>(token, canary_ptr as *const _) {
            Ok(canary) if canary == 0x11451415 => {}
            Ok(canary) => {
                // Some user handlers (or libc internals) may legitimately overwrite the
                // scratch word we use as a canary. Treat this as a diagnostic only.
                warn!(
                    "[sigreturn] stack canary mismatch (non-fatal) pid={} sp={:#x} canary_ptr={:#x} canary={:#x} expected=0x11451415",
                    pid, current_sp, canary_ptr, canary
                );
            }
            Err(err) => {
                warn!(
                    "[sigreturn] cannot read canary (non-fatal) pid={} sp={:#x} canary_ptr={:#x} err={}",
                    pid, current_sp, canary_ptr, err
                );
            }
        }
    }
    let saved_a0 = saved[TrapFrameArgs::RET] as isize;
    let ucontext_ptr = task.with_signals_mut(|signals| {
        let ucontext_ptr = signals.signal_ucontext_ptr;
        signals.signal_ucontext_ptr = 0;
        ucontext_ptr
    });

    // 恢复 trap context 和 signal_mask
    let return_pc;
    let saved_pc = saved.sepc; // handler 修改前的原始 PC
    let ret_a0;
    if ucontext_ptr != 0 {
        let token = current_user_token();
        let ucontext = match read_from_user::<UserContext>(token, ucontext_ptr as *const _) {
            Ok(value) => value,
            Err(_) => {
                error!(
                    "[sigreturn] cannot read ucontext! pid={} ucontext_ptr={:#x}, force SIGSEGV",
                    pid, ucontext_ptr
                );
                task.set_handling_sig(-1);
                exit_current_and_run_next(-(SIGSEGV as i32));
                panic!("Unreachable after fatal sigreturn ucontext read failure");
            }
        };
        info!(
            "[sigreturn] pid={} ucontext_ptr={:#x} saved_pc={:#x} ucontext_pc={:#x} sigmask={:#x}",
            pid,
            ucontext_ptr,
            saved.sepc,
            ucontext.user_pc(),
            ucontext.signal_mask_word0()
        );
        let mut restored = saved;
        ucontext.restore_trap_context(&mut restored);
        task.with_trap_cx_mut(|trap_cx| *trap_cx = restored);
        // 从 ucontext 恢复信号掩码（per-thread）
        let mut new_mask = user_mask_to_flags(ucontext.signal_mask_word0());
        new_mask.remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        task.set_signal_mask(new_mask);
        return_pc = restored.sepc;
        ret_a0 = restored[TrapFrameArgs::RET] as isize;
    } else {
        task.with_trap_cx_mut(|trap_cx| *trap_cx = saved);
        // 从 backup 恢复信号掩码（per-thread）
        let signal_mask_backup = task.with_signals(|signals| signals.signal_mask_backup);
        task.set_signal_mask(signal_mask_backup);
        return_pc = saved.sepc;
        ret_a0 = saved_a0;
    }

    #[cfg(target_arch = "loongarch64")]
    {
        let (sp, ra) = task
            .with_trap_cx_mut(|trap_cx| (trap_cx[TrapFrameArgs::SP], trap_cx[TrapFrameArgs::RA]));
        trace!(
            "[sigreturn] pid={} sig={} ucontext_ptr={:#x} saved_pc={:#x} return_pc={:#x} sp={:#x} ra={:#x}",
            pid,
            task.handling_sig(),
            ucontext_ptr,
            saved_pc,
            return_pc,
            sp,
            ra
        );
    }

    // SA_RESETHAND 处理
    let current_sig = task.handling_sig();
    task.set_handling_sig(-1);
    let mut process_pending_snapshot = SignalFlags::empty();

    if current_sig >= 0 && (current_sig as usize) <= crate::task::MAX_SIG {
        let process = current_process();
        process_pending_snapshot = process.pending_signal();
        let action = process.with_signal_action(current_sig as usize, |action| action);
        if (action.flags & crate::task::SA_RESETHAND) != 0 {
            info!(
                "[sigreturn] SA_RESETHAND set for signal {}, resetting handler to SIG_DFL",
                current_sig
            );
            process
                .update_signal_action(current_sig as usize, |slot| *slot = SignalAction::default());
        }
    }

    // SIGCANCEL (signal 33) 处理说明：
    // musl 的 cancel_handler 总是把 SIG33 加入 uc_sigmask 防止重复投递。
    // 取消通过两种机制工作：
    //   1. 异步取消(cancelasync=1)：handler 直接调用 pthread_exit，不会到达 sigreturn
    //   2. 延迟取消：handler 修改 ucontext PC 指向 __cancel，或者不做任何事
    //      后续 musl 的 __syscall_cp_asm 会在每个系统调用入口检查 cancel 标志
    //
    // 不要重新注入 SIG33！musl 的 cancel flag 机制会在下次系统调用时自动生效。
    // 如果 handler 修改了 PC（重定向到 __cancel），确保 SIG33 不阻塞后续行为。
    if current_sig == 33 && return_pc != saved_pc {
        // handler 成功修改了 PC，确保 SIG33 不被掩码
        task.update_signal_mask(|mask| mask.remove(SignalFlags::SIG33));
    }

    if current_sig == 32 || current_sig == 33 {
        let tid = task.tid();
        let tp = task.with_trap_cx_mut(|trap_cx| trap_cx[TrapFrameArgs::TLS]);
        let (task_pending, signal_mask) =
            task.with_signals(|signals| (signals.signal_pending, signals.signal_mask));
        info!(
            "[signal-flow] route=sigreturn pid={} tid={} sig{} tp={:#x} saved_pc={:#x} return_pc={:#x} pc_changed={} handling_reset=-1 mask_after={:?} task_pending={:?} proc_pending={:?}",
            pid,
            tid,
            current_sig,
            tp,
            saved_pc,
            return_pc,
            return_pc != saved_pc,
            signal_mask,
            task_pending,
            process_pending_snapshot
        );
    }

    ret_a0
}

/// Get user ID
pub fn sys_getuid() -> isize {
    let process = current_process();
    syscall!("kernel:pid[{}] sys_getuid", process.pid.0);
    let uid = process.real_uid() as isize;
    uid
}

/// Get effective user ID
pub fn sys_geteuid() -> isize {
    let process = current_process();
    syscall!("kernel:pid[{}] sys_geteuid", process.pid.0);
    let uid = process.effective_uid() as isize;
    uid
}

/// Get group ID
pub fn sys_getgid() -> isize {
    let process = current_process();
    syscall!("kernel:pid[{}] sys_getgid", process.pid.0);
    let gid = process.real_gid() as isize;
    gid
}

/// Get effective group ID
pub fn sys_getegid() -> isize {
    let process = current_process();
    syscall!("kernel:pid[{}] sys_getegid", process.pid.0);
    let gid = process.effective_gid() as isize;
    gid
}

/// Set filesystem user ID.
/// Linux semantics: always returns previous fsuid, never fails.
pub fn sys_setfsuid(uid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let prev = inner.fs_uid;
        let neg1 = u32::MAX;

        if uid == neg1 {
            return prev as isize;
        }

        let is_privileged = inner.effective_uid == 0;
        if is_privileged
            || uid == inner.real_uid
            || uid == inner.effective_uid
            || uid == inner.saved_uid
            || uid == inner.fs_uid
        {
            inner.fs_uid = uid;
        }
        prev as isize
    })
}

/// Set filesystem group ID.
/// Linux semantics: always returns previous fsgid, never fails.
pub fn sys_setfsgid(gid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let prev = inner.fs_gid;
        let neg1 = u32::MAX;

        if gid == neg1 {
            return prev as isize;
        }

        let is_privileged = inner.effective_uid == 0;
        if is_privileged
            || gid == inner.real_gid
            || gid == inner.effective_gid
            || gid == inner.saved_gid
            || gid == inner.fs_gid
        {
            inner.fs_gid = gid;
        }
        prev as isize
    })
}

/// reboot(magic, magic2, cmd) - only the non-rebooting CAD toggles are accepted.
pub fn sys_reboot(magic: usize, magic2: usize, cmd: usize) -> isize {
    const LINUX_REBOOT_MAGIC1: u32 = 0xfee1_dead;
    const LINUX_REBOOT_MAGIC2: u32 = 672_274_793;
    const LINUX_REBOOT_MAGIC2A: u32 = 850_722_78;
    const LINUX_REBOOT_MAGIC2B: u32 = 369_367_448;
    const LINUX_REBOOT_MAGIC2C: u32 = 537_993_216;
    const LINUX_REBOOT_CMD_CAD_ON: u32 = 0x89ab_cdef;
    const LINUX_REBOOT_CMD_CAD_OFF: u32 = 0x0000_0000;

    let magic = magic as u32;
    let magic2 = magic2 as u32;
    let cmd = cmd as u32;
    let cmd = if magic == LINUX_REBOOT_MAGIC1 {
        if !matches!(
            magic2,
            LINUX_REBOOT_MAGIC2
                | LINUX_REBOOT_MAGIC2A
                | LINUX_REBOOT_MAGIC2B
                | LINUX_REBOOT_MAGIC2C
        ) {
            return errno(EINVAL);
        }
        cmd
    } else {
        magic
    };

    if !matches!(cmd, LINUX_REBOOT_CMD_CAD_ON | LINUX_REBOOT_CMD_CAD_OFF) {
        return errno(EINVAL);
    }
    if current_process().effective_uid() != 0 {
        return errno(EPERM);
    }
    0
}

/// Set user ID with minimal Linux-like semantics needed by LTP.
pub fn sys_setuid(uid: u32) -> isize {
    let process = current_process();
    let pid = process.pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_setuid uid={}", pid, uid);
    }
    process.with_identity_mut(|inner| {
        if inner.effective_uid == 0 {
            inner.real_uid = uid;
            inner.effective_uid = uid;
            inner.fs_uid = uid;
            return 0;
        }
        if uid == inner.real_uid || uid == inner.effective_uid {
            inner.effective_uid = uid;
            inner.fs_uid = uid;
            return 0;
        }
        errno(EPERM)
    })
}

/// Set group ID with minimal Linux-like semantics needed by LTP.
pub fn sys_setgid(gid: u32) -> isize {
    let process = current_process();
    let pid = process.pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_setgid gid={}", pid, gid);
    }
    process.with_identity_mut(|inner| {
        if inner.effective_uid == 0 {
            inner.real_gid = gid;
            inner.effective_gid = gid;
            inner.saved_gid = gid;
            inner.fs_gid = gid;
            return 0;
        }
        if gid == inner.real_gid || gid == inner.effective_gid {
            inner.effective_gid = gid;
            inner.fs_gid = gid;
            return 0;
        }
        errno(EPERM)
    })
}

/// getgroups(size, list) - syscall 158
/// Returns the supplementary group IDs of the calling process.
pub fn sys_getgroups(size: i32, list: *mut u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_getgroups size={} list={:p}",
            pid,
            size,
            list
        );
    }
    if size < 0 {
        return errno(EINVAL);
    }

    let groups = current_process().with_identity(|identity| identity.supplementary_groups.clone());
    let group_count = groups.len();
    if size == 0 {
        return group_count as isize;
    }
    if list.is_null() {
        return errno(EFAULT);
    }
    if (size as usize) < group_count {
        return errno(EINVAL);
    }

    let mut bytes = Vec::with_capacity(group_count * core::mem::size_of::<u32>());
    for gid in groups {
        bytes.extend_from_slice(&gid.to_ne_bytes());
    }
    let token = current_user_token();
    if copy_to_user(token, list as *mut u8, &bytes).is_err() {
        return errno(EFAULT);
    }
    group_count as isize
}

/// setgroups(size, list) - syscall 159
/// Set supplementary group IDs.
pub fn sys_setgroups(size: usize, list: *const u32) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_setgroups size={} list={:p}",
            pid,
            size,
            list
        );
    }
    const NGROUPS_MAX: usize = 65536;
    if size > NGROUPS_MAX {
        return errno(EINVAL);
    }

    let process = current_process();
    if process.effective_uid() != 0 {
        return errno(EPERM);
    }

    if size > 0 && list.is_null() {
        return errno(EFAULT);
    }

    let mut groups = Vec::with_capacity(size);
    if size > 0 {
        let byte_len = size * core::mem::size_of::<u32>();
        let mut bytes = vec![0u8; byte_len];
        if user_mem::copy_from_user(
            current_user_token(),
            list as *const u8,
            &mut bytes,
            UserReadPolicy::DemandPaged,
        )
        .is_err()
        {
            return errno(EFAULT);
        }
        for chunk in bytes.chunks_exact(core::mem::size_of::<u32>()) {
            groups.push(u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
    }
    process.with_identity_mut(|identity| {
        identity.supplementary_groups = groups;
    });
    0
}

/// setregid(rgid, egid) - syscall 143
/// Set real and/or effective group ID.
pub fn sys_setregid(rgid: u32, egid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let is_root = inner.effective_uid == 0;
        let neg1 = u32::MAX;

        if !is_root {
            // unprivileged: can only set to one of real/effective/saved
            if rgid != neg1 && rgid != inner.real_gid && rgid != inner.effective_gid {
                return errno(EPERM);
            }
            if egid != neg1
                && egid != inner.real_gid
                && egid != inner.effective_gid
                && egid != inner.saved_gid
            {
                return errno(EPERM);
            }
        }

        let old_real_gid = inner.real_gid;
        if rgid != neg1 {
            inner.real_gid = rgid;
        }
        let new_egid = if egid != neg1 {
            egid
        } else {
            inner.effective_gid
        };
        if egid != neg1 {
            inner.effective_gid = new_egid;
            inner.fs_gid = new_egid;
        }
        // Linux setregid: saved_gid is set to new effective GID if:
        //   1. real GID was changed, OR
        //   2. new effective GID != old real GID
        if rgid != neg1 || new_egid != old_real_gid {
            inner.saved_gid = new_egid;
        }
        0
    })
}

/// setreuid(ruid, euid) - syscall 145
pub fn sys_setreuid(ruid: u32, euid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let is_root = inner.effective_uid == 0;
        let neg1 = u32::MAX;

        if !is_root {
            if ruid != neg1 && ruid != inner.real_uid && ruid != inner.effective_uid {
                return errno(EPERM);
            }
            if euid != neg1
                && euid != inner.real_uid
                && euid != inner.effective_uid
                && euid != inner.saved_uid
            {
                return errno(EPERM);
            }
        }

        let old_real_uid = inner.real_uid;
        if ruid != neg1 {
            inner.real_uid = ruid;
        }
        let new_euid = if euid != neg1 {
            euid
        } else {
            inner.effective_uid
        };
        if euid != neg1 {
            inner.effective_uid = new_euid;
            inner.fs_uid = new_euid;
        }
        // Linux setreuid: saved_uid is set to new effective UID if:
        //   1. real UID was changed, OR
        //   2. new effective UID != old real UID
        if ruid != neg1 || new_euid != old_real_uid {
            inner.saved_uid = new_euid;
        }
        0
    })
}

/// setresuid(ruid, euid, suid) - syscall 147
pub fn sys_setresuid(ruid: u32, euid: u32, suid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let is_root = inner.effective_uid == 0;
        let neg1 = u32::MAX;

        if !is_root {
            // Non-root: can only set to one of current real/effective/saved
            let allowed = [inner.real_uid, inner.effective_uid, inner.saved_uid];
            if ruid != neg1 && !allowed.contains(&ruid) {
                return errno(EPERM);
            }
            if euid != neg1 && !allowed.contains(&euid) {
                return errno(EPERM);
            }
            if suid != neg1 && !allowed.contains(&suid) {
                return errno(EPERM);
            }
        }

        if ruid != neg1 {
            inner.real_uid = ruid;
        }
        if euid != neg1 {
            inner.effective_uid = euid;
            inner.fs_uid = euid;
        }
        if suid != neg1 {
            inner.saved_uid = suid;
        }
        0
    })
}

/// getresuid(ruid, euid, suid) - syscall 148
pub fn sys_getresuid(ruid: *mut u32, euid: *mut u32, suid: *mut u32) -> isize {
    let process = current_process();
    let (r, e, s) =
        process.with_identity(|inner| (inner.real_uid, inner.effective_uid, inner.saved_uid));
    drop(process);
    let token = current_user_token();
    let r_bytes = r.to_ne_bytes();
    let e_bytes = e.to_ne_bytes();
    let s_bytes = s.to_ne_bytes();
    if !ruid.is_null() {
        if copy_to_user(token, ruid as *mut u8, &r_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    if !euid.is_null() {
        if copy_to_user(token, euid as *mut u8, &e_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    if !suid.is_null() {
        if copy_to_user(token, suid as *mut u8, &s_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    0
}

/// setresgid(rgid, egid, sgid) - syscall 149
pub fn sys_setresgid(rgid: u32, egid: u32, sgid: u32) -> isize {
    let process = current_process();
    process.with_identity_mut(|inner| {
        let is_root = inner.effective_uid == 0;
        let neg1 = u32::MAX;

        if !is_root {
            let allowed = [inner.real_gid, inner.effective_gid, inner.saved_gid];
            if rgid != neg1 && !allowed.contains(&rgid) {
                return errno(EPERM);
            }
            if egid != neg1 && !allowed.contains(&egid) {
                return errno(EPERM);
            }
            if sgid != neg1 && !allowed.contains(&sgid) {
                return errno(EPERM);
            }
        }

        if rgid != neg1 {
            inner.real_gid = rgid;
        }
        if egid != neg1 {
            inner.effective_gid = egid;
            inner.fs_gid = egid;
        }
        if sgid != neg1 {
            inner.saved_gid = sgid;
        }
        0
    })
}

/// getresgid(rgid, egid, sgid) - syscall 150
pub fn sys_getresgid(rgid: *mut u32, egid: *mut u32, sgid: *mut u32) -> isize {
    let process = current_process();
    let (r, e, s) =
        process.with_identity(|inner| (inner.real_gid, inner.effective_gid, inner.saved_gid));
    drop(process);
    let token = current_user_token();
    let r_bytes = r.to_ne_bytes();
    let e_bytes = e.to_ne_bytes();
    let s_bytes = s.to_ne_bytes();
    if !rgid.is_null() {
        if copy_to_user(token, rgid as *mut u8, &r_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    if !egid.is_null() {
        if copy_to_user(token, egid as *mut u8, &e_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    if !sgid.is_null() {
        if copy_to_user(token, sgid as *mut u8, &s_bytes).is_err() {
            return errno(EFAULT);
        }
    }
    0
}

/// rt_sigsuspend(mask, sigsetsize) - syscall 133
/// Replace the signal mask and suspend until a signal is delivered.
pub fn sys_rt_sigsuspend(mask_ptr: *const usize, sigsetsize: usize) -> isize {
    if mask_ptr.is_null() {
        return errno(EFAULT);
    }
    if sigsetsize != core::mem::size_of::<usize>() {
        return errno(EINVAL);
    }
    let token = current_user_token();
    let user_mask = match read_from_user::<usize>(token, mask_ptr) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };
    let new_mask = user_mask_to_flags(user_mask as u64);

    // Save old mask, install new mask
    let task = current_task().unwrap();
    let old_mask = {
        let old = task.signal_mask();
        let mut m = new_mask;
        m.remove(SignalFlags::SIGKILL | SignalFlags::SIGSTOP);
        task.set_signal_mask(m);
        old
    };

    // Wait for any unmasked signal
    loop {
        let has_signal = {
            let process = current_process();
            let (task_pending, mask) =
                task.with_signals(|signals| (signals.signal_pending, signals.signal_mask));
            let pending = process.pending_signal() | task_pending;
            !(pending & !mask).is_empty()
        };
        if has_signal {
            break;
        }
        suspend_current_and_run_next();
    }

    // Restore old mask
    task.set_signal_mask(old_mask);
    errno(EINTR)
}

/// adjtimex(buf) - syscall 159
/// Tune kernel clock. Stub: return TIME_OK (0) for valid modes, EINVAL for invalid.
#[repr(C)]
#[allow(dead_code)]
struct Timex {
    modes: u32,
    offset: i64,
    freq: i64,
    maxerror: i64,
    esterror: i64,
    status: i32,
    constant: i64,
    precision: i64,
    tolerance: i64,
    time_sec: i64,
    time_usec: i64,
    tick: i64,
    // more fields...
    _pad: [u8; 128],
}

#[derive(Clone, Copy)]
struct TimexState {
    offset: i64,
    freq: i64,
    maxerror: i64,
    esterror: i64,
    status: i32,
    constant: i64,
    tick: i64,
}

impl Default for TimexState {
    fn default() -> Self {
        Self {
            offset: 0,
            freq: 0,
            maxerror: 0,
            esterror: 0,
            status: 0,
            constant: 0,
            tick: 10000,
        }
    }
}

pub fn sys_adjtimex(buf: *mut u8) -> isize {
    if buf.is_null() {
        return errno(EFAULT);
    }
    // Read the modes field (first u32 in struct timex)
    let token = current_user_token();
    let modes = match read_from_user::<u32>(token, buf as *const u32) {
        Ok(m) => m,
        Err(_) => return errno(EFAULT),
    };

    const ADJ_OFFSET: u32 = 0x0001;
    const ADJ_FREQUENCY: u32 = 0x0002;
    const ADJ_MAXERROR: u32 = 0x0004;
    const ADJ_ESTERROR: u32 = 0x0008;
    const ADJ_STATUS: u32 = 0x0010;
    const ADJ_TIMECONST: u32 = 0x0020;
    const ADJ_TICK: u32 = 0x4000;
    const ADJ_ADJTIME: u32 = 0x8000;

    // Any "set" operation requires CAP_SYS_TIME (root). modes == 0 is read-only.
    if modes != 0 {
        let process = current_process();
        if process.effective_uid() != 0 {
            return errno(EPERM);
        }
    }

    // ADJ_ADJTIME (0x8000) without ADJ_OFFSET (0x0001) is invalid
    if (modes & ADJ_ADJTIME) != 0 && (modes & ADJ_OFFSET) == 0 {
        return errno(EINVAL);
    }

    // Field offsets in the 64-bit kernel timex ABI used by LTP.
    const OFFSET_OFFSET: usize = 8;
    const FREQ_OFFSET: usize = 16;
    const MAXERROR_OFFSET: usize = 24;
    const ESTERROR_OFFSET: usize = 32;
    const STATUS_OFFSET: usize = 40;
    const CONSTANT_OFFSET: usize = 48;
    const TICK_OFFSET: usize = 88;

    let read_i64 = |offset: usize| -> Result<i64, isize> {
        let ptr = unsafe { (buf as *const u8).add(offset) } as *const i64;
        read_from_user::<i64>(token, ptr)
    };
    let read_i32 = |offset: usize| -> Result<i32, isize> {
        let ptr = unsafe { (buf as *const u8).add(offset) } as *const i32;
        read_from_user::<i32>(token, ptr)
    };

    let mut state = *TIMEX_STATE.exclusive_access();
    if (modes & ADJ_OFFSET) != 0 {
        state.offset = match read_i64(OFFSET_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_FREQUENCY) != 0 {
        state.freq = match read_i64(FREQ_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_MAXERROR) != 0 {
        state.maxerror = match read_i64(MAXERROR_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_ESTERROR) != 0 {
        state.esterror = match read_i64(ESTERROR_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_STATUS) != 0 {
        state.status = match read_i32(STATUS_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_TIMECONST) != 0 {
        state.constant = match read_i64(CONSTANT_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
    }
    if (modes & ADJ_TICK) != 0 {
        let tick = match read_i64(TICK_OFFSET) {
            Ok(v) => v,
            Err(err) => return err,
        };
        // Only validate explicitly non-default tick values
        if tick != 0 && (tick < 9000 || tick > 11000) {
            return errno(EINVAL);
        }
        if tick != 0 {
            state.tick = tick;
        }
    }

    *TIMEX_STATE.exclusive_access() = state;

    let write_i64 = |offset: usize, value: i64| -> Result<(), isize> {
        let ptr = unsafe { (buf as *mut u8).add(offset) };
        copy_to_user(token, ptr, &value.to_ne_bytes())
    };
    let write_i32 = |offset: usize, value: i32| -> Result<(), isize> {
        let ptr = unsafe { (buf as *mut u8).add(offset) };
        copy_to_user(token, ptr, &value.to_ne_bytes())
    };

    if let Err(err) = write_i64(OFFSET_OFFSET, state.offset) {
        return err;
    }
    if let Err(err) = write_i64(FREQ_OFFSET, state.freq) {
        return err;
    }
    if let Err(err) = write_i64(MAXERROR_OFFSET, state.maxerror) {
        return err;
    }
    if let Err(err) = write_i64(ESTERROR_OFFSET, state.esterror) {
        return err;
    }
    if let Err(err) = write_i32(STATUS_OFFSET, state.status) {
        return err;
    }
    if let Err(err) = write_i64(CONSTANT_OFFSET, state.constant) {
        return err;
    }
    if let Err(err) = write_i64(TICK_OFFSET, state.tick) {
        return err;
    }

    // Return TIME_OK (0)
    0
}

/// clock_adjtime(clockid, buf) - share adjtimex semantics for supported clocks.
pub fn sys_clock_adjtime(clockid: usize, buf: *mut u8) -> isize {
    const CLOCK_REALTIME: usize = 0;
    const CLOCK_MONOTONIC: usize = 1;
    if clockid != CLOCK_REALTIME && clockid != CLOCK_MONOTONIC {
        return errno(EINVAL);
    }
    sys_adjtimex(buf)
}

/// Capability header and data structs (Linux ABI)
#[repr(C)]
#[derive(Clone, Copy)]
struct CapHeader {
    version: u32,
    pid: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

const _LINUX_CAPABILITY_VERSION_1: u32 = 0x19980330;
const _LINUX_CAPABILITY_VERSION_2: u32 = 0x20071026;
const _LINUX_CAPABILITY_VERSION_3: u32 = 0x20080522;

/// capget(header, data) - syscall 90
pub fn sys_capget(header_ptr: *mut u8, data_ptr: *mut u8) -> isize {
    if header_ptr.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();

    // Read header
    let mut hdr = match read_from_user::<CapHeader>(token, header_ptr as *const CapHeader) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };

    // Validate version; if invalid, set preferred and return EINVAL
    let valid_versions = [
        _LINUX_CAPABILITY_VERSION_1,
        _LINUX_CAPABILITY_VERSION_2,
        _LINUX_CAPABILITY_VERSION_3,
    ];
    if !valid_versions.contains(&hdr.version) {
        hdr.version = _LINUX_CAPABILITY_VERSION_3;
        let hdr_bytes = unsafe {
            core::slice::from_raw_parts(
                &hdr as *const CapHeader as *const u8,
                core::mem::size_of::<CapHeader>(),
            )
        };
        let _ = copy_to_user(token, header_ptr, hdr_bytes);
        return errno(EINVAL);
    }

    // Validate pid
    if hdr.pid < 0 {
        return errno(EINVAL);
    }

    // Check if target process exists (if pid != 0 and != current)
    let cur_pid = current_process().pid.0 as i32;
    if hdr.pid != 0 && hdr.pid != cur_pid {
        // Check if process exists
        if pid2process(hdr.pid as usize).is_none() {
            return errno(ESRCH);
        }
    }

    // If data_ptr is provided, fill capability data
    if !data_ptr.is_null() {
        // Determine data count (V2/V3 have 2 sets)
        let data_count: usize = if hdr.version == _LINUX_CAPABILITY_VERSION_1 {
            1
        } else {
            2
        };
        let data_bytes_len = data_count * core::mem::size_of::<CapData>();
        if !user_mem::ensure_user_writable(
            token,
            data_ptr,
            data_bytes_len,
            UserWritePolicy::DemandCowWithForkFallback,
        ) {
            return errno(EFAULT);
        }

        // Get caps for the target process
        let process = current_process();
        let caps = process.credentials_snapshot();
        let (eff_lo, eff_hi) = (
            (caps.cap_effective & 0xFFFFFFFF) as u32,
            ((caps.cap_effective >> 32) & 0xFFFFFFFF) as u32,
        );
        let (perm_lo, perm_hi) = (
            (caps.cap_permitted & 0xFFFFFFFF) as u32,
            ((caps.cap_permitted >> 32) & 0xFFFFFFFF) as u32,
        );
        let (inh_lo, inh_hi) = (
            (caps.cap_inheritable & 0xFFFFFFFF) as u32,
            ((caps.cap_inheritable >> 32) & 0xFFFFFFFF) as u32,
        );
        drop(process);

        let data0 = CapData {
            effective: eff_lo,
            permitted: perm_lo,
            inheritable: inh_lo,
        };
        let data_bytes0 = unsafe {
            core::slice::from_raw_parts(
                &data0 as *const CapData as *const u8,
                core::mem::size_of::<CapData>(),
            )
        };
        if copy_to_user(token, data_ptr, data_bytes0).is_err() {
            return errno(EFAULT);
        }

        if data_count == 2 {
            let data1 = CapData {
                effective: eff_hi,
                permitted: perm_hi,
                inheritable: inh_hi,
            };
            let data_bytes1 = unsafe {
                core::slice::from_raw_parts(
                    &data1 as *const CapData as *const u8,
                    core::mem::size_of::<CapData>(),
                )
            };
            let data_ptr1 = unsafe { data_ptr.add(core::mem::size_of::<CapData>()) };
            if copy_to_user(token, data_ptr1, data_bytes1).is_err() {
                return errno(EFAULT);
            }
        }
    }

    0
}

/// capset(header, data) - syscall 91
pub fn sys_capset(header_ptr: *const u8, data_ptr: *const u8) -> isize {
    if header_ptr.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();

    let mut hdr = match read_from_user::<CapHeader>(token, header_ptr as *const CapHeader) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };

    let valid_versions = [
        _LINUX_CAPABILITY_VERSION_1,
        _LINUX_CAPABILITY_VERSION_2,
        _LINUX_CAPABILITY_VERSION_3,
    ];
    if !valid_versions.contains(&hdr.version) {
        // Write preferred version back to header before returning EINVAL
        hdr.version = _LINUX_CAPABILITY_VERSION_3;
        let _ = copy_to_user(token, header_ptr as *mut u8, unsafe {
            core::slice::from_raw_parts(
                &hdr as *const CapHeader as *const u8,
                core::mem::size_of::<CapHeader>(),
            )
        });
        return errno(EINVAL);
    }

    if hdr.pid < 0 {
        return errno(EINVAL);
    }

    // Only allow setting own capabilities
    let cur_pid = current_process().pid.0 as i32;
    if hdr.pid != 0 && hdr.pid != cur_pid {
        return errno(EPERM);
    }

    if data_ptr.is_null() {
        return errno(EFAULT);
    }

    let data_count: usize = if hdr.version == _LINUX_CAPABILITY_VERSION_1 {
        1
    } else {
        2
    };

    let data0 = match read_from_user::<CapData>(token, data_ptr as *const CapData) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };

    let mut data1 = CapData::default();
    if data_count == 2 {
        let data_ptr1 = unsafe { data_ptr.add(core::mem::size_of::<CapData>()) };
        data1 = match read_from_user::<CapData>(token, data_ptr1 as *const CapData) {
            Ok(v) => v,
            Err(_) => return errno(EFAULT),
        };
    }

    // Compute new capability sets
    let new_effective = (data0.effective as u64) | ((data1.effective as u64) << 32);
    let new_permitted = (data0.permitted as u64) | ((data1.permitted as u64) << 32);
    let new_inheritable = (data0.inheritable as u64) | ((data1.inheritable as u64) << 32);

    // Validate and update process capability sets
    let process = current_process();
    process.with_identity_mut(|inner| {
        // new_effective must be subset of new_permitted
        if (new_effective & !new_permitted) != 0 {
            return errno(EPERM);
        }
        // new_permitted must be subset of old_permitted (can't raise permitted)
        if (new_permitted & !inner.cap_permitted) != 0 {
            return errno(EPERM);
        }
        // new_inheritable must be subset of (old_inheritable | old_permitted), unless CAP_SETPCAP
        // With CAP_SETPCAP, still bounded by bounding set
        // CAP_SETPCAP = bit 8
        const CAP_SETPCAP: u64 = 1 << 8;
        if (new_inheritable & !(inner.cap_inheritable | inner.cap_permitted)) != 0 {
            if (inner.cap_effective & CAP_SETPCAP) == 0 {
                return errno(EPERM);
            }
            // Even with CAP_SETPCAP, can't exceed bounding set
            if (new_inheritable & !inner.cap_bounding) != 0 {
                return errno(EPERM);
            }
        }

        inner.cap_effective = new_effective;
        inner.cap_permitted = new_permitted;
        inner.cap_inheritable = new_inheritable;
        0
    })
}

/// prctl(option, arg2, arg3, arg4, arg5) - syscall 167
/// Process control operations.
pub fn sys_prctl(option: usize, arg2: usize, arg3: usize, _arg4: usize, _arg5: usize) -> isize {
    const PR_SET_PDEATHSIG: usize = 1;
    const PR_SET_DUMPABLE: usize = 4;
    const PR_GET_DUMPABLE: usize = 3;
    const PR_SET_KEEPCAPS: usize = 8;
    const PR_GET_KEEPCAPS: usize = 7;
    const PR_SET_TIMING: usize = 14;
    const PR_SET_NAME: usize = 15;
    const PR_GET_NAME: usize = 16;
    const PR_CAPBSET_READ: usize = 23;
    const PR_CAPBSET_DROP: usize = 24;
    const PR_SET_SECUREBITS: usize = 28;
    const PR_SET_TIMERSLACK: usize = 29;
    const PR_GET_TIMERSLACK: usize = 30;
    const PR_SET_SECCOMP: usize = 22;
    const PR_GET_SECCOMP: usize = 21;
    const SECCOMP_MODE_FILTER: usize = 2;
    const CAP_SETPCAP: u64 = 1 << 8;

    match option {
        PR_SET_PDEATHSIG => {
            if arg2 > MAX_SIG {
                return errno(EINVAL);
            }
            0
        }
        PR_SET_DUMPABLE => {
            if arg2 > 1 {
                return errno(EINVAL);
            }
            0
        }
        PR_SET_KEEPCAPS => 0,
        PR_SET_SECCOMP => {
            if arg2 != SECCOMP_MODE_FILTER {
                return errno(EINVAL);
            }
            if arg3 == 0 {
                return errno(EFAULT);
            }
            let token = current_user_token();
            let mut raw = [0u8; 16];
            if user_mem::copy_from_user(
                token,
                arg3 as *const u8,
                &mut raw,
                UserReadPolicy::StrictChecked,
            )
            .is_err()
            {
                return errno(EFAULT);
            }
            errno(EACCES)
        }
        PR_GET_DUMPABLE => {
            // Return 1 (SUID_DUMP_USER) - process is dumpable
            1
        }
        PR_GET_KEEPCAPS => {
            0 // Not keeping caps across setuid
        }
        PR_GET_SECCOMP => {
            0 // SECCOMP_MODE_DISABLED
        }
        PR_CAPBSET_READ => {
            // Return 1 if cap is in bounding set, 0 if not
            if arg2 >= 64 {
                return errno(EINVAL);
            }
            let process = current_process();
            process.with_identity(|inner| {
                if (inner.cap_bounding >> arg2) & 1 == 1 {
                    1
                } else {
                    0
                }
            })
        }
        PR_CAPBSET_DROP => {
            // Drop a capability from the bounding set
            if arg2 >= 64 {
                return errno(EINVAL);
            }
            let process = current_process();
            process.with_identity_mut(|inner| {
                if (inner.cap_effective & CAP_SETPCAP) == 0 {
                    return errno(EPERM);
                }
                inner.cap_bounding &= !(1u64 << arg2);
                0
            })
        }
        PR_SET_SECUREBITS => {
            let process = current_process();
            if (process.credentials_snapshot().cap_effective & CAP_SETPCAP) == 0 {
                return errno(EPERM);
            }
            0
        }
        PR_SET_TIMERSLACK => {
            let process = current_process();
            process.with_timers_mut(|timers| {
                if arg2 == 0 {
                    timers.timer_slack_ns = timers.default_timer_slack_ns;
                } else {
                    timers.timer_slack_ns = arg2;
                }
            });
            0
        }
        PR_GET_TIMERSLACK => {
            let process = current_process();
            process.with_timers(|timers| timers.timer_slack_ns as isize)
        }
        PR_SET_TIMING => errno(EINVAL),
        PR_SET_NAME => {
            if arg2 == 0 {
                return errno(EFAULT);
            }
            let token = current_user_token();
            let mut name = [0u8; 16];
            if user_mem::copy_from_user(
                token,
                arg2 as *const u8,
                &mut name,
                UserReadPolicy::StrictChecked,
            )
            .is_err()
            {
                return errno(EFAULT);
            }
            0
        }
        PR_GET_NAME => {
            // Get process name - return empty string
            if arg2 != 0 {
                let token = current_user_token();
                let name_bytes = [0u8; 16];
                let _ = copy_to_user(token, arg2 as *mut u8, &name_bytes);
            }
            0
        }
        _ => {
            // Unknown prctl option - return EINVAL
            errno(EINVAL)
        }
    }
}

pub fn sys_umask(mask: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_umask", pid);
    }
    let new_mask = mask & 0o777;
    let mut state = UMASK_STATE.exclusive_access();
    let old = *state;
    *state = new_mask;
    old as isize
}

pub fn get_current_umask() -> usize {
    *UMASK_STATE.exclusive_access()
}

pub fn sys_getrlimit(resource: usize, rlim: *mut RLimit) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!("kernel:pid[{}] sys_getrlimit resource={}", pid, resource);
    }
    if rlim.is_null() {
        return errno(EFAULT);
    }
    if resource >= RLIMIT_NLIMITS {
        return errno(EINVAL);
    }
    let process = current_process();
    let limit = process.with_limits(|limits| limits.rlimits[resource]);
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&limit as *const RLimit) as *const u8,
            core::mem::size_of::<RLimit>(),
        )
    };
    drop(process);
    let token = current_user_token();
    match copy_to_user(token, rlim as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

pub fn sys_prlimit64(
    pid: usize,
    resource: usize,
    new_limit: *const RLimit,
    old_limit: *mut RLimit,
) -> isize {
    let current_pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(current_pid) {
        trace!(
            "kernel:pid[{}] sys_prlimit64 pid={} resource={}",
            current_pid,
            pid,
            resource
        );
    }
    if resource >= RLIMIT_NLIMITS {
        return errno(EINVAL);
    }
    if pid != 0 && pid != current_pid {
        return errno(ESRCH);
    }

    let token = current_user_token();
    let process = current_process();
    let old = process.with_limits(|limits| limits.rlimits[resource]);
    if !old_limit.is_null() {
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old as *const RLimit) as *const u8,
                core::mem::size_of::<RLimit>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_limit as *mut u8, bytes) {
            return err;
        }
    }
    if !new_limit.is_null() {
        let new_val = match read_from_user::<RLimit>(token, new_limit) {
            Ok(v) => v,
            Err(_) => return errno(EFAULT),
        };
        if new_val.rlim_cur > new_val.rlim_max {
            return errno(EINVAL);
        }
        process.with_limits_mut(|limits| limits.rlimits[resource] = new_val);
    }
    0
}

/// Exit all threads in the process (Linux do_group_exit semantics).
/// Sends SIGKILL to all sibling threads, then exits the current thread.
pub fn sys_exit_group(exit_code: i32) -> ! {
    let task = current_task().unwrap();
    let process = task.process.upgrade().unwrap();
    let pid = process.getpid();
    trace!(
        "kernel:pid[{}] sys_exit_group (exit_code={})",
        pid,
        exit_code
    );

    // Send SIGKILL to all sibling threads so they exit on next schedule
    {
        for other_task in process.tasks_snapshot() {
            if !Arc::ptr_eq(&other_task, &task) {
                if other_task.exit_code().is_some() {
                    continue; // already exited
                }
                other_task.insert_pending_signal(SignalFlags::SIGKILL);
                if other_task.status() == TaskStatus::Blocked {
                    futex_remove_waiter_any(&other_task);
                    other_task.mark_interrupted();
                    other_task.set_status(TaskStatus::Ready);
                    add_task(other_task.clone());
                }
            }
        }
    }
    drop(process);
    drop(task);

    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit_group!");
}

pub fn sys_shutdown() -> ! {
    trace!("kernel:pid[{}] sys_shutdown", current_process().pid.0);
    crate::fs::shutdown_filesystems();
    arch::shutdown();
}

/// mprotect - change memory region protection
///
/// # Arguments
/// * `addr` - starting address of memory region (must be page-aligned)
/// * `len` - length of memory region
/// * `prot` - new protection flags (PROT_READ | PROT_WRITE | PROT_EXEC)
///
/// # Returns
/// * On success: 0
/// * On error: -errno
pub fn sys_mprotect(addr: usize, len: usize, prot: usize) -> isize {
    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_mprotect addr=0x{:x} len=0x{:x} prot=0x{:x}",
            pid,
            addr,
            len,
            prot
        );
    }

    const PROT_READ: usize = 0x1;
    const PROT_WRITE: usize = 0x2;
    const PROT_EXEC: usize = 0x4;

    use crate::config::PAGE_SIZE;

    // Check alignment
    if addr % PAGE_SIZE != 0 {
        return errno(EINVAL);
    }

    if len == 0 {
        return 0;
    }
    if addr == 0 {
        return errno(ENOMEM);
    }
    if (prot & !(PROT_READ | PROT_WRITE | PROT_EXEC)) != 0 {
        return errno(EINVAL);
    }

    // Convert prot flags to MapPermission
    let mut map_perm = MapPermission::U;
    if (prot & PROT_READ) != 0 {
        map_perm |= MapPermission::R;
    }
    if (prot & PROT_WRITE) != 0 {
        map_perm |= MapPermission::W;
    }
    if (prot & PROT_EXEC) != 0 {
        map_perm |= MapPermission::X;
    }

    // Round up length to page boundary
    let page_count = (len + PAGE_SIZE - 1) / PAGE_SIZE;
    let Some(end_addr) = addr.checked_add(page_count * PAGE_SIZE) else {
        return errno(ENOMEM);
    };

    // Change protection for the memory region
    let process = current_process();
    let result = process.with_memory_set_mut(|memory_set| {
        memory_set.change_protection(VirtAddr(addr), VirtAddr(end_addr), map_perm)
    });

    match result {
        Ok(()) => 0,
        Err(ProtectError::Unmapped) => errno(ENOMEM),
        Err(ProtectError::AccessDenied) => errno(EACCES),
    }
}

/// rt_sigtimedwait - Wait for signal
///
/// # Arguments
/// - set: pointer to signal set to wait for
/// - info: pointer to siginfo_t structure (output)
/// - timeout: pointer to timespec structure (timeout)
/// - sigsetsize: size of signal set
///
/// # Returns
/// - Success: signal number
/// - Failure: -errno
pub fn sys_rt_sigtimedwait(
    set: *const usize,
    info: *mut usize,
    timeout: *const TimeSpec,
    _sigsetsize: usize,
) -> isize {
    const SIG_IGN_HANDLER: usize = 1;

    // Validate pointers
    if set.is_null() {
        return errno(EFAULT);
    }

    // Read the signal set from user space
    let token = current_user_token();
    let sigset = match read_from_user::<usize>(token, set) {
        Ok(v) => v,
        Err(_) => return errno(EFAULT),
    };

    // Read timeout if provided
    let timeout_us = if !timeout.is_null() {
        let ts = match read_from_user::<TimeSpec>(token, timeout) {
            Ok(v) => v,
            Err(_) => return errno(EFAULT),
        };
        if ts.tv_nsec >= 1_000_000_000 {
            return errno(EINVAL);
        }
        Some(
            ts.tv_sec
                .saturating_mul(1_000_000)
                .saturating_add(ts.tv_nsec / 1_000),
        )
    } else {
        None
    };
    // musl's sigtimedwait wrapper retries internally on EINTR. With an empty
    // wait-set and no timeout this can spin forever when peer threads keep
    // delivering signals (observed in LTP sigtimedwait01 on musl).
    if sigset == 0 && timeout_us.is_none() {
        return errno(EINVAL);
    }

    let start = get_time_us();
    let deadline = timeout_us.map(|delta| start.saturating_add(delta));

    loop {
        let mut found: Option<(usize, bool, i32, i32)> = None;
        let mut interrupted_by_other = false;
        {
            let process = current_process();
            let task = current_task().unwrap();
            let process_pending = process.pending_signal();
            let (task_pending, signal_mask) =
                task.with_signals(|signals| (signals.signal_pending, signals.signal_mask));
            let pending_any = process_pending | task_pending;

            // SIGKILL/SIGSTOP must break out of kernel wait loops immediately.
            // Otherwise a task blocked in rt_sigtimedwait() can become unkillable.
            if pending_any.intersects(SignalFlags::SIGKILL | SignalFlags::SIGSTOP) {
                interrupted_by_other = true;
            }

            for signum in 1..=MAX_SIG {
                if interrupted_by_other {
                    break;
                }
                // SIGKILL 和 SIGSTOP 不能被等待
                if signum == SIGKILL || signum == SIGSTOP {
                    continue;
                }
                let flag = SignalFlags::from_bits_truncate(1u64 << (signum - 1));
                if !pending_any.contains(flag) {
                    continue;
                }
                if (sigset & (1usize << (signum - 1))) != 0 {
                    if process_pending.contains(flag) {
                        let (sender_pid, si_code) = process.process_signal_siginfo(signum);
                        found = Some((signum, true, sender_pid, si_code));
                    } else {
                        found = Some((signum, false, 0, 0));
                    }
                    break;
                }
                // Non-waited unmasked signal interrupts with EINTR.
                if !signal_mask.contains(flag) {
                    let action = process.with_signal_action(signum, |action| action);
                    if action.handler == SIG_IGN_HANDLER
                        || (action.handler == 0 && signal_default_ignored(signum))
                    {
                        continue;
                    }
                    interrupted_by_other = true;
                    break;
                }
            }
        }

        if let Some((signum, from_process, sender_pid, si_code)) = found {
            if !info.is_null() {
                let si = LinuxSigInfo::for_signal(signum as i32, sender_pid, si_code);
                if let Err(err) = write_waitid_siginfo(info as *mut u8, si) {
                    return err;
                }
            }
            let process = current_process();
            let task = current_task().unwrap();
            let flag = SignalFlags::from_bits_truncate(1u64 << (signum - 1));
            if from_process {
                process.take_process_signal(signum);
            } else {
                task.remove_pending_signal(flag);
            }
            return signum as isize;
        }

        if interrupted_by_other {
            return errno(EINTR);
        }

        if let Some(dl) = deadline {
            if get_time_us() >= dl {
                return errno(EAGAIN);
            }
        }
        suspend_current_and_run_next();
    }
}

pub fn sys_membarrier(_cmd: isize, _flags: isize) -> isize {
    0
}

pub fn sys_personality(persona: usize) -> isize {
    let pid = current_process().pid.0;
    let mut state = PERSONALITY_STATE.exclusive_access();
    let old = *state.get(&pid).unwrap_or(&0);
    if persona != usize::MAX && persona != u32::MAX as usize {
        state.insert(pid, persona);
    }
    old as isize
}

pub fn sys_unshare(flags: usize) -> isize {
    const CLONE_FS: usize = 0x0000_0200;
    const CLONE_FILES: usize = 0x0000_0400;
    const CLONE_NEWNS: usize = 0x0002_0000;
    const SUPPORTED: usize = CLONE_FS | CLONE_FILES | CLONE_NEWNS;

    if flags & !SUPPORTED != 0 {
        return errno(EINVAL);
    }
    if flags & CLONE_NEWNS != 0 {
        let process = current_process();
        if process.effective_uid() != 0 {
            return errno(EPERM);
        }
    }
    0
}

fn normalize_sched_pid(pid: usize) -> Result<usize, isize> {
    if (pid as isize) < 0 {
        return Err(errno(EINVAL));
    }
    if pid == 0 {
        Ok(current_process().pid.0)
    } else if pid2process(pid).is_some() {
        Ok(pid)
    } else {
        Err(errno(ESRCH))
    }
}

/// sched_setscheduler(pid, policy, param)
pub fn sys_sched_setscheduler(pid: usize, policy: i32, param: *const u8) -> isize {
    const SCHED_OTHER: i32 = 0;
    const SCHED_FIFO: i32 = 1;
    const SCHED_RR: i32 = 2;

    let target_pid = match normalize_sched_pid(pid) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    if !matches!(policy, SCHED_OTHER | SCHED_FIFO | SCHED_RR) {
        return errno(EINVAL);
    }
    if param.is_null() {
        return errno(EFAULT);
    }
    let token = current_user_token();
    let sched_priority = match read_from_user::<i32>(token, param as *const i32) {
        Ok(priority) => priority,
        Err(_) => return errno(EFAULT),
    };
    let valid_priority = match policy {
        SCHED_OTHER => sched_priority == 0,
        SCHED_FIFO | SCHED_RR => (1..=99).contains(&sched_priority),
        _ => false,
    };
    if !valid_priority {
        return errno(EINVAL);
    }
    SCHED_POLICIES.exclusive_access().insert(target_pid, policy);
    0
}

/// sched_getscheduler(pid) -> policy
pub fn sys_sched_getscheduler(pid: usize) -> isize {
    let target_pid = match normalize_sched_pid(pid) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    SCHED_POLICIES
        .exclusive_access()
        .get(&target_pid)
        .copied()
        .unwrap_or(0) as isize
}

/// sched_getparam(pid, param)
/// Write sched_priority = 0 (for SCHED_OTHER).
pub fn sys_sched_getparam(pid: usize, param: *mut u8) -> isize {
    if let Err(err) = normalize_sched_pid(pid) {
        return err;
    }
    if param.is_null() {
        return errno(EINVAL);
    }
    let token = current_user_token();
    // struct sched_param { int sched_priority; }
    let priority: i32 = 0;
    match copy_to_user(token, param, &priority.to_le_bytes()) {
        Ok(_) => 0,
        Err(e) => e,
    }
}

/// sched_rr_get_interval(pid, tp)
pub fn sys_sched_rr_get_interval(pid: usize, interval: *mut TimeSpec) -> isize {
    const SCHED_FIFO: i32 = 1;
    const SCHED_RR: i32 = 2;
    const RR_TIMESLICE_NS: usize = 100_000_000;

    let target_pid = match normalize_sched_pid(pid) {
        Ok(pid) => pid,
        Err(err) => return err,
    };
    if interval.is_null() {
        return errno(EFAULT);
    }

    let policy = SCHED_POLICIES
        .exclusive_access()
        .get(&target_pid)
        .copied()
        .unwrap_or(0);
    let spec = match policy {
        SCHED_RR => TimeSpec {
            tv_sec: 0,
            tv_nsec: RR_TIMESLICE_NS,
        },
        SCHED_FIFO => TimeSpec {
            tv_sec: 0,
            tv_nsec: 0,
        },
        _ => TimeSpec {
            tv_sec: 0,
            tv_nsec: RR_TIMESLICE_NS,
        },
    };

    let token = current_user_token();
    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&spec as *const TimeSpec) as *const u8,
            core::mem::size_of::<TimeSpec>(),
        )
    };
    match copy_to_user(token, interval as *mut u8, bytes) {
        Ok(_) => 0,
        Err(err) => err,
    }
}

/// sched_setattr(pid, attr, flags)
/// Stub: pretend success.
pub fn sys_sched_setattr(_pid: usize, _attr: usize, _flags: usize) -> isize {
    warn!("[sched] sched_setattr: stub, returning 0");
    0
}

/// sched_getattr(pid, attr, size, flags)
/// Return a sched_attr struct with SCHED_OTHER policy and priority 0.
pub fn sys_sched_getattr(_pid: usize, attr: *mut u8, size: usize, _flags: usize) -> isize {
    warn!("[sched] sched_getattr: called, size={}", size);
    if attr.is_null() || size < 48 {
        return errno(EINVAL);
    }
    let token = current_user_token();
    // struct sched_attr (48 bytes minimum):
    //   u32 size = 48
    //   u32 sched_policy = 0 (SCHED_OTHER)
    //   u64 sched_flags = 0
    //   s32 sched_nice = 0
    //   u32 sched_priority = 0
    //   u64 sched_runtime = 0
    //   u64 sched_deadline = 0
    //   u64 sched_period = 0
    let mut buf = [0u8; 48];
    buf[0..4].copy_from_slice(&48u32.to_le_bytes()); // size = 48
                                                     // All other fields are 0 (SCHED_OTHER, no priority, etc.)
    match copy_to_user(token, attr, &buf) {
        Ok(_) => 0,
        Err(e) => e,
    }
}

/// sched_setaffinity(pid, cpusetsize, mask)
/// Stub: accept and ignore — we are single-core, so any mask is fine.
pub fn sys_sched_setaffinity(_pid: usize, _cpusetsize: usize, _mask: *const u8) -> isize {
    info!("[sched] sched_setaffinity: stub, always success");
    0
}

/// get_mempolicy(mode, nodemask, maxnode, addr, flags)
/// Stub: return MPOL_DEFAULT (0) — we have no NUMA.
pub fn sys_get_mempolicy(
    mode: *mut i32,
    nodemask: *mut usize,
    maxnode: usize,
    _addr: usize,
    _flags: usize,
) -> isize {
    let token = current_user_token();
    // Write mode = MPOL_DEFAULT (0)
    if !mode.is_null() {
        let val: i32 = 0; // MPOL_DEFAULT
        if let Err(e) = copy_to_user(token, mode as *mut u8, &val.to_le_bytes()) {
            return e;
        }
    }
    // Write nodemask = node 0
    if !nodemask.is_null() && maxnode > 0 {
        let val: usize = 1; // node 0
        if let Err(e) = copy_to_user(token, nodemask as *mut u8, &val.to_le_bytes()) {
            return e;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// POSIX timers (timer_create / timer_settime / timer_gettime / timer_delete)
// ---------------------------------------------------------------------------

struct PosixTimer {
    #[allow(dead_code)]
    clockid: usize,
    #[allow(dead_code)]
    sigev_signo: i32,
    /// Expiry time in microseconds (absolute monotonic)
    value_us: u64,
    /// Interval for periodic timers (microseconds)
    interval_us: u64,
    overrun: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct ITimerSpec {
    it_interval: TimeSpec,
    it_value: TimeSpec,
}

/// timer_create(clockid_t clockid, struct sigevent *sevp, timer_t *timerid)
pub fn sys_timer_create(clockid: usize, sevp: *const u8, timerid: *mut usize) -> isize {
    if timerid.is_null() {
        return errno(EFAULT);
    }
    const MAX_CLOCKS: usize = 16;
    if clockid >= MAX_CLOCKS {
        return errno(EINVAL);
    }
    let pid = current_process().pid.0;
    let token = current_user_token();

    // struct sigevent layout on the tested 64-bit ABI:
    // sigev_value (8 bytes), sigev_signo (4 bytes), sigev_notify (4 bytes).
    const SIGEV_SIGNAL: i32 = 0;
    const SIGEV_NONE: i32 = 1;
    const SIGEV_THREAD: i32 = 2;
    const SIGEV_THREAD_ID: i32 = 4;

    let (sigev_signo, sigev_notify) = if !sevp.is_null() {
        let signo_ptr = (sevp as usize + 8) as *const i32;
        let notify_ptr = (sevp as usize + 12) as *const i32;
        let signo = match read_from_user::<i32>(token, signo_ptr) {
            Ok(v) => v,
            Err(err) => return err,
        };
        let notify = match read_from_user::<i32>(token, notify_ptr) {
            Ok(v) => v,
            Err(err) => return err,
        };
        match notify {
            SIGEV_SIGNAL | SIGEV_THREAD | SIGEV_THREAD_ID
                if signo <= 0 || signo as usize > crate::task::MAX_SIG =>
            {
                return errno(EINVAL);
            }
            SIGEV_SIGNAL | SIGEV_NONE | SIGEV_THREAD | SIGEV_THREAD_ID => {}
            _ => return errno(EINVAL),
        }
        (signo, notify)
    } else {
        (14, SIGEV_SIGNAL)
    };

    let timer_id = {
        let mut next_id = POSIX_TIMER_NEXT_ID.exclusive_access();
        let id = *next_id;
        *next_id += 1;
        id
    };

    let timer = PosixTimer {
        clockid,
        sigev_signo: if sigev_notify == SIGEV_NONE {
            0
        } else {
            sigev_signo
        },
        value_us: 0,
        interval_us: 0,
        overrun: 0,
    };

    POSIX_TIMERS
        .exclusive_access()
        .insert((pid, timer_id), timer);

    // Linux timer_t is int-sized on the tested ABIs; do not overwrite the
    // adjacent user stack slot by copying a native usize.
    let id_bytes = (timer_id as i32).to_ne_bytes();
    if let Err(err) = copy_to_user(token, timerid as *mut u8, &id_bytes) {
        return err;
    }
    0
}

/// timer_settime(timer_t timerid, int flags, const struct itimerspec *new_value,
///               struct itimerspec *old_value)
pub fn sys_timer_settime(
    timerid: usize,
    flags: i32,
    new_value: *const u8,
    old_value: *mut u8,
) -> isize {
    let pid = current_process().pid.0;
    let token = current_user_token();

    let key = (pid, timerid);

    if new_value.is_null() {
        return errno(EINVAL);
    }

    // Read new itimerspec from userspace
    let new_spec = match read_from_user::<ITimerSpec>(token, new_value as *const ITimerSpec) {
        Ok(v) => v,
        Err(err) => return err,
    };

    const NSEC_PER_SEC: usize = 1_000_000_000;
    if new_spec.it_interval.tv_nsec >= NSEC_PER_SEC || new_spec.it_value.tv_nsec >= NSEC_PER_SEC {
        return errno(EINVAL);
    }

    let mut timers = POSIX_TIMERS.exclusive_access();
    let timer = match timers.get_mut(&key) {
        Some(t) => t,
        None => return errno(EINVAL),
    };

    // Write old value if requested
    if !old_value.is_null() {
        let remaining_us = if timer.value_us > 0 {
            let now = get_time_us() as u64;
            if timer.value_us > now {
                timer.value_us - now
            } else {
                0
            }
        } else {
            0
        };
        let old_spec = ITimerSpec {
            it_interval: TimeSpec {
                tv_sec: (timer.interval_us / 1_000_000) as usize,
                tv_nsec: ((timer.interval_us % 1_000_000) * 1000) as usize,
            },
            it_value: TimeSpec {
                tv_sec: (remaining_us / 1_000_000) as usize,
                tv_nsec: ((remaining_us % 1_000_000) * 1000) as usize,
            },
        };
        let bytes = unsafe {
            core::slice::from_raw_parts(
                (&old_spec as *const ITimerSpec) as *const u8,
                core::mem::size_of::<ITimerSpec>(),
            )
        };
        if let Err(err) = copy_to_user(token, old_value as *mut u8, bytes) {
            return err;
        }
    }

    // Compute new expiry
    let interval_us =
        new_spec.it_interval.tv_sec as u64 * 1_000_000 + new_spec.it_interval.tv_nsec as u64 / 1000;
    let value_us =
        new_spec.it_value.tv_sec as u64 * 1_000_000 + new_spec.it_value.tv_nsec as u64 / 1000;

    timer.interval_us = interval_us;

    const TIMER_ABSTIME: i32 = 1;
    if value_us == 0 {
        // Disarm timer
        timer.value_us = 0;
    } else if (flags & TIMER_ABSTIME) != 0 {
        // Absolute time
        timer.value_us = value_us;
    } else {
        // Relative time
        timer.value_us = get_time_us() as u64 + value_us;
    }

    0
}

/// timer_gettime(timer_t timerid, struct itimerspec *curr_value)
pub fn sys_timer_gettime(timerid: usize, curr_value: *mut u8) -> isize {
    let pid = current_process().pid.0;
    let token = current_user_token();

    let key = (pid, timerid);
    let timers = POSIX_TIMERS.exclusive_access();
    let timer = match timers.get(&key) {
        Some(t) => t,
        None => return errno(EINVAL),
    };

    let remaining_us = if timer.value_us > 0 {
        let now = get_time_us() as u64;
        if timer.value_us > now {
            timer.value_us - now
        } else {
            0
        }
    } else {
        0
    };

    let spec = ITimerSpec {
        it_interval: TimeSpec {
            tv_sec: (timer.interval_us / 1_000_000) as usize,
            tv_nsec: ((timer.interval_us % 1_000_000) * 1000) as usize,
        },
        it_value: TimeSpec {
            tv_sec: (remaining_us / 1_000_000) as usize,
            tv_nsec: ((remaining_us % 1_000_000) * 1000) as usize,
        },
    };

    let bytes = unsafe {
        core::slice::from_raw_parts(
            (&spec as *const ITimerSpec) as *const u8,
            core::mem::size_of::<ITimerSpec>(),
        )
    };
    if let Err(err) = copy_to_user(token, curr_value as *mut u8, bytes) {
        return err;
    }
    0
}

/// timer_getoverrun(timer_t timerid)
pub fn sys_timer_getoverrun(timerid: usize) -> isize {
    let pid = current_process().pid.0;
    let key = (pid, timerid);
    let timers = POSIX_TIMERS.exclusive_access();
    match timers.get(&key) {
        Some(t) => t.overrun as isize,
        None => errno(EINVAL),
    }
}

/// timer_delete(timer_t timerid)
pub fn sys_timer_delete(timerid: usize) -> isize {
    let pid = current_process().pid.0;
    let key = (pid, timerid);
    let mut timers = POSIX_TIMERS.exclusive_access();
    if timers.remove(&key).is_some() {
        0
    } else {
        errno(EINVAL)
    }
}

/// Called from timer interrupt to check which POSIX timers have expired.
/// Returns a list of (pid, sigev_signo) for timers that just fired.
pub fn posix_timers_check_expired(current_us: u64) -> alloc::vec::Vec<(usize, i32)> {
    let mut result = alloc::vec::Vec::new();
    let mut timers = POSIX_TIMERS.exclusive_access();
    for ((pid, _tid), timer) in timers.iter_mut() {
        if timer.value_us == 0 {
            continue;
        }
        if timer.value_us <= current_us {
            result.push((*pid, timer.sigev_signo));
            timer.overrun += 1;
            if timer.interval_us > 0 {
                timer.value_us = current_us + timer.interval_us;
            } else {
                timer.value_us = 0;
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// mremap
// ---------------------------------------------------------------------------

/// mremap(old_addr, old_size, new_size, flags, new_addr)
pub fn sys_mremap(
    old_addr: usize,
    old_size: usize,
    new_size: usize,
    flags: usize,
    _new_addr: usize,
) -> isize {
    const MREMAP_MAYMOVE: usize = 1;

    let pid = current_process().pid.0;
    if crate::syscall::should_trace_syscall(pid) {
        syscall!(
            "kernel:pid[{}] sys_mremap old_addr={:#x} old_size={:#x} new_size={:#x} flags={:#x}",
            pid,
            old_addr,
            old_size,
            new_size,
            flags
        );
    }

    if old_addr % PAGE_SIZE != 0 {
        return errno(EINVAL);
    }

    let old_size_aligned = (old_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
    let new_size_aligned = (new_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);

    // Shrink or same size: just return old address (no-op)
    if new_size_aligned <= old_size_aligned {
        return old_addr as isize;
    }

    // Grow: need to allocate more space
    if (flags & MREMAP_MAYMOVE) != 0 {
        // Allocate a new region, copy old data, unmap old region
        let new_addr = sys_mmap(0, new_size_aligned, 0x3 /* PROT_READ|PROT_WRITE */, 0x22 /* MAP_PRIVATE|MAP_ANON */, usize::MAX, 0);
        if new_addr < 0 {
            return new_addr; // propagate error
        }
        let new_addr = new_addr as usize;

        // Copy old data page-by-page via the page table
        let token = current_user_token();
        let page_table = PageTable::from_token(token);
        let copy_len = old_size_aligned;
        let mut offset = 0;
        while offset < copy_len {
            let src_va = old_addr + offset;
            let dst_va = new_addr + offset;
            let chunk = core::cmp::min(PAGE_SIZE, copy_len - offset);

            if let Some(src_pa) = page_table.translate_va(VirtAddr::from(src_va)) {
                if let Some(dst_pa) = page_table.translate_va(VirtAddr::from(dst_va)) {
                    let src_ptr = src_pa.0 as *const u8;
                    let dst_ptr = dst_pa.0 as *mut u8;
                    unsafe {
                        core::ptr::copy_nonoverlapping(src_ptr, dst_ptr, chunk);
                    }
                }
            }
            offset += chunk;
        }

        // Unmap old region
        sys_munmap(old_addr, old_size_aligned);

        new_addr as isize
    } else {
        // Cannot move, try to extend in place — for simplicity return ENOMEM
        errno(ENOMEM)
    }
}
