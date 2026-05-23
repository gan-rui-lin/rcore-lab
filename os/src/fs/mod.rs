//! File trait & inode(dir, file, pipe, stdin, stdout)

mod memfd;
mod pipe;
mod stdio;
pub mod timerfd;
mod vfs;

use crate::mm::UserBuffer;
#[cfg(feature = "ext4")]
use alloc::ffi::CString;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
#[cfg(feature = "ext4")]
use lwext4_rust::bindings::ext4_flink;
pub(crate) use vfs::{VfsInode, VfsMetadata, VfsStatFs};

/// trait File for all file types
pub trait File: Send + Sync {
    /// the file readable?
    fn readable(&self) -> bool;
    /// the file writable?
    fn writable(&self) -> bool;
    /// read from the file to buf, return the number of bytes read
    fn read(&self, buf: UserBuffer) -> usize;
    /// write to the file from buf, return the number of bytes written
    fn write(&self, buf: UserBuffer) -> usize;
    /// write to the file from buf, returning either bytes written or errno
    fn write_user_buffer(&self, buf: UserBuffer) -> Result<usize, isize> {
        Ok(self.write(buf))
    }
    /// read entire file content into a buffer
    fn read_all(&self) -> Vec<u8> {
        Vec::new()
    }
    /// Read file content directly into a kernel buffer at a given byte offset.
    /// Used by demand paging for file-backed mappings. Default returns 0 (unsupported).
    fn read_at_kernel(&self, _offset: usize, _buf: &mut [u8]) -> usize {
        0
    }
    /// poll the file for events, return the events bitflags
    /// default implementation returns POLLIN | POLLOUT, which means always readable and writable.
    /// ! for files that are not always ready, e.g. pipes, this should be overridden to return the actual events.
    fn poll(&self, _events: PollEvents) -> PollEvents {
        PollEvents::POLLIN | PollEvents::POLLOUT
    }
    /// Optional: underlying inode, for metadata queries.
    fn inode(&self) -> Option<Arc<dyn VfsInode>> {
        None
    }
    /// Optional: absolute path of this file.
    fn path(&self) -> Option<&str> {
        None
    }
    /// Optional: get current file offset.
    fn get_offset(&self) -> Option<usize> {
        None
    }
    /// Optional: set current file offset.
    fn set_offset(&self, _offset: usize) {}
    /// Optional: timestamp tracking id for per-file timestamp storage.
    fn ts_id(&self) -> Option<usize> {
        None
    }
    /// Optional: downcast to socket handle + type for network syscalls.
    fn as_socket(&self) -> Option<(smoltcp::iface::SocketHandle, crate::net::SocketType)> {
        None
    }
    /// Optional: get FD flags (FD_CLOEXEC). Returns 0 by default.
    fn fd_flags(&self) -> u32 {
        0
    }
    /// Optional: get file status flags (O_NONBLOCK, etc.). Returns 0 by default.
    fn status_flags(&self) -> u32 {
        0
    }
    /// Optional: set file status flags (O_NONBLOCK, O_APPEND, etc.).
    fn set_status_flags(&self, _flags: u32) {}
    /// Optional: get/add memfd seals. Regular files do not support them.
    fn get_seals(&self) -> Option<u32> { None }
    /// Optional: add memfd seals.
    fn add_seals(&self, _seals: u32) -> isize { -22 }
    /// Check if O_NONBLOCK is set.
    fn is_nonblock(&self) -> bool {
        self.status_flags() & 0x800 != 0 // O_NONBLOCK = 0x800
    }
    /// Optional: get bound port for TCP sockets.
    fn bound_port(&self) -> u16 {
        0
    }
    /// Optional: set bound port for TCP sockets.
    fn set_bound_port(&self, _port: u16) {}
    /// Optional: check if socket is listening.
    fn is_listening(&self) -> bool {
        false
    }
    /// Optional: set listening state.
    fn set_listening(&self, _listening: bool) {}
    /// Optional: mark socket handle as transferred (prevents Drop from cleaning up).
    fn mark_transferred(&self) {}
    /// Optional: set the connected remote endpoint for UDP sockets (used by connect()).
    fn set_connected_remote(&self, _addr: smoltcp::wire::IpEndpoint) {}
    /// Optional: get the connected remote endpoint for UDP sockets (used by getpeername()).
    fn get_connected_remote(&self) -> Option<smoltcp::wire::IpEndpoint> { None }
    /// Returns true if this file is an AF_UNIX domain socket.
    fn is_unix_socket(&self) -> bool { false }
    /// For AF_UNIX sockets: get address family (always 1) and socket type.
    fn unix_socket_type(&self) -> u8 { 0 }
    /// For AF_UNIX sockets: bind to a path.
    /// `path`: the sun_path bytes (null-terminated or abstract if first byte is \0).
    /// Returns 0 on success, negative errno on failure.
    fn unix_do_bind(&self, _path: alloc::string::String, _is_abstract: bool) -> isize { -88 }
    /// For AF_UNIX sockets: get the bound path (None if unbound).
    fn unix_bound_path(&self) -> Option<alloc::string::String> { None }
    /// For AF_UNIX sockets: mark as listening with given backlog.
    fn unix_do_listen(&self, _backlog: usize) -> isize { -95 } // EOPNOTSUPP
    /// For AF_UNIX sockets: accept a pending connection. Returns new socket file or None.
    fn unix_do_accept(&self) -> Option<alloc::sync::Arc<dyn File>> { None }
    /// For AF_UNIX sockets: connect to a listening socket.
    fn unix_do_connect(&self, _path: alloc::string::String, _is_abstract: bool) -> isize { -111 } // ECONNREFUSED
    /// For AF_UNIX sockets: read data, returns bytes read.
    fn unix_read(&self, _buf: &mut [u8]) -> isize { 0 }
    /// For AF_UNIX sockets: write data, returns bytes written.
    fn unix_write(&self, _buf: &[u8]) -> isize { -32 } // EPIPE
    /// For AF_UNIX sockets: peek/check if readable.
    fn unix_readable(&self) -> bool { false }
    /// For AF_UNIX sockets: poll events.
    fn unix_poll(&self, _events: PollEvents) -> PollEvents { PollEvents::empty() }
    /// For AF_UNIX sockets: push bytes into receive queue (internal use).
    fn unix_push_rx_bytes(&self, _data: &[u8]) -> usize { 0 }
    /// For AF_UNIX sockets: push an accepted socket into listen backlog (internal use).
    fn unix_push_backlog(&self, _sock: Arc<dyn File>) {}
    /// For AF_UNIX sockets: set peer socket by dynamic weak pointer (internal use).
    fn unix_set_peer_dyn(&self, _peer: Weak<dyn File>) {}
    /// For AF_UNIX sockets: mark peer as closed (internal use).
    fn unix_mark_peer_closed(&self) {}
    /// For AF_UNIX sockets: get current internal state code.
    fn unix_get_state_u8(&self) -> u8 { 0 }
    /// For AF_UNIX sockets: set peer credentials (pid, uid, gid) on server-side socket.
    fn unix_set_peer_cred(&self, _pid: u32, _uid: u32, _gid: u32) {}
    /// For AF_UNIX sockets: get peer credentials (pid, uid, gid). None if not set.
    fn unix_get_peer_cred(&self) -> Option<(u32, u32, u32)> { None }
    /// Returns true if this is a timerfd file descriptor.
    fn is_timerfd(&self) -> bool { false }
    /// For timerfd: arm the timer (expiry_us = absolute monotonic µs, interval_us = 0 for one-shot).
    fn timerfd_arm(&self, _expiry_us: u64, _interval_us: u64) {}
    /// For timerfd: disarm the timer. Returns remaining µs.
    fn timerfd_disarm(&self) -> u64 { 0 }
    /// For timerfd: get (remaining_us, interval_us). Returns None if not a timerfd.
    fn timerfd_gettime(&self) -> Option<(u64, u64)> { None }
    /// For timerfd: clockid (0=REALTIME, 1=MONOTONIC).
    fn timerfd_clockid(&self) -> i32 { -1 }
}

/// Linux-compatible stat layout (riscv64, matches musl struct stat).
#[allow(missing_docs)]
#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct Stat {
    pub dev: u64,
    pub ino: u64,
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: u64,
    pub _pad: u64,
    pub size: i64,
    pub blksize: i32,
    pub _pad2: i32,
    pub blocks: i64,
    pub atime_sec: i64,
    pub atime_nsec: i64,
    pub mtime_sec: i64,
    pub mtime_nsec: i64,
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub _unused: [u32; 2],
}

bitflags! {
    /// The mode of a inode
    /// whether a directory or a file
    pub struct StatMode: u32 {
        /// null
        const NULL  = 0;
        /// character device
        const CHR   = 0o020000;
        /// directory
        const DIR   = 0o040000;
        /// block device
        const BLK   = 0o060000;
        /// ordinary regular file
        const FILE  = 0o100000;
        /// symbolic link
        const LNK   = 0o120000;
    }
}

bitflags! {
    /// The flags argument to the open() system call.
    pub struct OpenFlags: u32 {
        /// write only
        const WRONLY = 1 << 0;
        /// read and write
        const RDWR = 1 << 1;
        /// create new file
        const CREATE = 1 << 6;
        /// truncate file size to 0
        const TRUNC = 1 << 9;
        /// append
        const APPEND = 1 << 10;
        /// direct I/O
        const DIRECT = 1 << 14;
        /// must be a directory
        const DIRECTORY = 1 << 16;
        /// close-on-exec
        const CLOEXEC = 1 << 19;
        /// path-only descriptor (Linux O_PATH on riscv64)
        const PATH = 1 << 21;
    }
}

impl OpenFlags {
    /// Return (readable, writable) tuple.
    pub fn read_write(&self) -> (bool, bool) {
        match self.bits() & 0b11 {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => (true, true),
        }
    }
}

bitflags::bitflags! {
    /// Poll events for file polling.
    pub struct PollEvents: i16 {
        /// There is data to read.
        const POLLIN = 0x001;
        /// There is urgent data to read.
        const POLLPRI = 0x002;
        ///  Writing now will not block.
        const POLLOUT = 0x004;
        /// Error condition.
        const POLLERR = 0x008;
        /// Hang up.
        const POLLHUP = 0x010;
        /// Invalid poll request.
        const POLLINVAL = 0x020;
    }
}

pub use pipe::make_pipe;
#[cfg(feature = "ext4")]
/// Mount ext4 as root with explicit device size.
pub use vfs::mount_ext4;
#[cfg(feature = "ext4")]
/// Auto-detect ext4 and mount it as root if present.
pub use vfs::mount_ext4_auto;
pub use vfs::{
    create_dir, list_apps, mount_easyfs, mount_fat32, mount_fat32_auto, mount_procfs, open_file,
    path_exists, path_is_dir, remove_path, shutdown_filesystems,
};
pub use memfd::MemFdFile;
pub use stdio::{DevNull, DevUrandom, DevZero, Stdin, Stdout};
pub use timerfd::{TimerFdFile, TIMERFD_EAGAIN};

/// Create minimal /etc and /dev files used by BusyBox tests.
pub fn ensure_basic_paths() {
    create_dir("/etc");
    create_dir("/dev");
    create_dir("/dev/misc");
    create_dir("/dev/shm");
    create_dir("/bin");
    create_dir("/usr");
    create_dir("/usr/bin");
    create_dir("/tmp");
    create_dir("/root");

    // Always overwrite passwd/group to ensure all required entries are present
    // across multiple test runs on the same sdcard image.
    write_file_overwrite(
        "/etc/passwd",
        "root:x:0:0:root:/root:/bin/sh\n\
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n\
bin:x:2:2:bin:/bin:/usr/sbin/nologin\n\
sys:x:3:3:sys:/dev:/usr/sbin/nologin\n\
nobody:x:65534:65534:nobody:/nonexistent:/bin/sh\n",
    );
    write_file_overwrite(
        "/etc/group",
        "root:x:0:\n\
daemon:x:1:\n\
bin:x:2:\n\
sys:x:3:\n\
adm:x:4:\n\
tty:x:5:\n\
disk:x:6:\n\
lp:x:7:\n\
mail:x:8:\n\
news:x:9:\n\
uucp:x:10:\n\
man:x:12:\n\
proxy:x:13:\n\
kmem:x:15:\n\
dialout:x:20:\n\
fax:x:21:\n\
voice:x:22:\n\
cdrom:x:24:\n\
floppy:x:25:\n\
tape:x:26:\n\
sudo:x:27:\n\
audio:x:29:\n\
dip:x:30:\n\
www-data:x:33:\n\
backup:x:34:\n\
operator:x:37:\n\
list:x:38:\n\
irc:x:39:\n\
src:x:40:\n\
gnats:x:41:\n\
shadow:x:42:\n\
utmp:x:43:\n\
video:x:44:\n\
sasl:x:45:\n\
plugdev:x:46:\n\
staff:x:50:\n\
games:x:60:\n\
users:x:100:\n\
",
    );
    append_line_if_missing("/etc/passwd", "nobody:x:65534:65534:nobody:/nonexistent:/bin/sh\n");
    append_line_if_missing("/etc/group", "nogroup:x:65534:\n");
    write_file_if_missing("/etc/localtime", "");
    write_file_if_missing("/etc/adjtime", "");
    // NSS configuration: ensure glibc uses "files" for all relevant databases.
    write_file_if_missing(
        "/etc/nsswitch.conf",
        "passwd:     files\n\
group:      files\n\
shadow:     files\n\
hosts:      files\n\
networks:   files\n\
protocols:  files\n\
services:   files\n\
ethers:     files\n\
rpc:        files\n\
netgroup:   files\n",
    );
    // Minimal /etc/hosts for hostname resolution.
    write_file_if_missing(
        "/etc/hosts",
        "127.0.0.1\tlocalhost\n\
::1\t\tlocalhost\n",
    );
    // Keep /etc/protocols deterministic for getprotobyname()-based tests.
    // hopopt comes BEFORE ip to ensure getprotobyname("hopopt") finds it first.
    write_file_overwrite(
        "/etc/protocols",
        "hopopt\t0\tHOPOPT\n\
ip\t0\tIP\n\
icmp\t1\tICMP\n\
tcp\t6\tTCP\n\
udp\t17\tUDP\n\
ipv6\t41\tIPv6\n\
ipv6-route\t43\tIPv6-Route\n\
ipv6-frag\t44\tIPv6-Frag\n\
esp\t50\tIPSEC-ESP\n\
ah\t51\tIPSEC-AH\n\
ipv6-icmp\t58\tIPv6-ICMP\n\
ipv6-nonxt\t59\tIPv6-NoNxt\n\
ipv6-opts\t60\tIPv6-Opts\n",
    );

    write_file_if_missing("/dev/null", "");
    write_file_if_missing("/dev/zero", "");
    write_file_if_missing("/dev/tty", "");
    write_file_if_missing("/dev/urandom", "");
    write_file_if_missing("/dev/random", "");
    write_file_if_missing("/dev/rtc", "");
    write_file_if_missing("/dev/rtc0", "");
    write_file_if_missing("/dev/misc/rtc", "");
}

/// Create a file with content if it does not exist.
fn write_file_if_missing(path: &str, content: &str) {
    if path_exists(path) {
        return;
    }
    let Some(file) = open_file(
        path,
        OpenFlags::CREATE | OpenFlags::TRUNC | OpenFlags::WRONLY,
    ) else {
        return;
    };
    let Some(inode) = file.inode() else {
        return;
    };
    let _ = inode.write_at(0, content.as_bytes());
}

fn write_file_overwrite(path: &str, content: &str) {
    let Some(file) = open_file(
        path,
        OpenFlags::CREATE | OpenFlags::TRUNC | OpenFlags::WRONLY,
    ) else {
        return;
    };
    let Some(inode) = file.inode() else {
        return;
    };
    let _ = inode.write_at(0, content.as_bytes());
}

fn append_line_if_missing(path: &str, line: &str) {
    let Some(file) = open_file(path, OpenFlags::RDWR) else {
        return;
    };
    let Some(inode) = file.inode() else {
        return;
    };
    let existing = file.read_all();
    if existing.windows(line.len()).any(|window| window == line.as_bytes()) {
        return;
    }
    let mut data = existing;
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.push(b'\n');
    }
    let offset = data.len();
    let _ = inode.write_at(offset, line.as_bytes());
}

#[cfg(feature = "ext4")]
fn ensure_hardlink(linkpath: &str, target: &str) {
    if path_exists(linkpath) {
        return;
    }
    let link_c = match CString::new(linkpath) {
        Ok(v) => v,
        Err(_) => {
            warn!("ext4: invalid link path {}", linkpath);
            return;
        }
    };
    let target_c = match CString::new(target) {
        Ok(v) => v,
        Err(_) => {
            warn!("ext4: invalid target path {}", target);
            return;
        }
    };
    let rc = unsafe { ext4_flink(target_c.as_ptr(), link_c.as_ptr()) };
    if rc == 0 {
        info!("ext4: created hardlink {} -> {}", linkpath, target);
    } else {
        warn!(
            "ext4: hardlink create failed {} -> {} rc={}",
            linkpath, target, rc
        );
    }
}

#[cfg(feature = "ext4")]
/// Create common BusyBox hardlinks on ext4 (e.g. /bin/sh) to keep scripts working.
pub fn ensure_busybox_links() {
    let has_musl_busybox = open_file("/musl/busybox", OpenFlags::empty()).is_some();
    let has_glibc_busybox = open_file("/glibc/busybox", OpenFlags::empty()).is_some();

    if !has_musl_busybox && !has_glibc_busybox {
        error!("[ext4] busybox not found at /musl/busybox and /glibc/busybox");
        return;
    }

    debug!(
        "[ext4] ensure busybox links strict mode: musl={} glibc={}",
        has_musl_busybox, has_glibc_busybox
    );

    create_dir("/bin");
    create_dir("/usr");
    create_dir("/usr/bin");
    create_dir("/lib");
    #[cfg(target_arch = "loongarch64")]
    create_dir("/lib64");

    if has_musl_busybox {
        ensure_hardlink("/musl/basename", "/musl/busybox");
        ensure_hardlink("/musl/sleep", "/musl/busybox");
    }

    if has_glibc_busybox {
        ensure_hardlink("/glibc/basename", "/glibc/busybox");
        ensure_hardlink("/glibc/sleep", "/glibc/busybox");
    }

    if open_file("/musl/lib/libc.so", OpenFlags::empty()).is_some() {
        ensure_hardlink("/lib/ld-musl-riscv64-sf.so.1", "/musl/lib/libc.so");
        ensure_hardlink("/lib/ld-musl-riscv64.so.1", "/musl/lib/libc.so");
        #[cfg(target_arch = "loongarch64")]
        {
            ensure_hardlink("/lib64/ld-musl-loongarch-lp64d.so.1", "/musl/lib/libc.so");
        }
    }

    #[cfg(target_arch = "riscv64")]
    if open_file("/glibc/lib/ld-linux-riscv64-lp64d.so.1", OpenFlags::empty()).is_some() {
        ensure_hardlink("/lib/ld-linux-riscv64-lp64d.so.1", "/glibc/lib/ld-linux-riscv64-lp64d.so.1");
        // glibc ld.so searches /lib/ for shared libraries; create hardlinks so it finds them
        // RV sdcard may have libc.so (no .6 suffix) or libc.so.6 — handle both
        for (soname, candidates) in [
            ("libc.so.6", &["/glibc/lib/libc.so.6", "/glibc/lib/libc.so"][..]),
            ("libm.so.6", &["/glibc/lib/libm.so.6", "/glibc/lib/libm.so"][..]),
        ] {
            for src in candidates {
                if open_file(src, OpenFlags::empty()).is_some() {
                    let dst = alloc::format!("/lib/{}", soname);
                    ensure_hardlink(&dst, src);
                    break;
                }
            }
        }
    }

    #[cfg(target_arch = "loongarch64")]
    if open_file("/glibc/lib/ld-linux-loongarch-lp64d.so.1", OpenFlags::empty()).is_some() {
        ensure_hardlink("/lib64/ld-linux-loongarch-lp64d.so.1", "/glibc/lib/ld-linux-loongarch-lp64d.so.1");
        // glibc ld.so searches /lib64/ for shared libraries; create hardlinks so it finds them
        if open_file("/glibc/lib/libc.so.6", OpenFlags::empty()).is_some() {
            ensure_hardlink("/lib64/libc.so.6", "/glibc/lib/libc.so.6");
        }
        if open_file("/glibc/lib/libm.so.6", OpenFlags::empty()).is_some() {
            ensure_hardlink("/lib64/libm.so.6", "/glibc/lib/libm.so.6");
        }
    }

    if open_file("/bin/sh", OpenFlags::empty()).is_none() {
        if has_musl_busybox {
            ensure_hardlink("/bin/sh", "/musl/busybox");
            ensure_hardlink("/bin/basename", "/musl/busybox");
            ensure_hardlink("/bin/ls", "/musl/busybox");
            ensure_hardlink("/bin/sleep", "/musl/busybox");
            ensure_hardlink("/usr/bin/basename", "/musl/busybox");
            ensure_hardlink("/usr/bin/ls", "/musl/busybox");
            ensure_hardlink("/usr/bin/sleep", "/musl/busybox");
        } else if has_glibc_busybox {
            ensure_hardlink("/bin/sh", "/glibc/busybox");
            ensure_hardlink("/bin/basename", "/glibc/busybox");
            ensure_hardlink("/bin/ls", "/glibc/busybox");
            ensure_hardlink("/bin/sleep", "/glibc/busybox");
            ensure_hardlink("/usr/bin/basename", "/glibc/busybox");
            ensure_hardlink("/usr/bin/ls", "/glibc/busybox");
            ensure_hardlink("/usr/bin/sleep", "/glibc/busybox");
        }
    }

    if open_file("/bin/sh", OpenFlags::empty()).is_some() {
        debug!("[ext4] /bin/sh ready");
    } else {
        debug!("[ext4] /bin/sh not created (both musl and glibc busybox present, deferred to initcode)");
    }
}
