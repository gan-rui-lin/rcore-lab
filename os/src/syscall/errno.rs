// Minimal errno values aligned with xv6-lab/src/errno.h
#![allow(dead_code)]
pub const EPERM: isize = 1;
pub const ENOENT: isize = 2;
pub const ESRCH: isize = 3;
pub const EINTR: isize = 4;
pub const EIO: isize = 5;
pub const E2BIG: isize = 7;
pub const ENOEXEC: isize = 8;
pub const EBADF: isize = 9;
pub const ECHILD: isize = 10;
pub const EAGAIN: isize = 11;
pub const ENOMEM: isize = 12;
pub const EACCES: isize = 13;
pub const EFAULT: isize = 14;
pub const EEXIST: isize = 17;
pub const ENODEV: isize = 19;
pub const ENOTDIR: isize = 20;
pub const EISDIR: isize = 21;
pub const EINVAL: isize = 22;
pub const ENFILE: isize = 23;
pub const EMFILE: isize = 24;
pub const ENOTTY: isize = 25;
pub const EPIPE: isize = 32;
pub const ERANGE: isize = 34;
pub const ENAMETOOLONG: isize = 36;
pub const ESPIPE: isize = 29;
pub const EROFS: isize = 30;
pub const ENOSYS: isize = 38;
pub const ENOTEMPTY: isize = 39;
pub const ELOOP: isize = 40;
pub const ENOMSG: isize = 42;
pub const ENOTSUP: isize = 95;
pub const ETIMEDOUT: isize = 110;

#[inline]
pub const fn errno(code: isize) -> isize {
    -code
}
