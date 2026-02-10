//! Implementation of syscalls
//!
//! The single entry point to all system calls, [`syscall()`], is called
//! whenever userspace wishes to perform a system call using the `ecall`
//! instruction. In this case, the processor raises an 'Environment call from
//! U-mode' exception, which is handled as one of the cases in
//! [`crate::trap::trap_handler`].
//!
//! For clarity, each single syscall is implemented as its own function, named
//! `sys_` then the name of the syscall. You can find functions like this in
//! submodules, and you should also implement syscalls this way.

/// getcwd syscall
const SYSCALL_GETCWD: usize = 17;
/// dup syscall
const SYSCALL_DUP: usize = 23;
/// dup3 syscall
const SYSCALL_DUP3: usize = 24;
/// unlinkat syscall
const SYSCALL_UNLINKAT: usize = 35;
/// mkdirat syscall
const SYSCALL_MKDIRAT: usize = 34;
/// linkat syscall
const SYSCALL_LINKAT: usize = 37;
/// umount2 syscall
const SYSCALL_UMOUNT2: usize = 39;
/// mount syscall
const SYSCALL_MOUNT: usize = 40;
/// chdir syscall
const SYSCALL_CHDIR: usize = 49;
/// openat syscall
const SYSCALL_OPENAT: usize = 56;
/// close syscall
const SYSCALL_CLOSE: usize = 57;
/// pipe2 syscall
const SYSCALL_PIPE2: usize = 59;
/// getdents64 syscall
const SYSCALL_GETDENTS64: usize = 61;
/// read syscall
const SYSCALL_READ: usize = 63;
/// write syscall
const SYSCALL_WRITE: usize = 64;
/// fstat syscall
const SYSCALL_FSTAT: usize = 80;
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// nanosleep syscall
const SYSCALL_NANOSLEEP: usize = 101;
/// yield syscall
const SYSCALL_YIELD: usize = 124;
/// thread_create syscall
const SYSCALL_THREAD_CREATE: usize = 460;
/// gettid syscall
const SYSCALL_GETTID: usize = 178;
/// waittid syscall
const SYSCALL_WAITTID: usize = 462;
/// mutex_create syscall
const SYSCALL_MUTEX_CREATE: usize = 463;
/// mutex_lock syscall
const SYSCALL_MUTEX_LOCK: usize = 464;
/// mutex_unlock syscall
const SYSCALL_MUTEX_UNLOCK: usize = 466;
/// semaphore_create syscall
const SYSCALL_SEMAPHORE_CREATE: usize = 467;
/// semaphore_up syscall
const SYSCALL_SEMAPHORE_UP: usize = 468;
/// semaphore_down syscall
const SYSCALL_SEMAPHORE_DOWN: usize = 470;
/// condvar_create syscall
const SYSCALL_CONDVAR_CREATE: usize = 471;
/// condvar_signal syscall
const SYSCALL_CONDVAR_SIGNAL: usize = 472;
/// condvar_wait syscall
const SYSCALL_CONDVAR_WAIT: usize = 473;
/// kill syscall
const SYSCALL_KILL: usize = 129;
/// sigaction syscall
const SYSCALL_SIGACTION: usize = 134;
/// sigprocmask syscall
const SYSCALL_SIGPROCMASK: usize = 135;
/// sigreturn syscall
const SYSCALL_SIGRETURN: usize = 139;
/// setpriority syscall
const SYSCALL_SET_PRIORITY: usize = 140;
/// times syscall
const SYSCALL_TIMES: usize = 153;
/// uname syscall
const SYSCALL_UNAME: usize = 160;
/// gettime syscall
const SYSCALL_GET_TIME: usize = 169;
/// getpid syscall
const SYSCALL_GETPID: usize = 172;
/// getppid syscall
const SYSCALL_GETPPID: usize = 173;
/// sbrk syscall
const SYSCALL_SBRK: usize = 214;
/// munmap syscall
const SYSCALL_MUNMAP: usize = 215;
/// fork syscall
const SYSCALL_FORK: usize = 220;
/// exec syscall
const SYSCALL_EXEC: usize = 221;
/// mmap syscall
const SYSCALL_MMAP: usize = 222;
/// waitpid syscall
const SYSCALL_WAITPID: usize = 260;
/// spawn syscall
const SYSCALL_SPAWN: usize = 400;

// shutdown syscall, never returns
const SYSCALL_SHUTDOWN: usize = 1001;

mod errno;
mod fs;
mod process;
mod sync;
mod thread;

use core::sync::atomic::{AtomicBool, Ordering};
use errno::ENOSYS;
use fs::*;
use process::*;
use sync::*;
use thread::*;

use crate::fs::Stat;
use crate::task::{current_process, SignalAction};

const fn parse_trace_pid(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut out: usize = 0;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b < b'0' || b > b'9' {
            return None;
        }
        out = out.saturating_mul(10).saturating_add((b - b'0') as usize);
        i += 1;
    }
    Some(out)
}

const TRACE_PID: Option<usize> = match option_env!("TRACE_PID") {
    Some(v) => parse_trace_pid(v),
    None => None,
};

const TRACE_NAME: Option<&str> = option_env!("TRACE_NAME");

/// Global switch for syscall tracing, useful for toggling via a debugger.
pub static SYSCALL_TRACE_ALL: AtomicBool = AtomicBool::new(false);

const SYSCALL_NAME_MAP: &[(usize, &str)] = &[
    (1, "fork"),
    (3, "wait"),
    (5, "setxattr"),
    (6, "kill/lsetxattr"),
    (7, "fsetxattr"),
    (8, "getxattr"),
    (9, "lgetxattr"),
    (10, "fgetxattr"),
    (11, "listxattr"),
    (12, "llistxattr"),
    (13, "sleep/flistxattr"),
    (14, "uptime/removexattr"),
    (15, "lremovexattr"),
    (16, "mknod/fremovexattr"),
    (17, "getcwd"),
    (19, "eventfd2"),
    (20, "epoll_create1"),
    (21, "epoll_ctl"),
    (22, "dup2"),
    (23, "dup"),
    (24, "dup3"),
    (25, "fcntl"),
    (26, "inotify_init1"),
    (29, "ioctl"),
    (32, "flock"),
    (33, "mknodat"),
    (34, "mkdirat"),
    (35, "unlinkat"),
    (36, "symlinkat"),
    (37, "linkat"),
    (39, "umount2"),
    (40, "mount"),
    (43, "statfs"),
    (44, "fstatfs"),
    (45, "truncate"),
    (46, "ftruncate"),
    (47, "fallocate"),
    (48, "faccessat"),
    (49, "chdir"),
    (50, "fchdir"),
    (51, "chroot"),
    (52, "fchmod"),
    (53, "fchmodat"),
    (54, "fchownat"),
    (55, "fchown/exec"),
    (56, "openat"),
    (57, "close"),
    (59, "pipe2"),
    (61, "getdents64"),
    (62, "lseek"),
    (63, "read"),
    (64, "write"),
    (65, "readv"),
    (66, "writev"),
    (67, "pread64"),
    (68, "pwrite64"),
    (69, "preadv"),
    (70, "pwritev"),
    (71, "sendfile"),
    (72, "pselect6"),
    (73, "ppoll"),
    (74, "signalfd4"),
    (75, "vmsplice"),
    (76, "splice"),
    (78, "readlinkat"),
    (79, "fstatat"),
    (80, "fstat"),
    (81, "sync"),
    (82, "fsync"),
    (83, "fdatasync"),
    (84, "sync_file_range"),
    (85, "timerfd_create"),
    (88, "symlink/utimensat"),
    (89, "acct"),
    (93, "exit"),
    (94, "exit_group"),
    (95, "waitid"),
    (96, "set_tid_address"),
    (98, "futex"),
    (99, "set_robust_list"),
    (100, "get_robust_list"),
    (101, "nanosleep"),
    (102, "getitimer"),
    (103, "setitimer"),
    (107, "timer_create"),
    (108, "timer_gettime"),
    (110, "timer_settime"),
    (111, "timer_delete"),
    (112, "clock_settime"),
    (113, "clock_gettime"),
    (114, "clock_getres"),
    (115, "clock_nanosleep"),
    (116, "syslog"),
    (117, "ptrace"),
    (119, "sched_setscheduler"),
    (120, "sched_getscheduler"),
    (121, "sched_getparam"),
    (122, "sched_setaffinity"),
    (123, "sched_getaffinity"),
    (124, "sched_yield"),
    (129, "kill_signal"),
    (130, "tkill"),
    (131, "tgkill"),
    (132, "sigaltstack"),
    (133, "rt_sigsuspend"),
    (134, "rt_sigaction"),
    (135, "rt_sigprocmask"),
    (136, "rt_sigpending"),
    (137, "rt_sigtimedwait"),
    (138, "rt_sigqueueinfo"),
    (139, "rt_sigreturn"),
    (140, "setpriority"),
    (141, "getpriority"),
    (142, "reboot"),
    (143, "setregrid"),
    (144, "setgid"),
    (145, "setreuid"),
    (146, "setuid"),
    (147, "setresuid"),
    (148, "getresuid"),
    (149, "setresgid"),
    (150, "getresgid"),
    (151, "setfsuid"),
    (152, "setfsgid"),
    (153, "times"),
    (154, "setpgid"),
    (155, "getpgid"),
    (156, "getsid"),
    (157, "setsid"),
    (158, "getgroups"),
    (159, "setgroups"),
    (160, "uname"),
    (161, "sethostname"),
    (162, "setdomainname"),
    (165, "getrusage"),
    (166, "umask"),
    (167, "prctl"),
    (169, "gettimeofday"),
    (171, "adjtimex"),
    (172, "getpid"),
    (173, "getppid"),
    (174, "getuid"),
    (175, "geteuid"),
    (176, "getgid"),
    (177, "getegid"),
    (178, "gettid"),
    (179, "sysinfo"),
    (186, "msgget"),
    (187, "msgsnd"),
    (188, "msgrcv"),
    (189, "msgctl"),
    (190, "semget"),
    (191, "semctl"),
    (192, "semtimedop"),
    (193, "semop"),
    (194, "shmget"),
    (195, "shmctl"),
    (196, "shmat"),
    (197, "shmdt"),
    (198, "socket"),
    (199, "socketpair"),
    (200, "bind"),
    (201, "listen"),
    (202, "accept"),
    (203, "connect"),
    (204, "getsockname"),
    (205, "getpeername"),
    (206, "sendto"),
    (207, "recvfrom"),
    (208, "setsockopt"),
    (209, "getsockopt"),
    (210, "shutdown_socket"),
    (211, "sendmsg"),
    (212, "recvmsg"),
    (213, "readahead"),
    (214, "brk"),
    (215, "munmap"),
    (216, "mremap"),
    (217, "add_key"),
    (219, "keyctl"),
    (220, "clone"),
    (221, "execve"),
    (222, "mmap"),
    (223, "fadvise64"),
    (226, "mprotect"),
    (227, "msync"),
    (228, "mlock"),
    (233, "madvise"),
    (234, "remap_file_pages"),
    (236, "get_mempolicy"),
    (241, "perf_event_open"),
    (242, "accept4"),
    (260, "wait4"),
    (261, "prlimit64"),
    (262, "fanotify_init"),
    (266, "clockadjtime"),
    (268, "setns"),
    (276, "renameat2"),
    (278, "getrandom"),
    (279, "memfd_create"),
    (280, "bpf"),
    (282, "userfaultfd"),
    (283, "membarrier"),
    (285, "copy_file_range"),
    (291, "statx"),
    (300, "strerror"),
    (301, "perror"),
    (425, "io_uring_setup"),
    (428, "open_tree"),
    (430, "fsopen"),
    (433, "fspick"),
    (434, "pidfd_open"),
    (435, "clone3"),
    (436, "close_range"),
    (437, "openat2"),
    (439, "faccessat2"),
    (447, "memfd_secret"),
    (452, "fchmodat2"),
    (1001, "xv6_mknod"),
    (1002, "xv6_shutdown"),
    (1003, "xv6_sbrk"),
];

fn syscall_name(syscall_id: usize) -> &'static str {
    for (num, name) in SYSCALL_NAME_MAP {
        if *num == syscall_id {
            return name;
        }
    }
    "unknown"
}

/// Check if a syscall trace should be emitted for the given pid.
pub fn should_trace_syscall(pid: usize) -> bool {
    if SYSCALL_TRACE_ALL.load(Ordering::Relaxed) {
        return true;
    }
    if let Some(target) = TRACE_PID {
        if target != pid {
            return false;
        }
    }
    if let Some(target) = TRACE_NAME {
        let process = current_process();
        let name = process.inner_exclusive_access().name.clone();
        return name == target;
    }
    true
}

/// handle syscall exception with `syscall_id` and other arguments
pub fn syscall(syscall_id: usize, args: [usize; 6]) -> isize {
    let process = current_process();
    let pid = process.pid.0;
    let name = process.inner_exclusive_access().name.clone();
    let trace = should_trace_syscall(pid);
    let mut known = true;
    let ret = match syscall_id {
        SYSCALL_GETCWD => sys_getcwd(args[0] as *mut u8, args[1]),
        SYSCALL_DUP => sys_dup(args[0]),
        SYSCALL_DUP3 => sys_dup3(args[0], args[1]),
        SYSCALL_OPENAT => sys_openat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYSCALL_MKDIRAT => sys_mkdirat(args[0] as isize, args[1] as *const u8, args[2] as u32),
        SYSCALL_CLOSE => sys_close(args[0]),
        SYSCALL_PIPE2 => sys_pipe2(args[0] as *mut i32, args[1] as u32),
        SYSCALL_LINKAT => sys_linkat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as isize,
            args[3] as *const u8,
            args[4] as u32,
        ),
        SYSCALL_UNLINKAT => sys_unlinkat(args[0] as isize, args[1] as *const u8, args[2] as u32),
        SYSCALL_UMOUNT2 => sys_umount2(args[0] as *const u8, args[1] as u32),
        SYSCALL_MOUNT => sys_mount(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *const u8,
            args[3] as u32,
            args[4],
        ),
        SYSCALL_CHDIR => sys_chdir(args[0] as *const u8),
        SYSCALL_GETDENTS64 => sys_getdents64(args[0], args[1] as *mut u8, args[2]),
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_FSTAT => sys_fstat(args[0], args[1] as *mut Stat),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_NANOSLEEP => sys_nanosleep(args[0] as *const TimeSpec, args[1] as *mut TimeSpec),
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_KILL => sys_kill(args[0], args[1] as i32),
        SYSCALL_SIGACTION => sys_sigaction(
            args[0] as i32,
            args[1] as *const SignalAction,
            args[2] as *mut SignalAction,
        ),
        SYSCALL_SIGPROCMASK => sys_sigprocmask(args[0] as u32),
        SYSCALL_SIGRETURN => sys_sigreturn(),
        SYSCALL_THREAD_CREATE => sys_thread_create(args[0], args[1]),
        SYSCALL_GETTID => sys_gettid(),
        SYSCALL_WAITTID => sys_waittid(args[0]) as isize,
        SYSCALL_MUTEX_CREATE => sys_mutex_create(args[0] == 1),
        SYSCALL_MUTEX_LOCK => sys_mutex_lock(args[0]),
        SYSCALL_MUTEX_UNLOCK => sys_mutex_unlock(args[0]),
        SYSCALL_SEMAPHORE_CREATE => sys_semaphore_create(args[0]),
        SYSCALL_SEMAPHORE_UP => sys_semaphore_up(args[0]),
        SYSCALL_SEMAPHORE_DOWN => sys_semaphore_down(args[0]),
        SYSCALL_CONDVAR_CREATE => sys_condvar_create(),
        SYSCALL_CONDVAR_SIGNAL => sys_condvar_signal(args[0]),
        SYSCALL_CONDVAR_WAIT => sys_condvar_wait(args[0], args[1]),
        SYSCALL_GETPID => sys_getpid(),
        SYSCALL_GETPPID => sys_getppid(),
        SYSCALL_FORK => sys_fork(),
        SYSCALL_EXEC => sys_exec(
            args[0] as *const u8,
            args[1] as *const usize,
            args[2] as *const usize,
        ),
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32),
        SYSCALL_TIMES => sys_times(args[0] as *mut Tms),
        SYSCALL_UNAME => sys_uname(args[0] as *mut UtsName),
        SYSCALL_GET_TIME => sys_get_time(args[0] as *mut TimeVal, args[1]),
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5]),
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]),
        SYSCALL_SBRK => sys_sbrk(args[0] as isize),
        SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        SYSCALL_SET_PRIORITY => sys_set_priority(args[0] as isize),
        SYSCALL_SHUTDOWN => sys_shutdown(),
        _ => {
            known = false;
            error!(
                "{} {}: unimplemented syscall {} ({})",
                pid,
                name,
                syscall_id,
                syscall_name(syscall_id)
            );
            -ENOSYS
        },
    };
    if known && trace && !(syscall_id == SYSCALL_WRITE && args[0] == 1) {
        trace!(
            "[syscall] pid={} name={} num={} args=[0x{:x},0x{:x},0x{:x},0x{:x},0x{:x},0x{:x}] ret={}",
            pid,
            name,
            syscall_id,
            args[0],
            args[1],
            args[2],
            args[3],
            args[4],
            args[5],
            ret
        );
    }
    ret
}
