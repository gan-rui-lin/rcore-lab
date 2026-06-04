//! Implementation of syscalls
//!
//! The single entry point to all system calls, [`syscall()`], is called
//! whenever userspace wishes to perform a system call using the `ecall`
//! instruction. In this case, the processor raises an 'Environment call from
//! U-mode' exception, which is handled as one of the cases in
//! [`crate::arch::trap_handler`].
//!
//! For clarity, each single syscall is implemented as its own function, named
//! `sys_` then the name of the syscall. You can find functions like this in
//! submodules, and you should also implement syscalls this way.

/// capget syscall
const SYSCALL_CAPGET: usize = 90;
/// capset syscall
const SYSCALL_CAPSET: usize = 91;
/// personality syscall
const SYSCALL_PERSONALITY: usize = 92;
/// getcwd syscall
const SYSCALL_GETCWD: usize = 17;
/// eventfd2 syscall
const SYSCALL_EVENTFD2: usize = 19;
/// epoll_create1 syscall
const SYSCALL_EPOLL_CREATE1: usize = 20;
/// epoll_ctl syscall
const SYSCALL_EPOLL_CTL: usize = 21;
/// epoll_pwait syscall
const SYSCALL_EPOLL_PWAIT: usize = 22;
const SYSCALL_SETXATTR: usize = 5;
const SYSCALL_LSETXATTR: usize = 6;
const SYSCALL_FSETXATTR: usize = 7;
const SYSCALL_GETXATTR: usize = 8;
const SYSCALL_LGETXATTR: usize = 9;
const SYSCALL_FGETXATTR: usize = 10;
const SYSCALL_LISTXATTR: usize = 11;
const SYSCALL_LLISTXATTR: usize = 12;
const SYSCALL_FLISTXATTR: usize = 13;
const SYSCALL_REMOVEXATTR: usize = 14;
const SYSCALL_LREMOVEXATTR: usize = 15;
const SYSCALL_FREMOVEXATTR: usize = 16;
/// dup syscall
const SYSCALL_DUP: usize = 23;
/// dup3 syscall
const SYSCALL_DUP3: usize = 24;
/// fcntl syscall
const SYSCALL_FCNTL: usize = 25;
/// ioctl syscall
const SYSCALL_IOCTL: usize = 29;
/// flock syscall
const SYSCALL_FLOCK: usize = 32;
/// mknodat syscall
const SYSCALL_MKNODAT: usize = 33;
/// unlinkat syscall
const SYSCALL_UNLINKAT: usize = 35;
/// mkdirat syscall
const SYSCALL_MKDIRAT: usize = 34;
/// symlinkat syscall
const SYSCALL_SYMLINKAT: usize = 36;
/// linkat syscall
const SYSCALL_LINKAT: usize = 37;
/// umount2 syscall
const SYSCALL_UMOUNT2: usize = 39;
/// mount syscall
const SYSCALL_MOUNT: usize = 40;
/// truncate syscall (by path)
const SYSCALL_TRUNCATE: usize = 45;
/// ftruncate syscall
const SYSCALL_FTRUNCATE: usize = 46;
/// fallocate syscall
const SYSCALL_FALLOCATE: usize = 47;
/// faccessat syscall
const SYSCALL_FACCESSAT: usize = 48;
/// fchdir syscall
const SYSCALL_FCHDIR: usize = 50;
/// chroot syscall
const SYSCALL_CHROOT: usize = 51;
/// fchmod syscall
const SYSCALL_FCHMOD: usize = 52;
/// fchmodat syscall
const SYSCALL_FCHMODAT: usize = 53;
/// chdir syscall
const SYSCALL_CHDIR: usize = 49;
/// fchownat syscall
const SYSCALL_FCHOWNAT: usize = 54;
/// fchown syscall
const SYSCALL_FCHOWN: usize = 55;
/// openat syscall
const SYSCALL_OPENAT: usize = 56;
/// close syscall
const SYSCALL_CLOSE: usize = 57;
/// pipe2 syscall
const SYSCALL_PIPE2: usize = 59;
/// getdents64 syscall
const SYSCALL_GETDENTS64: usize = 61;
/// lseek syscall
const SYSCALL_LSEEK: usize = 62;
/// read syscall
const SYSCALL_READ: usize = 63;
/// write syscall
const SYSCALL_WRITE: usize = 64;
/// readv syscall
const SYSCALL_READV: usize = 65;
/// writev syscall
const SYSCALL_WRITEV: usize = 66;
/// pread64 syscall
const SYSCALL_PREAD64: usize = 67;
/// pwrite64 syscall
const SYSCALL_PWRITE64: usize = 68;
/// preadv syscall
const SYSCALL_PREADV: usize = 69;
/// pwritev syscall
const SYSCALL_PWRITEV: usize = 70;
/// sendfile syscall
const SYSCALL_SENDFILE: usize = 71;
/// vmsplice syscall
const SYSCALL_VMSPLICE: usize = 75;
/// splice syscall
const SYSCALL_SPLICE: usize = 76;
/// tee syscall
const SYSCALL_TEE: usize = 77;
/// readlinkat syscall
const SYSCALL_READLINKAT: usize = 78;
/// pselect6 syscall
const SYSCALL_PSELECT6: usize = 72;
/// ppoll syscall
const SYSCALL_POLL: usize = 73;
/// fstatat syscall
const SYSCALL_FSTATAT: usize = 79;
/// fstat syscall
const SYSCALL_FSTAT: usize = 80;
/// sync syscall
const SYSCALL_SYNC: usize = 81;
/// fsync syscall
const SYSCALL_FSYNC: usize = 82;
/// fdatasync syscall
const SYSCALL_FDATASYNC: usize = 83;
/// timerfd_create syscall
const SYSCALL_TIMERFD_CREATE: usize = 85;
/// timerfd_settime syscall
const SYSCALL_TIMERFD_SETTIME: usize = 86;
/// timerfd_gettime syscall
const SYSCALL_TIMERFD_GETTIME: usize = 87;
/// utimensat syscall
const SYSCALL_UTIMENSAT: usize = 88;
/// statx syscall
const SYSCALL_STATX: usize = 291;
/// preadv2 syscall
const SYSCALL_PREADV2: usize = 286;
/// pwritev2 syscall
const SYSCALL_PWRITEV2: usize = 287;
/// renameat2 syscall
const SYSCALL_RENAMEAT2: usize = 276;
/// getrandom syscall
const SYSCALL_GETRANDOM: usize = 278;
/// memfd_create syscall
const SYSCALL_MEMFD_CREATE: usize = 279;
/// name_to_handle_at syscall
const SYSCALL_NAME_TO_HANDLE_AT: usize = 264;
/// open_by_handle_at syscall
const SYSCALL_OPEN_BY_HANDLE_AT: usize = 265;
const SYSCALL_INOTIFY_INIT1: usize = 26;
const SYSCALL_INOTIFY_ADD_WATCH: usize = 27;
const SYSCALL_INOTIFY_RM_WATCH: usize = 28;
const SYSCALL_SIGNALFD4: usize = 74;
const SYSCALL_COPY_FILE_RANGE: usize = 285;
/// exit syscall
const SYSCALL_EXIT: usize = 93;
/// exit_group syscall
const SYSCALL_EXIT_GROUP: usize = 94;
/// waitid syscall
const SYSCALL_WAITID: usize = 95;
/// set_tid_address syscall
const SYSCALL_SET_TID_ADDRESS: usize = 96;
/// unshare syscall
const SYSCALL_UNSHARE: usize = 97;
/// futex syscall
const SYSCALL_FUTEX: usize = 98;
/// set_robust_list syscall
const SYSCALL_SET_ROBUST_LIST: usize = 99;
/// get_robust_list syscall
const SYSCALL_GET_ROBUST_LIST: usize = 100;
/// nanosleep syscall
const SYSCALL_NANOSLEEP: usize = 101;
/// getitimer syscall
const SYSCALL_GETITIMER: usize = 102;
/// setitimer syscall
const SYSCALL_SETITIMER: usize = 103;
/// timer_create syscall
const SYSCALL_TIMER_CREATE: usize = 107;
/// timer_gettime syscall
const SYSCALL_TIMER_GETTIME: usize = 108;
/// timer_getoverrun syscall
const SYSCALL_TIMER_GETOVERRUN: usize = 109;
/// timer_settime syscall
const SYSCALL_TIMER_SETTIME: usize = 110;
/// timer_delete syscall
const SYSCALL_TIMER_DELETE: usize = 111;
/// clock_nanosleep syscall
const SYSCALL_CLOCK_NANOSLEEP: usize = 115;
/// ptrace syscall
const SYSCALL_PTRACE: usize = 117;
/// sched_setscheduler syscall
const SYSCALL_SCHED_SETSCHEDULER: usize = 119;
/// sched_getscheduler syscall
const SYSCALL_SCHED_GETSCHEDULER: usize = 120;
/// sched_getparam syscall
const SYSCALL_SCHED_GETPARAM: usize = 121;
/// sched_setaffinity syscall
const SYSCALL_SCHED_SETAFFINITY: usize = 122;
/// sched_getaffinity syscall
const SYSCALL_SCHED_GETAFFINITY: usize = 123;
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
/// tkill syscall
const SYSCALL_TKILL: usize = 130;
/// tgkill syscall
const SYSCALL_TGKILL: usize = 131;
/// rt_sigsuspend syscall
const SYSCALL_RT_SIGSUSPEND: usize = 133;
/// sigaction syscall
const SYSCALL_SIGACTION: usize = 134;
/// sigprocmask syscall
const SYSCALL_SIGPROCMASK: usize = 135;
/// rt_sigpending syscall
const SYSCALL_RT_SIGPENDING: usize = 136;
/// rt_sigtimedwait syscall
const SYSCALL_RT_SIGTIMEDWAIT: usize = 137;
/// rt_sigqueueinfo syscall
const SYSCALL_RT_SIGQUEUEINFO: usize = 138;
/// sigreturn syscall
const SYSCALL_SIGRETURN: usize = 139;
/// setpriority syscall
const SYSCALL_SET_PRIORITY: usize = 140;
/// getpriority syscall
const SYSCALL_GET_PRIORITY: usize = 141;
/// setregid syscall
const SYSCALL_SETREGID: usize = 143;
/// setgid syscall
const SYSCALL_SETGID: usize = 144;
/// setreuid syscall
const SYSCALL_SETREUID: usize = 145;
/// setuid syscall
const SYSCALL_SETUID: usize = 146;
/// setresuid syscall
const SYSCALL_SETRESUID: usize = 147;
/// getresuid syscall
const SYSCALL_GETRESUID: usize = 148;
/// setresgid syscall
const SYSCALL_SETRESGID: usize = 149;
/// getresgid syscall
const SYSCALL_GETRESGID: usize = 150;
/// setfsuid syscall
const SYSCALL_SETFSUID: usize = 151;
/// setfsgid syscall
const SYSCALL_SETFSGID: usize = 152;
/// times syscall
const SYSCALL_TIMES: usize = 153;
/// setpgid syscall
const SYSCALL_SETPGID: usize = 154;
/// getpgid syscall
const SYSCALL_GETPGID: usize = 155;
/// getsid syscall
const SYSCALL_GETSID: usize = 156;
/// setsid syscall
const SYSCALL_SETSID: usize = 157;
/// getgroups syscall
const SYSCALL_GETGROUPS: usize = 158;
/// setgroups syscall
const SYSCALL_SETGROUPS: usize = 159;
/// adjtimex syscall
const SYSCALL_ADJTIMEX: usize = 171;
const SYSCALL_PRCTL: usize = 167;
/// uname syscall
const SYSCALL_UNAME: usize = 160;
/// sethostname syscall
const SYSCALL_SETHOSTNAME: usize = 161;
/// setdomainname syscall
const SYSCALL_SETDOMAINNAME: usize = 162;
/// getrlimit syscall
const SYSCALL_GETRLIMIT: usize = 163;
/// getrusage syscall
const SYSCALL_GETRUSAGE: usize = 165;
/// umask syscall
const SYSCALL_UMASK: usize = 166;
/// statfs syscall
const SYSCALL_STATFS: usize = 43;
/// fstatfs syscall
const SYSCALL_FSTATFS: usize = 44;
/// clock_gettime syscall
const SYSCALL_CLOCK_GETTIME: usize = 113;
/// clock_settime syscall
const SYSCALL_CLOCK_SETTIME: usize = 112;
/// clock_getres syscall
const SYSCALL_CLOCK_GETRES: usize = 114;
/// syslog syscall
const SYSCALL_SYSLOG: usize = 116;
/// gettime syscall
const SYSCALL_GET_TIME: usize = 169;
/// getpid syscall
const SYSCALL_GETPID: usize = 172;
/// getppid syscall
const SYSCALL_GETPPID: usize = 173;
/// getuid syscall
const SYSCALL_GETUID: usize = 174;
/// geteuid syscall
const SYSCALL_GETEUID: usize = 175;
/// getgid syscall
const SYSCALL_GETGID: usize = 176;
/// getegid syscall
const SYSCALL_GETEGID: usize = 177;
/// sysinfo syscall
const SYSCALL_SYSINFO: usize = 179;
/// msgget syscall
const SYSCALL_MSGGET: usize = 186;
/// msgctl syscall
const SYSCALL_MSGCTL: usize = 187;
/// msgrcv syscall
const SYSCALL_MSGRCV: usize = 188;
/// msgsnd syscall
const SYSCALL_MSGSND: usize = 189;
/// shmget syscall
const SYSCALL_SHMGET: usize = 194;
/// shmctl syscall
const SYSCALL_SHMCTL: usize = 195;
/// shmat syscall
const SYSCALL_SHMAT: usize = 196;
/// shmdt syscall
const SYSCALL_SHMDT: usize = 197;
/// sbrk syscall
const SYSCALL_SBRK: usize = 214;
/// munmap syscall
const SYSCALL_MUNMAP: usize = 215;
/// mremap syscall
const SYSCALL_MREMAP: usize = 216;
/// fork syscall
const SYSCALL_FORK: usize = 220;
/// clone3 syscall
const SYSCALL_CLONE3: usize = 435;
/// exec syscall
const SYSCALL_EXEC: usize = 221;
/// mmap syscall
const SYSCALL_MMAP: usize = 222;
/// fadvise64 syscall
const SYSCALL_FADVISE64: usize = 223;
/// mprotect syscall
const SYSCALL_MPROTECT: usize = 226;
/// msync syscall
const SYSCALL_MSYNC: usize = 227;
const SYSCALL_MLOCK: usize = 228;
const SYSCALL_MUNLOCK: usize = 229;
const SYSCALL_MLOCKALL: usize = 230;
const SYSCALL_MUNLOCKALL: usize = 231;
const SYSCALL_MINCORE: usize = 232;
/// madvise syscall
const SYSCALL_MADVISE: usize = 233;
/// waitpid syscall
const SYSCALL_WAITPID: usize = 260;
/// prlimit64 syscall
const SYSCALL_PRLIMIT64: usize = 261;
/// spawn syscall
const SYSCALL_SPAWN: usize = 400;

// shutdown syscall, never returns
const SYSCALL_SHUTDOWN: usize = 1001;

// ---- Network syscalls ----
const SYSCALL_SOCKET: usize = 198;
const SYSCALL_SOCKETPAIR: usize = 199;
const SYSCALL_BIND: usize = 200;
const SYSCALL_LISTEN: usize = 201;
const SYSCALL_ACCEPT: usize = 202;
const SYSCALL_CONNECT: usize = 203;
const SYSCALL_GETSOCKNAME: usize = 204;
const SYSCALL_GETPEERNAME: usize = 205;
const SYSCALL_SENDTO: usize = 206;
const SYSCALL_RECVFROM: usize = 207;
const SYSCALL_SETSOCKOPT: usize = 208;
const SYSCALL_GETSOCKOPT: usize = 209;
const SYSCALL_SHUTDOWN_SOCKET: usize = 210;
const SYSCALL_SENDMSG: usize = 211;
const SYSCALL_RECVMSG: usize = 212;
const SYSCALL_ACCEPT4: usize = 242;
const SYSCALL_SCHED_SETATTR: usize = 274;
const SYSCALL_SCHED_GETATTR: usize = 275;
const SYSCALL_GET_MEMPOLICY: usize = 236;
const SYSCALL_MEMBARRIER: usize = 283;
const SYSCALL_CLOSE_RANGE: usize = 436;

mod errno;
mod fs;
mod ipc;
pub(crate) mod process;
mod sync;
mod thread;
pub(crate) mod user_mem;

use core::sync::atomic::{AtomicBool, Ordering};
use errno::ENOSYS;
use fs::*;
use ipc::*;
use process::*;
use sync::*;
use thread::*;

use crate::fs::Stat;
#[allow(unused_imports)] // debug: for current_trap_cx in syscall()
use crate::task::{current_process, current_task, current_trap_cx, RLimit, SignalAction};

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
pub static SYSCALL_TRACE_ALL: AtomicBool = AtomicBool::new(true);

/// Called from timer interrupt to check expired POSIX timers.
pub fn posix_timers_check_expired(current_us: u64) -> alloc::vec::Vec<(usize, i32)> {
    process::posix_timers_check_expired(current_us)
}

/// Procfs helper for `/proc/sysvipc/msg`.
pub fn proc_sysvipc_msg() -> alloc::string::String {
    ipc::proc_sysvipc_msg()
}

/// Procfs helper for `/proc/sysvipc/shm`.
pub fn proc_sysvipc_shm() -> alloc::string::String {
    ipc::proc_sysvipc_shm()
}

/// Procfs helper for `/proc/sys/kernel/msgmni`.
pub fn proc_kernel_msgmni() -> alloc::string::String {
    ipc::proc_kernel_msgmni()
}

/// Procfs helper for `/proc/sys/kernel/shmmax`.
pub fn proc_kernel_shmmax() -> alloc::string::String {
    ipc::proc_kernel_shmmax()
}

/// Procfs helper for `/proc/sys/kernel/shmmni`.
pub fn proc_kernel_shmmni() -> alloc::string::String {
    ipc::proc_kernel_shmmni()
}

/// Procfs helper for `/proc/sys/kernel/shmall`.
pub fn proc_kernel_shmall() -> alloc::string::String {
    ipc::proc_kernel_shmall()
}

/// Procfs writer for `/proc/sys/kernel/msgmni`.
pub fn set_msgmni_from_proc_write(buf: &[u8]) -> usize {
    ipc::set_msgmni_from_proc_write(buf)
}

/// Register SysV shared-memory attachments inherited by a forked process.
pub fn inherit_shm_for_process_fork(parent_pid: usize, child_pid: usize) {
    ipc::inherit_shm_attachments_for_fork(parent_pid, child_pid);
}

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
    (22, "epoll_pwait"),
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
    (77, "tee"),
    (78, "readlinkat"),
    (79, "fstatat"),
    (80, "fstat"),
    (81, "sync"),
    (82, "fsync"),
    (83, "fdatasync"),
    (84, "sync_file_range"),
    (85, "timerfd_create"),
    (86, "timerfd_settime"),
    (87, "timerfd_gettime"),
    (88, "symlink/utimensat"),
    (89, "acct"),
    (92, "personality"),
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
    (163, "getrlimit"),
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
    (187, "msgctl"),
    (188, "msgrcv"),
    (189, "msgsnd"),
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
    (264, "name_to_handle_at"),
    (265, "open_by_handle_at"),
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

// TODO: 现在这个实现好低效
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
    if !crate::logging::syscall_enabled() {
        return false;
    }
    if SYSCALL_TRACE_ALL.load(Ordering::Relaxed) {
        return true;
    }
    if TRACE_PID.is_none() && TRACE_NAME.is_none() {
        return false;
    }
    if let Some(target) = TRACE_PID {
        if target != pid {
            return false;
        }
    }
    if let Some(target) = TRACE_NAME {
        let process = current_process();
        let name = process.name();
        return name == target;
    }
    true
}

/// Called by task-exit path to release per-process SHM attachment records.
pub fn cleanup_shm_for_process_exit(pid: usize) {
    ipc::cleanup_shm_attachments_for_pid(pid);
}

/// handle syscall exception with `syscall_id` and other arguments
pub fn syscall(syscall_id: usize, args: [usize; 6]) -> isize {
    // !Avoid holding Arc<ProcessControlBlock> across potentially non-returning
    // syscalls (e.g. exit/exit_group), which can leak references.
    let pid = current_process().pid.0;
    let trace = should_trace_syscall(pid);
    let (name_for_trace, cwd_for_exec_trace) = if trace || (pid == 4 && syscall_id == SYSCALL_EXEC)
    {
        let process = current_process();
        let name = if trace { Some(process.name()) } else { None };
        let cwd = if pid == 4 && syscall_id == 221 {
            Some((process.name(), process.cwd()))
        } else {
            None
        };
        (name, cwd)
    } else {
        (None, None)
    };
    if let Some((name, cwd)) = cwd_for_exec_trace {
        trace!("[syscall] pid=4 entry name={} cwd={}", name, cwd);
    }
    // for debug
    if let Some(task) = current_task() {
        task.set_last_syscall(syscall_id);
    }
    let mut known = true;
    let ret = match syscall_id {
        SYSCALL_SETXATTR => sys_setxattr(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *const u8,
            args[3],
            args[4] as u32,
            true,
        ),
        SYSCALL_LSETXATTR => sys_setxattr(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *const u8,
            args[3],
            args[4] as u32,
            false,
        ),
        SYSCALL_FSETXATTR => sys_fsetxattr(
            args[0],
            args[1] as *const u8,
            args[2] as *const u8,
            args[3],
            args[4] as u32,
        ),
        SYSCALL_GETXATTR => sys_getxattr(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3],
            true,
        ),
        SYSCALL_LGETXATTR => sys_getxattr(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3],
            false,
        ),
        SYSCALL_FGETXATTR => {
            sys_fgetxattr(args[0], args[1] as *const u8, args[2] as *mut u8, args[3])
        }
        SYSCALL_LISTXATTR => sys_listxattr(args[0] as *const u8, args[1] as *mut u8, args[2], true),
        SYSCALL_LLISTXATTR => {
            sys_listxattr(args[0] as *const u8, args[1] as *mut u8, args[2], false)
        }
        SYSCALL_FLISTXATTR => sys_flistxattr(args[0], args[1] as *mut u8, args[2]),
        SYSCALL_REMOVEXATTR => sys_removexattr(args[0] as *const u8, args[1] as *const u8, true),
        SYSCALL_LREMOVEXATTR => sys_removexattr(args[0] as *const u8, args[1] as *const u8, false),
        SYSCALL_FREMOVEXATTR => sys_fremovexattr(args[0], args[1] as *const u8),
        SYSCALL_GETCWD => sys_getcwd(args[0] as *mut u8, args[1]),
        SYSCALL_EVENTFD2 => sys_eventfd2(args[0], args[1]),
        SYSCALL_EPOLL_CREATE1 => sys_epoll_create1(args[0]),
        SYSCALL_EPOLL_CTL => sys_epoll_ctl(args[0], args[1], args[2], args[3] as *const u8),
        SYSCALL_EPOLL_PWAIT => {
            sys_epoll_pwait(args[0], args[1] as *mut u8, args[2], args[3] as isize)
        }
        SYSCALL_INOTIFY_INIT1 => sys_inotify_init1(args[0]),
        SYSCALL_INOTIFY_ADD_WATCH => {
            sys_inotify_add_watch(args[0], args[1] as *const u8, args[2] as u32)
        }
        SYSCALL_INOTIFY_RM_WATCH => sys_inotify_rm_watch(args[0], args[1] as i32),
        SYSCALL_SIGNALFD4 => {
            sys_signalfd4(args[0] as isize, args[1] as *const usize, args[2], args[3])
        }
        SYSCALL_COPY_FILE_RANGE => sys_copy_file_range(
            args[0],
            args[1] as *mut i64,
            args[2],
            args[3] as *mut i64,
            args[4],
            args[5] as u32,
        ),
        SYSCALL_DUP => sys_dup(args[0]),
        SYSCALL_DUP3 => sys_dup3(args[0], args[1], args[2] as u32),
        SYSCALL_FCNTL => sys_fcntl(args[0], args[1] as i32, args[2]),
        SYSCALL_FLOCK => sys_flock(args[0], args[1] as i32),
        SYSCALL_IOCTL => sys_ioctl(args[0], args[1], args[2]),
        SYSCALL_MKNODAT => sys_mknodat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYSCALL_TRUNCATE => sys_truncate(args[0] as *const u8, args[1] as isize),
        SYSCALL_FTRUNCATE => sys_ftruncate(args[0], args[1] as isize),
        SYSCALL_FALLOCATE => {
            sys_fallocate(args[0], args[1] as u32, args[2] as isize, args[3] as isize)
        }
        SYSCALL_STATFS => sys_statfs(args[0] as *const u8, args[1] as *mut StatFs),
        SYSCALL_FSTATFS => sys_fstatfs(args[0], args[1] as *mut StatFs),
        SYSCALL_FACCESSAT => sys_faccessat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYSCALL_OPENAT => sys_openat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYSCALL_MKDIRAT => sys_mkdirat(args[0] as isize, args[1] as *const u8, args[2] as u32),
        SYSCALL_CLOSE => sys_close(args[0]),
        SYSCALL_CLOSE_RANGE => sys_close_range(args[0], args[1], args[2] as u32),
        SYSCALL_PIPE2 => sys_pipe2(args[0] as *mut i32, args[1] as u32),
        SYSCALL_LINKAT => sys_linkat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as isize,
            args[3] as *const u8,
            args[4] as u32,
        ),
        SYSCALL_SYMLINKAT => {
            sys_symlinkat(args[0] as *const u8, args[1] as isize, args[2] as *const u8)
        }
        SYSCALL_UNLINKAT => sys_unlinkat(args[0] as isize, args[1] as *const u8, args[2] as u32),
        SYSCALL_RENAMEAT2 => sys_renameat2(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as isize,
            args[3] as *const u8,
            args[4] as u32,
        ),
        SYSCALL_UMOUNT2 => sys_umount2(args[0] as *const u8, args[1] as u32),
        SYSCALL_MOUNT => sys_mount(
            args[0] as *const u8,
            args[1] as *const u8,
            args[2] as *const u8,
            args[3] as u32,
            args[4],
        ),
        SYSCALL_CHDIR => sys_chdir(args[0] as *const u8),
        SYSCALL_FCHDIR => sys_fchdir(args[0]),
        SYSCALL_CHROOT => sys_chroot(args[0] as *const u8),
        SYSCALL_FCHMOD => sys_fchmod(args[0], args[1] as u32),
        SYSCALL_FCHMODAT => sys_fchmodat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
        ),
        SYSCALL_FCHOWNAT => sys_fchownat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as u32,
            args[3] as u32,
            args[4] as u32,
        ),
        SYSCALL_FCHOWN => sys_fchown(args[0], args[1] as u32, args[2] as u32),
        SYSCALL_GETDENTS64 => sys_getdents64(args[0], args[1] as *mut u8, args[2]),
        SYSCALL_LSEEK => sys_lseek(args[0], args[1] as isize, args[2]),
        SYSCALL_READ => sys_read(args[0], args[1] as *const u8, args[2]),
        SYSCALL_WRITE => sys_write(args[0], args[1] as *const u8, args[2]),
        SYSCALL_SYNC => {
            crate::fs::sync_filesystems();
            0
        }
        SYSCALL_FSYNC => sys_fsync(args[0]),
        SYSCALL_FDATASYNC => sys_fdatasync(args[0]),
        SYSCALL_TIMERFD_CREATE => sys_timerfd_create(args[0] as i32, args[1] as i32),
        SYSCALL_TIMERFD_SETTIME => sys_timerfd_settime(
            args[0],
            args[1] as i32,
            args[2] as *const u8,
            args[3] as *mut u8,
        ),
        SYSCALL_TIMERFD_GETTIME => sys_timerfd_gettime(args[0], args[1] as *mut u8),
        SYSCALL_READV => sys_readv(args[0], args[1] as *const usize, args[2]),
        SYSCALL_WRITEV => sys_writev(args[0], args[1] as *const usize, args[2]),
        SYSCALL_PREAD64 => sys_pread64(args[0], args[1] as *const u8, args[2], args[3] as isize),
        SYSCALL_PWRITE64 => sys_pwrite64(args[0], args[1] as *const u8, args[2], args[3] as isize),
        SYSCALL_PREADV => sys_preadv(args[0], args[1] as *const usize, args[2], args[3] as isize),
        SYSCALL_PWRITEV => sys_pwritev(args[0], args[1] as *const usize, args[2], args[3] as isize),
        SYSCALL_FADVISE64 => {
            sys_posix_fadvise(args[0], args[1] as isize, args[2] as isize, args[3] as i32)
        }
        SYSCALL_SENDFILE => sys_sendfile(args[0], args[1], args[2] as *mut isize, args[3]),
        SYSCALL_VMSPLICE => sys_vmsplice(args[0], args[1] as *const usize, args[2], args[3] as u32),
        SYSCALL_SPLICE => sys_splice(
            args[0],
            args[1] as *mut isize,
            args[2],
            args[3] as *mut isize,
            args[4],
            args[5] as u32,
        ),
        SYSCALL_TEE => sys_tee(args[0], args[1], args[2], args[3] as u32),
        SYSCALL_PSELECT6 => sys_pselect6(
            args[0],
            args[1] as *mut usize,
            args[2] as *mut usize,
            args[3] as *mut usize,
            args[4] as *const TimeSpec,
            args[5],
        ),
        SYSCALL_READLINKAT => sys_readlinkat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3],
        ),
        SYSCALL_FSTATAT => sys_fstatat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as *mut Stat,
            args[3] as u32,
        ),
        SYSCALL_FSTAT => sys_fstat(args[0], args[1] as *mut Stat),
        SYSCALL_STATX => sys_statx(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as i32,
            args[3] as u32,
            args[4] as *mut Statx,
        ),
        SYSCALL_PREADV2 => {
            let offset = (((args[4] as u32 as u64) << 32) | (args[3] as u32 as u64)) as i64;
            sys_preadv2(
                args[0],
                args[1] as *const usize,
                args[2],
                offset as isize,
                args[5],
            )
        }
        SYSCALL_PWRITEV2 => {
            let offset = (((args[4] as u32 as u64) << 32) | (args[3] as u32 as u64)) as i64;
            sys_pwritev2(
                args[0],
                args[1] as *const usize,
                args[2],
                offset as isize,
                args[5],
            )
        }
        SYSCALL_UTIMENSAT => sys_utimensat(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as *const process::TimeSpec,
            args[3] as u32,
        ),
        SYSCALL_EXIT => sys_exit(args[0] as i32),
        SYSCALL_EXIT_GROUP => {
            let name = current_process().name();
            log::warn!(
                "[exit_group] pid={} name={} code={}",
                pid,
                name,
                args[0] as i32
            );
            sys_exit_group(args[0] as i32)
        }
        SYSCALL_WAITID => sys_waitid(
            args[0],
            args[1],
            args[2] as *mut u8,
            args[3] as i32,
            args[4] as *mut u8,
        ),
        SYSCALL_SET_TID_ADDRESS => sys_set_tid_address(args[0] as *mut i32),
        SYSCALL_UNSHARE => process::sys_unshare(args[0]),
        SYSCALL_SET_ROBUST_LIST => sys_set_robust_list(args[0], args[1]),
        SYSCALL_GET_ROBUST_LIST => {
            sys_get_robust_list(args[0], args[1] as *mut u8, args[2] as *mut u8)
        }
        SYSCALL_FUTEX => sys_futex(
            args[0] as *mut i32,
            args[1] as u32,
            args[2] as i32,
            args[3] as *const TimeSpec,
            args[4] as *mut i32,
            args[5] as i32,
        ),
        SYSCALL_GETITIMER => sys_getitimer(args[0] as isize, args[1] as *mut ITimerVal),
        SYSCALL_SETITIMER => sys_setitimer(
            args[0] as isize,
            args[1] as *const ITimerVal,
            args[2] as *mut ITimerVal,
        ),
        SYSCALL_TIMER_CREATE => {
            sys_timer_create(args[0], args[1] as *const u8, args[2] as *mut usize)
        }
        SYSCALL_TIMER_GETTIME => sys_timer_gettime(args[0], args[1] as *mut u8),
        SYSCALL_TIMER_GETOVERRUN => sys_timer_getoverrun(args[0]),
        SYSCALL_TIMER_SETTIME => sys_timer_settime(
            args[0],
            args[1] as i32,
            args[2] as *const u8,
            args[3] as *mut u8,
        ),
        SYSCALL_TIMER_DELETE => sys_timer_delete(args[0]),
        SYSCALL_NANOSLEEP => sys_nanosleep(args[0] as *const TimeSpec, args[1] as *mut TimeSpec),
        SYSCALL_CLOCK_NANOSLEEP => sys_clock_nanosleep(
            args[0],
            args[1],
            args[2] as *const TimeSpec,
            args[3] as *mut TimeSpec,
        ),
        SYSCALL_SCHED_SETSCHEDULER => {
            sys_sched_setscheduler(args[0], args[1] as i32, args[2] as *const u8)
        }
        SYSCALL_SCHED_GETSCHEDULER => sys_sched_getscheduler(args[0]),
        SYSCALL_SCHED_GETPARAM => sys_sched_getparam(args[0], args[1] as *mut u8),
        SYSCALL_SCHED_SETAFFINITY => sys_sched_setaffinity(args[0], args[1], args[2] as *const u8),
        SYSCALL_SCHED_GETAFFINITY => {
            sys_sched_getaffinity(args[0] as isize, args[1], args[2] as *mut u8)
        }
        SYSCALL_YIELD => sys_yield(),
        SYSCALL_KILL => sys_kill(args[0] as isize, args[1] as i32),
        SYSCALL_TKILL => process::sys_tkill(args[0] as isize, args[1] as i32),
        SYSCALL_TGKILL => process::sys_tgkill(args[0] as isize, args[1] as isize, args[2] as i32),
        SYSCALL_RT_SIGSUSPEND => process::sys_rt_sigsuspend(args[0] as *const usize, args[1]),
        SYSCALL_RT_SIGTIMEDWAIT => sys_rt_sigtimedwait(
            args[0] as *const usize,
            args[1] as *mut usize,
            args[2] as *const TimeSpec,
            args[3],
        ),
        SYSCALL_SIGACTION => sys_sigaction(
            args[0] as i32,
            args[1] as *const SignalAction,
            args[2] as *mut SignalAction,
            args[3],
        ),
        SYSCALL_SIGPROCMASK => sys_sigprocmask(
            args[0],
            args[1] as *const usize,
            args[2] as *mut usize,
            args[3],
        ),
        SYSCALL_RT_SIGPENDING => sys_rt_sigpending(args[0] as *mut usize, args[1]),
        SYSCALL_RT_SIGQUEUEINFO => {
            sys_rt_sigqueueinfo(args[0] as isize, args[1] as i32, args[2] as *const u8)
        }
        // sigaltstack: stub for glibc compatibility
        132 => 0,
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
        SYSCALL_CAPGET => process::sys_capget(args[0] as *mut u8, args[1] as *mut u8),
        SYSCALL_CAPSET => process::sys_capset(args[0] as *const u8, args[1] as *const u8),
        SYSCALL_GETUID => sys_getuid(),
        SYSCALL_GETEUID => sys_geteuid(),
        SYSCALL_GETGID => sys_getgid(),
        SYSCALL_GETEGID => sys_getegid(),
        SYSCALL_SETREGID => process::sys_setregid(args[0] as u32, args[1] as u32),
        SYSCALL_SETGID => sys_setgid(args[0] as u32),
        SYSCALL_SETREUID => process::sys_setreuid(args[0] as u32, args[1] as u32),
        SYSCALL_SETUID => sys_setuid(args[0] as u32),
        SYSCALL_SETRESUID => process::sys_setresuid(args[0] as u32, args[1] as u32, args[2] as u32),
        SYSCALL_GETRESUID => process::sys_getresuid(
            args[0] as *mut u32,
            args[1] as *mut u32,
            args[2] as *mut u32,
        ),
        SYSCALL_SETRESGID => process::sys_setresgid(args[0] as u32, args[1] as u32, args[2] as u32),
        SYSCALL_GETRESGID => process::sys_getresgid(
            args[0] as *mut u32,
            args[1] as *mut u32,
            args[2] as *mut u32,
        ),
        SYSCALL_SETFSUID => process::sys_setfsuid(args[0] as u32),
        SYSCALL_SETFSGID => process::sys_setfsgid(args[0] as u32),
        SYSCALL_SETPGID => sys_setpgid(args[0] as isize, args[1] as isize),
        SYSCALL_GETPGID => sys_getpgid(args[0] as isize),
        SYSCALL_GETSID => sys_getsid(args[0] as isize),
        SYSCALL_SETSID => sys_setsid(),
        SYSCALL_GETGROUPS => process::sys_getgroups(args[0] as i32, args[1] as *mut u32),
        SYSCALL_SETGROUPS => process::sys_setgroups(args[0], args[1] as *const u32),
        SYSCALL_GETRLIMIT => sys_getrlimit(args[0], args[1] as *mut RLimit),
        SYSCALL_GETRUSAGE => sys_getrusage(args[0] as isize, args[1] as *mut RUsage),
        SYSCALL_UMASK => sys_umask(args[0]),
        SYSCALL_SYSINFO => sys_sysinfo(args[0] as *mut process::SysInfo),
        SYSCALL_MSGGET => sys_msgget(args[0] as i32, args[1] as i32),
        SYSCALL_MSGSND => sys_msgsnd(args[0] as i32, args[1], args[2], args[3] as i32),
        SYSCALL_MSGRCV => sys_msgrcv(
            args[0] as i32,
            args[1],
            args[2],
            args[3] as isize,
            args[4] as i32,
        ),
        SYSCALL_MSGCTL => sys_msgctl(args[0] as i32, args[1] as i32, args[2]),
        SYSCALL_SHMGET => sys_shmget(args[0] as i32, args[1], args[2] as i32),
        SYSCALL_SHMAT => sys_shmat(args[0] as i32, args[1], args[2] as i32),
        SYSCALL_SHMDT => sys_shmdt(args[0]),
        SYSCALL_SHMCTL => sys_shmctl(args[0] as i32, args[1] as i32, args[2]),
        // clone ABI differs between architectures:
        //   RISC-V:     clone(flags, stack, ptid, tls, ctid)
        //   LoongArch:  clone(flags, stack, ptid, ctid, tls)
        SYSCALL_FORK => {
            #[cfg(target_arch = "riscv64")]
            {
                sys_clone(
                    args[0],
                    args[1] as *const u8,
                    args[2] as *mut i32,
                    args[3] as *mut i32,
                    args[4] as *mut i32,
                )
            }
            #[cfg(target_arch = "loongarch64")]
            {
                sys_clone(
                    args[0],
                    args[1] as *const u8,
                    args[2] as *mut i32,
                    args[4] as *mut i32,
                    args[3] as *mut i32,
                )
            }
        }
        SYSCALL_CLONE3 => process::sys_clone3(args[0] as *const u8, args[1]),
        SYSCALL_EXEC => sys_exec(
            args[0] as *const u8,
            args[1] as *const usize,
            args[2] as *const usize,
        ),
        SYSCALL_WAITPID => sys_waitpid(args[0] as isize, args[1] as *mut i32, args[2] as i32),
        SYSCALL_PRLIMIT64 => sys_prlimit64(
            args[0],
            args[1],
            args[2] as *const RLimit,
            args[3] as *mut RLimit,
        ),
        SYSCALL_GETRANDOM => sys_getrandom(args[0] as *mut u8, args[1], args[2] as u32),
        SYSCALL_MEMFD_CREATE => sys_memfd_create(args[0] as *const u8, args[1] as u32),
        SYSCALL_NAME_TO_HANDLE_AT => sys_name_to_handle_at(
            args[0] as isize,
            args[1] as *const u8,
            args[2] as *mut u8,
            args[3] as *mut i32,
            args[4] as u32,
        ),
        SYSCALL_OPEN_BY_HANDLE_AT => {
            sys_open_by_handle_at(args[0] as isize, args[1] as *const u8, args[2] as u32)
        }
        SYSCALL_TIMES => sys_times(args[0] as *mut Tms),
        SYSCALL_ADJTIMEX => process::sys_adjtimex(args[0] as *mut u8),
        SYSCALL_PRCTL => process::sys_prctl(args[0], args[1], args[2], args[3], args[4]),
        SYSCALL_UNAME => sys_uname(args[0] as *mut UtsName),
        SYSCALL_SETHOSTNAME => process::sys_sethostname(args[0] as *const u8, args[1]),
        SYSCALL_SETDOMAINNAME => process::sys_setdomainname(args[0] as *const u8, args[1]),
        SYSCALL_CLOCK_SETTIME => sys_clock_settime(args[0], args[1] as *const TimeSpec),
        SYSCALL_CLOCK_GETTIME => sys_clock_gettime(args[0], args[1] as *mut TimeSpec),
        SYSCALL_CLOCK_GETRES => sys_clock_getres(args[0], args[1] as *mut TimeSpec),
        SYSCALL_SYSLOG => sys_syslog(args[0], args[1] as *mut u8, args[2]),
        SYSCALL_GET_TIME => sys_get_time(args[0] as *mut TimeVal, args[1]),
        SYSCALL_PERSONALITY => process::sys_personality(args[0]),
        SYSCALL_PTRACE => sys_ptrace(args[0], args[1] as isize, args[2], args[3]),
        SYSCALL_MMAP => sys_mmap(args[0], args[1], args[2], args[3], args[4], args[5]),
        SYSCALL_MUNMAP => sys_munmap(args[0], args[1]),
        SYSCALL_MREMAP => sys_mremap(args[0], args[1], args[2], args[3], args[4]),
        SYSCALL_MPROTECT => sys_mprotect(args[0], args[1], args[2]),
        SYSCALL_MSYNC => sys_msync(args[0], args[1], args[2]),
        SYSCALL_MLOCK => sys_mlock(args[0], args[1]),
        SYSCALL_MUNLOCK => sys_munlock(args[0], args[1]),
        SYSCALL_MLOCKALL => sys_mlockall(args[0]),
        SYSCALL_MUNLOCKALL => sys_munlockall(),
        SYSCALL_MINCORE => sys_mincore(args[0], args[1], args[2] as *mut u8),
        SYSCALL_MADVISE => 0, // stub: madvise hints are advisory only
        SYSCALL_SBRK => sys_sbrk(args[0] as isize),
        SYSCALL_SPAWN => sys_spawn(args[0] as *const u8),
        SYSCALL_SET_PRIORITY => {
            sys_set_priority(args[0] as isize, args[1] as isize, args[2] as isize)
        }
        SYSCALL_GET_PRIORITY => sys_get_priority(args[0] as isize, args[1] as isize),
        SYSCALL_POLL => sys_ppoll(args[0] as *mut PollFd, args[1], args[2] as *const TimeSpec),
        SYSCALL_SHUTDOWN => sys_shutdown(),
        // ---- Network syscalls ----
        SYSCALL_SOCKET => crate::net::syscall::sys_socket(args[0], args[1], args[2]),
        SYSCALL_SOCKETPAIR => {
            crate::net::syscall::sys_socketpair(args[0], args[1], args[2], args[3] as *mut i32)
        }
        SYSCALL_BIND => crate::net::syscall::sys_bind(args[0], args[1] as *const u8, args[2]),
        SYSCALL_LISTEN => crate::net::syscall::sys_listen(args[0], args[1]),
        SYSCALL_ACCEPT => {
            crate::net::syscall::sys_accept(args[0], args[1] as *mut u8, args[2] as *mut u32, 0)
        }
        SYSCALL_ACCEPT4 => crate::net::syscall::sys_accept(
            args[0],
            args[1] as *mut u8,
            args[2] as *mut u32,
            args[3],
        ),
        SYSCALL_CONNECT => crate::net::syscall::sys_connect(args[0], args[1] as *const u8, args[2]),
        SYSCALL_GETSOCKNAME => {
            crate::net::syscall::sys_getsockname(args[0], args[1] as *mut u8, args[2] as *mut u32)
        }
        SYSCALL_GETPEERNAME => {
            crate::net::syscall::sys_getpeername(args[0], args[1] as *mut u8, args[2] as *mut u32)
        }
        SYSCALL_SENDTO => crate::net::syscall::sys_sendto(
            args[0],
            args[1] as *const u8,
            args[2],
            args[3],
            args[4] as *const u8,
            args[5],
        ),
        SYSCALL_RECVFROM => crate::net::syscall::sys_recvfrom(
            args[0],
            args[1] as *mut u8,
            args[2],
            args[3],
            args[4] as *mut u8,
            args[5] as *mut u32,
        ),
        SYSCALL_SETSOCKOPT => crate::net::syscall::sys_setsockopt(
            args[0],
            args[1],
            args[2],
            args[3] as *const u8,
            args[4],
        ),
        SYSCALL_GETSOCKOPT => crate::net::syscall::sys_getsockopt(
            args[0],
            args[1],
            args[2],
            args[3] as *mut u8,
            args[4] as *mut u32,
        ),
        SYSCALL_SHUTDOWN_SOCKET => {
            crate::net::syscall::sys_shutdown_socket(args[0], args[1] as i32)
        }
        SYSCALL_SENDMSG => crate::net::syscall::sys_sendmsg(),
        SYSCALL_RECVMSG => crate::net::syscall::sys_recvmsg(),
        SYSCALL_SCHED_SETATTR => sys_sched_setattr(args[0], args[1], args[2]),
        SYSCALL_SCHED_GETATTR => sys_sched_getattr(args[0], args[1] as *mut u8, args[2], args[3]),
        SYSCALL_GET_MEMPOLICY => sys_get_mempolicy(
            args[0] as *mut i32,
            args[1] as *mut usize,
            args[2],
            args[3],
            args[4],
        ),
        SYSCALL_MEMBARRIER => sys_membarrier(args[0] as isize, args[1] as isize),
        _ => {
            known = false;
            let name = current_process().name();
            error!(
                "{} {}: unimplemented syscall {} ({})",
                pid,
                name,
                syscall_id,
                syscall_name(syscall_id)
            );
            -ENOSYS
        }
    };
    // if pid == 4 && syscall_id == SYSCALL_EXEC {
    //     trace!("[syscall] pid=4 exec ret={}", ret);
    // }
    // // Extra verbose logging for syscall 96 (set_tid_address)
    // if syscall_id == 96 {
    //     info!("[syscall] set_tid_address returned {} to {}, ra={:#x}, sepc={:#x}",
    //         ret, name, current_trap_cx()[arch::TrapFrameArgs::RA], current_trap_cx().sepc);
    // }

    if known && trace && !(syscall_id == SYSCALL_WRITE && args[0] == 1) {
        syscall!(
            "[syscall] pid={} name={} num={}({}) args=[0x{:x},0x{:x},0x{:x},0x{:x},0x{:x},0x{:x}] ret={}",
            pid,
            name_for_trace.as_deref().unwrap_or(""),
            syscall_id,
            syscall_name(syscall_id),
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
