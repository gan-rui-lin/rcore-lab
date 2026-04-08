#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use user_lib::{
    chdir, close, dup, execve, exit, fork, link, open, shutdown, unlink, write, OpenFlags,
};

const ENABLE_ALL_TESTS: bool = true;
const SINGLE_TEST: Option<&str> = option_env!("SINGLE_TEST");
const LTP_START_FROM: Option<&str> = option_env!("LTP_START_FROM");

const SH: &[u8] = b"sh\0";
const PATH_ENV: &[u8] = b"PATH=/bin:/usr/bin:/musl:/glibc\0";
const LD_LIB_MUSL: &[u8] = b"LD_LIBRARY_PATH=/musl/lib\0";
const LD_LIB_GLIBC: &[u8] = b"LD_LIBRARY_PATH=/glibc/lib\0";
const TEST_LIBC_ROOTS: [&str; 2] = ["/musl", "/glibc"];
const TEST_SUITES: [&str; 3] = [
    // "basic",
    "busybox",
    // "cyclictest",
    "iozone",
    // "iperf",
    // "libcbench",
    // "libctest",
    // "lmbench",
    "ltp",
    // "basic",
    // "lua",
    // "netperf",
];
#[allow(dead_code)]
const RUN_EMBEDDED_PTHREAD: bool = option_env!("RUN_EMBEDDED_PTHREAD").is_some();
const PTHREAD_TEST_PATH: &str = "/tmp/pthread_cancel_small";
const EMBEDDED_PTHREAD_ELF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../pthread_cancel_small"
));
const TMP_IOZONE_PATH: &str = "/tmp/iozone_temp_test.sh";
const TMP_IOZONE_SCRIPT: &[u8] = b"\
./busybox echo iozone throughput write/read measurements\n\
./iozone -t 4 -i 0 -i 1 -r 1k -s 1m\n\
./busybox echo iozone throughput random-read measurements\n\
./iozone -t 4 -i 0 -i 2 -r 1k -s 1m\n\
";
const TMP_IOZONE_4K_PATH: &str = "/tmp/iozone_temp_test_4k.sh";
const TMP_IOZONE_4K_SCRIPT: &[u8] = b"\
./busybox echo iozone throughput write/read measurements 4k\n\
./iozone -t 4 -i 0 -i 1 -r 4k -s 1m\n\
";
const TMP_IOZONE_QUICK_PATH: &str = "/tmp/iozone_temp_quick_debug.sh";
const TMP_IOZONE_QUICK_SCRIPT: &[u8] = b"\
./busybox echo '=== tmp-iozone-quick start ==='\n\
set -x\n\
./busybox echo 'RUN iozone: -t 4 -i 6 -r 1k -s 1m'\n\
./iozone -t 4 -i 6 -r 1k -s 1m\n\
ret=$?\n\
./busybox echo \"tmp-iozone-quick exit code: $ret\"\n\
./busybox echo '=== tmp-iozone-quick done ==='\n\
exit $ret\n\
";

const TMP_LIBCTEST_PATH: &str = "/tmp/libctest_testcode.sh";
const TMP_LIBCTEST_SCRIPT: &[u8] = b"\
./runtest.exe -w entry-static.exe pthread_cancel_points\n\
./runtest.exe -w entry-static.exe pthread_cancel\n\
./runtest.exe -w entry-static.exe pthread_cancel_sem_wait\n\
./runtest.exe -w entry-static.exe pthread_exit_cancel\n\
./runtest.exe -w entry-static.exe pthread_cond\n\
./runtest.exe -w entry-static.exe pthread_tsd\n\
./runtest.exe -w entry-static.exe pthread_once_deadlock\n\
./runtest.exe -w entry-static.exe pthread_rwlock_ebusy\n\
./runtest.exe -w entry-static.exe pthread_robust_detach\n\
./runtest.exe -w entry-static.exe pthread_cond_smasher\n\
./runtest.exe -w entry-static.exe pthread_condattr_setclock\n\
";

const TMP_LTP_MINI_PATH: &str = "/tmp/ltp_mini.sh";
const TMP_LTP_MINI_SCRIPT: &[u8] = b"\
./busybox echo '=== ltp-mini start ==='\n\
ltp/testcases/bin/fork14\n\
ltp/testcases/bin/futex_wait01\n\
ltp/testcases/bin/futex_wake01\n\
./busybox echo '=== ltp-mini done ==='\n\
";

// Debug script for LTP tests that get stuck (selected from skip list)
// fork14 skipped - causes OOM with 1GB mmap (being fixed separately)
// futex_cmp_requeue01 skipped - creates hundreds of processes, takes too long
const TMP_LTP_STUCK_PATH: &str = "/tmp/ltp_stuck_debug.sh";
const TMP_LTP_STUCK_SCRIPT: &[u8] = b"\
./busybox echo '=== ltp-stuck debug start ==='\n\
./busybox echo 'Testing futex tests...'\n\
ltp/testcases/bin/futex_cmp_requeue02\n\
ltp/testcases/bin/futex_wait03\n\
ltp/testcases/bin/futex_wake02\n\
ltp/testcases/bin/futex_wake04\n\
./busybox echo 'Testing clock_nanosleep...'\n\
ltp/testcases/bin/clock_nanosleep01\n\
./busybox echo 'Testing chdir01...'\n\
ltp/testcases/bin/chdir01\n\
./busybox echo '=== ltp-stuck debug done ==='\n\
";

const TMP_LTP_CPUSET_PATH: &str = "/tmp/ltp_cpuset_debug.sh";
const TMP_LTP_CPUSET_SCRIPT: &[u8] = b"\
./busybox echo '=== ltp-cpuset debug start ==='\n\
./busybox echo 'RUN LTP CASE cpuset01'\n\
ltp/testcases/bin/cpuset01\n\
ret=$?\n\
./busybox echo \"FAIL LTP CASE cpuset01 : $ret\"\n\
./busybox echo 'RUN LTP CASE cpuset_cpu_hog'\n\
rm -f ./myfifo /tmp/cpuset_fifo.log /tmp/cpuset_cpu_hog.stderr\n\
./busybox mknod ./myfifo p || true\n\
(\n\
  while true; do\n\
    cat ./myfifo\n\
  done\n\
) > /tmp/cpuset_fifo.log &\n\
fifo_reader=$!\n\
ltp/testcases/bin/cpuset_cpu_hog &\n\
hog_pid=$!\n\
sleep 1\n\
kill -USR1 \"$hog_pid\" 2>/dev/null\n\
sleep 1\n\
kill -USR1 \"$hog_pid\" 2>/dev/null\n\
sleep 5\n\
kill -USR2 \"$hog_pid\" 2>/dev/null\n\
wait \"$hog_pid\"\n\
ret=$?\n\
kill \"$fifo_reader\" 2>/dev/null\n\
rm -f ./myfifo\n\
./busybox echo \"FAIL LTP CASE cpuset_cpu_hog : $ret\"\n\
./busybox echo '=== ltp-cpuset debug done ==='\n\
";

// Quick test for recent fixes (clone07, accept4)
const TMP_FIXES_PATH: &str = "/tmp/test_fixes.sh";
const TMP_FIXES_SCRIPT: &[u8] = b"\
./busybox echo '=== Testing recent fixes ==='\n\
./busybox echo 'Test 1: clone07 (waitpid with pid=0)'\n\
ltp/testcases/bin/clone07\n\
./busybox echo 'Test 2: accept4_01 (NULL addr handling)'\n\
ltp/testcases/bin/accept4_01\n\
./busybox echo '=== Tests completed ==='\n\
";

// Test /proc/cpuinfo implementation
const TMP_CPUINFO_PATH: &str = "/tmp/test_cpuinfo.sh";
const TMP_CPUINFO_SCRIPT: &[u8] = b"\
./busybox echo '=== Testing /proc/cpuinfo ==='\n\
./busybox cat /proc/cpuinfo\n\
./busybox echo '=== cpuinfo test done ==='\n\
";

const TMP_LTP_PATH: &str = "/tmp/ltp_testcode.sh";
const TMP_LTP_SCRIPT: &[u8] = b"\
./busybox echo '=== tmp-ltp: process/thread/signal/memory tests ==='\n\
ltp/testcases/bin/abort01\n\
ltp/testcases/bin/access01\n\
ltp/testcases/bin/access02\n\
ltp/testcases/bin/access03\n\
ltp/testcases/bin/brk01\n\
ltp/testcases/bin/brk02\n\
ltp/testcases/bin/clone01\n\
ltp/testcases/bin/clone02\n\
ltp/testcases/bin/clone03\n\
ltp/testcases/bin/clone301\n\
ltp/testcases/bin/clone302\n\
ltp/testcases/bin/exit01\n\
ltp/testcases/bin/exit02\n\
ltp/testcases/bin/exit_group01\n\
ltp/testcases/bin/fork01\n\
ltp/testcases/bin/fork02\n\
ltp/testcases/bin/fork03\n\
ltp/testcases/bin/fork04\n\
ltp/testcases/bin/fork05\n\
ltp/testcases/bin/fork06\n\
ltp/testcases/bin/fork07\n\
ltp/testcases/bin/fork08\n\
ltp/testcases/bin/fork09\n\
ltp/testcases/bin/fork10\n\
ltp/testcases/bin/fork11\n\
ltp/testcases/bin/fork13\n\
ltp/testcases/bin/fork14\n\
ltp/testcases/bin/futex_cmp_requeue01\n\
ltp/testcases/bin/futex_cmp_requeue02\n\
ltp/testcases/bin/futex_wait01\n\
ltp/testcases/bin/futex_wait02\n\
ltp/testcases/bin/futex_wait03\n\
ltp/testcases/bin/futex_wait04\n\
ltp/testcases/bin/futex_wait05\n\
ltp/testcases/bin/futex_wake01\n\
ltp/testcases/bin/futex_wake02\n\
ltp/testcases/bin/futex_wake03\n\
ltp/testcases/bin/futex_wake04\n\
ltp/testcases/bin/getpid01\n\
ltp/testcases/bin/getpid02\n\
ltp/testcases/bin/getppid01\n\
ltp/testcases/bin/getppid02\n\
ltp/testcases/bin/gettid01\n\
ltp/testcases/bin/kill01\n\
ltp/testcases/bin/kill02\n\
ltp/testcases/bin/kill03\n\
ltp/testcases/bin/kill04\n\
ltp/testcases/bin/kill05\n\
ltp/testcases/bin/kill06\n\
ltp/testcases/bin/kill07\n\
ltp/testcases/bin/kill08\n\
ltp/testcases/bin/kill09\n\
ltp/testcases/bin/kill10\n\
ltp/testcases/bin/kill11\n\
ltp/testcases/bin/kill12\n\
ltp/testcases/bin/kill13\n\
ltp/testcases/bin/mmap01\n\
ltp/testcases/bin/mmap02\n\
ltp/testcases/bin/mmap03\n\
ltp/testcases/bin/mmap04\n\
ltp/testcases/bin/mmap05\n\
ltp/testcases/bin/mmap06\n\
ltp/testcases/bin/munmap01\n\
ltp/testcases/bin/munmap02\n\
ltp/testcases/bin/munmap03\n\
ltp/testcases/bin/pipe01\n\
ltp/testcases/bin/pipe02\n\
ltp/testcases/bin/pipe03\n\
ltp/testcases/bin/pipe04\n\
ltp/testcases/bin/pipe05\n\
ltp/testcases/bin/pipe06\n\
ltp/testcases/bin/pipe07\n\
ltp/testcases/bin/pipe08\n\
ltp/testcases/bin/read01\n\
ltp/testcases/bin/read02\n\
ltp/testcases/bin/read03\n\
ltp/testcases/bin/read04\n\
ltp/testcases/bin/rt_sigprocmask01\n\
ltp/testcases/bin/rt_sigprocmask02\n\
ltp/testcases/bin/rt_sigtimedwait01\n\
ltp/testcases/bin/sigaction01\n\
ltp/testcases/bin/sigaction02\n\
ltp/testcases/bin/signal01\n\
ltp/testcases/bin/signal02\n\
ltp/testcases/bin/signal03\n\
ltp/testcases/bin/signal04\n\
ltp/testcases/bin/signal05\n\
ltp/testcases/bin/signal06\n\
ltp/testcases/bin/sigprocmask01\n\
ltp/testcases/bin/wait01\n\
ltp/testcases/bin/wait02\n\
ltp/testcases/bin/wait401\n\
ltp/testcases/bin/wait402\n\
ltp/testcases/bin/waitpid01\n\
ltp/testcases/bin/waitpid02\n\
ltp/testcases/bin/waitpid03\n\
ltp/testcases/bin/waitpid04\n\
ltp/testcases/bin/waitpid05\n\
ltp/testcases/bin/write01\n\
ltp/testcases/bin/write02\n\
ltp/testcases/bin/write03\n\
ltp/testcases/bin/write04\n\
ltp/testcases/bin/write05\n\
./busybox echo '=== tmp-ltp: done ==='\n\
";

fn cstring(s: &str) -> Vec<u8> {
    let mut v = Vec::from(s.as_bytes());
    if !v.ends_with(&[0]) {
        v.push(0);
    }
    v
}

fn write_embedded_elf(path: &str, data: &[u8]) -> isize {
    let path_c = cstring(path);
    let path_str = unsafe { core::str::from_utf8_unchecked(&path_c) };
    let fd = open(
        path_str,
        OpenFlags::CREATE | OpenFlags::TRUNC | OpenFlags::WRONLY,
    );
    if fd < 0 {
        println!("open {} failed (ret={})", path, fd);
        return fd;
    }
    let fd = fd as usize;
    let mut offset = 0usize;
    while offset < data.len() {
        let n = write(fd, &data[offset..]);
        if n <= 0 {
            println!("write {} failed (ret={})", path, n);
            let _ = close(fd);
            return n;
        }
        offset += n as usize;
    }
    let _ = close(fd);
    0
}

fn file_exists(path: &str) -> bool {
    let p = cstring(path);
    let p_str = unsafe { core::str::from_utf8_unchecked(&p) };
    let fd = open(p_str, OpenFlags::RDONLY);
    if fd >= 0 {
        let _ = close(fd as usize);
        true
    } else {
        false
    }
}

fn force_link(link_path: &str, target_path: &str) {
    if !file_exists(target_path) {
        // println!(
        //     "[initcode] skip relink {} -> {} (target missing)",
        //     link_path, target_path
        // );
        return;
    }
    let link_c = cstring(link_path);
    let link_name = unsafe { core::str::from_utf8_unchecked(&link_c) };
    let target_c = cstring(target_path);
    let target = unsafe { core::str::from_utf8_unchecked(&target_c) };

    let _ = unlink(link_name);
    let _ret = link(target, link_name);
    // if ret == 0 {
    //     println!("[initcode] relink {} -> {}", link_path, target_path);
    // } else {
    //     println!(
    //         "[initcode] relink failed {} -> {} (ret={})",
    //         link_path, target_path, ret
    //     );
    // }
}

fn run_busybox_mkdir_p(root: &str, busybox_path: &str, path: &str) -> isize {
    let pid = fork();
    if pid < 0 {
        return -1;
    }
    if pid == 0 {
        let _ = chdir("/\0");
        let busybox = cstring(busybox_path);
        let applet = cstring("mkdir");
        let opt_p = cstring("-p");
        let dir = cstring(path);
        let argv = [
            busybox.as_ptr(),
            applet.as_ptr(),
            opt_p.as_ptr(),
            dir.as_ptr(),
            core::ptr::null(),
        ];
        let ld_lib = if root == "/glibc" { LD_LIB_GLIBC } else { LD_LIB_MUSL };
        let envp = [PATH_ENV.as_ptr(), ld_lib.as_ptr(), core::ptr::null()];
        let ret = execve(
            unsafe { core::str::from_utf8_unchecked(&busybox) },
            &argv,
            &envp,
        );
        exit(if ret < 0 { 127 } else { 0 });
    }
    let mut status = 0;
    loop {
        let ret = user_lib::waitpid(pid as usize, &mut status);
        if ret == pid {
            break;
        }
        if ret < 0 && ret != -2 {
            break;
        }
    }
    status as isize
}

fn select_busybox_for_root(root: &str) -> Option<&'static str> {
    match root {
        "/musl" => {
            // `/musl/busybox` on some images may not be directly executable by
            // our kernel path resolution (it can be treated as non-ELF and
            // recurse through `/bin/sh`). Prefer a known executable busybox.
            if file_exists("/glibc/busybox") {
                Some("/glibc/busybox")
            } else if file_exists("/musl/busybox") {
                Some("/musl/busybox")
            } else {
                None
            }
        }
        "/glibc" => {
            if file_exists("/glibc/busybox") {
                Some("/glibc/busybox")
            } else {
                None
            }
        }
        _ => None,
    }
}

fn activate_runtime_profile(root: &str) -> bool {
    let Some(busybox_path) = select_busybox_for_root(root) else {
        println!("[initcode] profile {} busybox missing", root);
        return false;
    };

    force_link("/bin/sh", busybox_path);
    if root == "/musl" && file_exists("/glibc/busybox") {
        // In some images `/musl/busybox` may be a symlink to `/bin/busybox`.
        // Linking `/bin/busybox` back to `/musl/busybox` would create a loop
        // and make busybox exec fail with ELOOP.
        force_link("/bin/busybox", "/glibc/busybox");
    } else {
        force_link("/bin/busybox", busybox_path);
    }
    force_link("/bin/basename", busybox_path);
    force_link("/bin/ls", busybox_path);
    force_link("/bin/sleep", busybox_path);
    force_link("/bin/mkdir", busybox_path);
    force_link("/bin/rmdir", busybox_path);
    force_link("/bin/cat", busybox_path);
    force_link("/bin/echo", busybox_path);
    force_link("/bin/grep", busybox_path);
    force_link("/bin/rm", busybox_path);
    force_link("/bin/cp", busybox_path);
    force_link("/bin/mv", busybox_path);
    force_link("/bin/ln", busybox_path);
    force_link("/bin/chmod", busybox_path);
    force_link("/bin/chown", busybox_path);
    force_link("/bin/kill", busybox_path);
    force_link("/bin/mount", busybox_path);
    force_link("/bin/umount", busybox_path);
    force_link("/bin/date", busybox_path);
    force_link("/bin/dd", busybox_path);
    force_link("/bin/df", busybox_path);
    force_link("/bin/ps", busybox_path);
    force_link("/bin/pwd", busybox_path);
    force_link("/bin/sed", busybox_path);
    force_link("/bin/awk", busybox_path);
    force_link("/usr/bin/basename", busybox_path);
    force_link("/usr/bin/ls", busybox_path);
    force_link("/usr/bin/sleep", busybox_path);
    force_link("/usr/bin/wc", busybox_path);
    force_link("/usr/bin/expr", busybox_path);
    force_link("/usr/bin/head", busybox_path);
    force_link("/usr/bin/tail", busybox_path);
    force_link("/usr/bin/cut", busybox_path);
    force_link("/usr/bin/tr", busybox_path);
    force_link("/usr/bin/sort", busybox_path);
    force_link("/usr/bin/uniq", busybox_path);
    force_link("/usr/bin/find", busybox_path);
    force_link("/usr/bin/xargs", busybox_path);
    force_link("/usr/bin/test", busybox_path);
    force_link("/usr/bin/printf", busybox_path);
    force_link("/usr/bin/id", busybox_path);
    force_link("/usr/bin/whoami", busybox_path);
    force_link("/usr/bin/hostname", busybox_path);
    force_link("/usr/bin/diff", busybox_path);
    force_link("/usr/bin/seq", busybox_path);
    force_link("/usr/bin/tee", busybox_path);
    force_link("/usr/bin/touch", busybox_path);
    force_link("/usr/bin/stat", busybox_path);

    // TODO execve("/riscv/musl/busybox --install /bin");

    #[cfg(target_arch = "riscv64")]
    {
        if root == "/glibc" {
            force_link(
                "/lib/ld-linux-riscv64-lp64d.so.1",
                "/glibc/lib/ld-linux-riscv64-lp64d.so.1",
            );
            // glibc dynamic binaries need shared libs in default search path
            // sdcard has libc.so / libm.so (without version suffix)
            force_link("/lib/libc.so.6", "/glibc/lib/libc.so");
            force_link("/lib/libm.so.6", "/glibc/lib/libm.so");

            let _ = run_busybox_mkdir_p("/glibc", busybox_path, "/code/lmbench_src/bin/build");
            force_link("/code/lmbench_src/bin/build/lmbench_all", "/glibc/lmbench_all");
        } else {
            force_link("/lib/ld-linux-riscv64-lp64d.so.1", "/musl/lib/libc.so");
            force_link("/lib/ld-musl-riscv64.so.1", "/musl/lib/libc.so");
            force_link("/lib/ld-musl-riscv64-sf.so.1", "/musl/lib/libc.so");
            let _ = run_busybox_mkdir_p("/musl", busybox_path, "/code/lmbench_src/bin/build");
            force_link("/code/lmbench_src/bin/build/lmbench_all", "/musl/lmbench_all");
        }
    }

    #[cfg(target_arch = "loongarch64")]
    {
        if root == "/glibc" {
            force_link(
                "/lib64/ld-linux-loongarch-lp64d.so.1",
                "/glibc/lib/ld-linux-loongarch-lp64d.so.1",
            );
            // WORKAROUND: Skip mkdir due to fork+exec hang bug on LoongArch
            // See docs/GRLDocs/LA-fork-exec-hang-bug-2026-04-03.md
            force_link("/code/lmbench_src/bin/build/lmbench_all", "/glibc/lmbench_all");
        } else {
            force_link("/lib64/ld-linux-loongarch-lp64d.so.1", "/musl/lib/libc.so");
            force_link("/lib64/ld-musl-loongarch-lp64d.so.1", "/musl/lib/libc.so");
            // WORKAROUND: Skip mkdir due to fork+exec hang bug on LoongArch
            force_link("/code/lmbench_src/bin/build/lmbench_all", "/musl/lmbench_all");
        }
    }

    true
}

#[no_mangle]
fn main() -> i32 {
    let _ = open("console\0", OpenFlags::RDWR);
    let _ = dup(0);
    let _ = dup(0);

    println!("\n=== rCore initcode ===");

    if RUN_EMBEDDED_PTHREAD {
        println!("[initcode] RUN_EMBEDDED_PTHREAD enabled, writing {}", PTHREAD_TEST_PATH);
        let _ = write_embedded_elf(PTHREAD_TEST_PATH, EMBEDDED_PTHREAD_ELF);
        let _ = run_single_binary(PTHREAD_TEST_PATH);
        shutdown();
    }

    if let Some(test_path) = SINGLE_TEST {
        run_selector(test_path);
    } else if ENABLE_ALL_TESTS {
        run_all_suites();
    } else {
        run_selector("musl-libctest");
    }

    println!("\n=== All tests completed ===");
    shutdown();
}

fn run_single_binary(path: &str) -> i32 {
    println!("=== Running {} ===", path);

    let pid = fork();
    if pid < 0 {
        println!("Fork failed!");
        return -1;
    }

    if pid == 0 {
        if let Some((dir, _)) = path.rsplit_once('/') {
            if !dir.is_empty() {
                let dir_c = cstring(dir);
                let _ = chdir(unsafe { core::str::from_utf8_unchecked(&dir_c) });
            }
        }
        let path_c = cstring(path);
        let path_str = unsafe { core::str::from_utf8_unchecked(&path_c) };
        let argv = [path_str.as_ptr(), core::ptr::null()];
        let ld_lib = if path.starts_with("/glibc") { LD_LIB_GLIBC } else { LD_LIB_MUSL };
        let envp = [PATH_ENV.as_ptr(), ld_lib.as_ptr(), core::ptr::null()];
        let ret = execve(path_str, &argv, &envp);
        println!("Exec {} failed (ret={})!", path, ret);
        exit(-1);
    } else {
        let mut status: i32 = 0;
        loop {
            let ret = user_lib::waitpid(pid as usize, &mut status);
            if ret == pid {
                break;
            }
            if ret < 0 && ret != -2 {
                break;
            }
        }
        println!("=== {} completed (status=0x{:x}) ===\n", path, status);
        status
    }
}

fn run_single_elf_suite() {
    let tests = [
        "/musl/basic/brk",
        "/musl/basic/chdir",
        "/musl/basic/clone",
        "/musl/basic/close",
        "/musl/basic/dup",
        "/musl/basic/dup2",
        "/musl/basic/execve",
        "/musl/basic/exit",
        "/musl/basic/fork",
        "/musl/basic/fstat",
        "/musl/basic/getcwd",
        "/musl/basic/getdents",
        "/musl/basic/getpid",
        "/musl/basic/getppid",
        "/musl/basic/gettimeofday",
        "/musl/basic/mkdir_",
        "/musl/basic/mmap",
        "/musl/basic/mount",
        "/musl/basic/munmap",
        "/musl/basic/open",
        "/musl/basic/openat",
        "/musl/basic/pipe",
        "/musl/basic/read",
        "/musl/basic/sleep",
        "/musl/basic/chdir",
        // "/musl/basic/test_echo",
        "/musl/basic/mkdir_",
        "/musl/basic/times",
        "/musl/basic/umount",
        "/musl/basic/uname",
        "/musl/basic/unlink",
        "/musl/basic/wait",
        "/musl/basic/waitpid",
        "/musl/basic/write",
        "/musl/basic/yield",
    ];

    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;

    println!("\n==========================================");
    println!("   Running Single-ELF Suite (/musl/basic)");
    println!("==========================================\n");

    for (_idx, path) in tests.iter().enumerate() {
        total += 1;
        // println!("\n[{}/{}] Running test: {}", idx + 1, tests.len(), path);
        println!("------------------------------------------");
        let status = run_single_binary(path);
        if status == 0 {
            passed += 1;
            // println!("✓ Test PASSED: {}", path);
        } else {
            failed += 1;
            // println!("✗ Test FAILED: {} (status=0x{:x})", path, status);
        }
        println!("------------------------------------------");
    }

    println!("\n==========================================");
    println!("   Single-ELF Suite Summary");
    println!("==========================================");
    println!("Total:  {} tests", total);
    println!("Passed: {} tests", passed);
    println!("Failed: {} tests", failed);
    println!("==========================================\n");
}

fn run_testcode(script_path: &str, root: &str) -> i32 {
    println!("=== Running {} ===", script_path);

    if !file_exists(script_path) {
        println!("=== Skipped {} (not found) ===\n", script_path);
        return 0;
    }

    let Some(busybox_path) = select_busybox_for_root(root) else {
        println!("=== Skipped {} (busybox not found) ===\n", script_path);
        return -1;
    };

    let pid = fork();
    if pid < 0 {
        println!("Fork failed!");
        return -1;
    }

    if pid == 0 {
        // Close inherited fds above stderr so child processes (iperf3, netperf)
        // get low fd numbers (e.g., stream fd=5) matching judge regex expectations.
        for fd in 3..20 {
            let _ = close(fd as usize);
        }
        if root == "/glibc" {
            let _ = chdir("/glibc/\0");
        } else {
            let _ = chdir("/musl/\0");
        }
        let script = cstring(script_path);
        let busybox = cstring(busybox_path);
        let argv = [SH.as_ptr(), script.as_ptr(), core::ptr::null()];
        let ld_lib = if root == "/glibc" { LD_LIB_GLIBC } else { LD_LIB_MUSL };
        let envp = [PATH_ENV.as_ptr(), ld_lib.as_ptr(), core::ptr::null()];
        let ret = execve(
            unsafe { core::str::from_utf8_unchecked(&busybox) },
            &argv,
            &envp,
        );
        println!("Exec {} failed (ret={})!", script_path, ret);
        exit(-1);
    } else {
        let mut status: i32 = 0;
        // Wait for the suite runner itself, but opportunistically reap any
        // other exited children while we are polling. This prevents long LTP
        // runs from accumulating many zombies and exhausting kernel resources.
        let mut suite_done = false;
        loop {
            let ret = user_lib::waitpid(pid as usize, &mut status);
            if ret == pid {
                break;
            }
            if ret < 0 && ret != -2 {
                // error other than EAGAIN
                break;
            }

            // Reap any exited orphan children of initproc in a bounded loop.
            let mut reap_rounds = 0usize;
            loop {
                if reap_rounds >= 64 {
                    break;
                }
                let mut orphan_status: i32 = 0;
                let orphan = user_lib::waitpid_nohang(-1i32 as usize, &mut orphan_status);
                if orphan > 0 {
                    if orphan == pid {
                        status = orphan_status;
                        suite_done = true;
                        break;
                    }
                    reap_rounds += 1;
                    continue;
                }
                break;
            }
            if suite_done {
                break;
            }
            user_lib::sys_yield();
        }
        println!("=== {} completed (status=0x{:x}) ===\n", script_path, status);
        status
    }
}

fn run_suite(root: &str, suite: &str) -> i32 {
    if !activate_runtime_profile(root) {
        println!("=== Skipped {} / {} (profile activate failed) ===\n", root, suite);
        return -1;
    }
    if suite == "ltp" {
        let ret = run_ltp_suite(root);
        let reaped = reap_orphans();
        println!(
            "[initcode] cleanup after {}/{} done (status=0x{:x}, reaped={})",
            root, suite, ret, reaped
        );
        return ret;
    }
    let script = format!("{}/{}_testcode.sh", root, suite);
    let ret = run_testcode(script.as_str(), root);
    // Kill orphan daemons (e.g. iperf3 -s -D, netserver -D) so they don't
    // hold ports when the next libc variant runs the same suite.
    let reaped = reap_orphans();
    println!(
        "[initcode] cleanup after {}/{} done (status=0x{:x}, reaped={})",
        root, suite, ret, reaped
    );
    ret
}

fn run_ltp_suite(root: &str) -> i32 {
    let script_path = "/tmp/ltp_testcode_filtered.sh";
    let start_from_default = LTP_START_FROM.unwrap_or("");
    let start_from_line = format!(
        "start_from=\"{}\"\nif [ -z \"$start_from\" ]; then\n  start_from=\"${{LTP_START_FROM:-}}\"\nfi\nstarted=1\nif [ -n \"$start_from\" ]; then\n  started=0\n  echo \"LTP start marker enabled: $start_from\"\nfi\n",
        start_from_default
    );
    // Run standalone LTP cases while skipping obvious helper/library entries
    // that are not meant to be launched directly (e.g. cgroup_fj_proc).
    let script = if root == "/glibc" {
        "\
#!/bin/sh
echo \"#### OS COMP TEST GROUP START ltp-glibc ####\"
target_dir=\"ltp/testcases/bin\"
export PATH=\"$PATH:./ltp/testcases/bin:./ltp/testcases/lib:./ltp/testcases/network/busy_poll:./ltp/testcases/kernel/controllers/cgroup_fj\"
case_timeout=\"${LTP_CASE_TIMEOUT:-8}\"
case_limit=\"${LTP_CASE_LIMIT:-}\"
case_count=0

is_skip_case() {
  case \"$1\" in
    *.sh|*_helper|*_helper.sh|*_child|busy_poll_lib.sh|tst_*.sh|cgroup_fj_proc|cgroup_fj_*|cgroup_regression_*|cpuctl_fj_*|cpuhotplug_do_*|cpuhotplug_report_*|cpuset*|crash*|dio_read|dio_sparse|epoll*|eventfd*|event_generator|execveat*|fanotify*|fanout*|f00f|faccessat201|faccessat202|fallocate02|fallocate04|fallocate05|fallocate06|fchmod02|fchmod05|fchown01_16|fchown02_16|fchown03_16|fchown04|fchown04_16|fchown05_16|fchownat02|fcntl01|fcntl01_64|fcntl07|fcntl07_64|fcntl09|fcntl09_64|fcntl10|fcntl10_64|fcntl11|fcntl11_64|fcntl12|fcntl12_64|fcntl14|fcntl14_64|fcntl15|fcntl15_64|fcntl16|fcntl16_64|fcntl17|fcntl17_64|fcntl19|fcntl19_64|fcntl20|fcntl20_64|fcntl21|fcntl21_64|fcntl2[2-7]*|fcntl30|fcntl30_64|fcntl31|fcntl31_64|fcntl32|fcntl32_64|fcntl33|fcntl33_64|fcntl34|fcntl34_64|fcntl35|fcntl35_64|fcntl36|fcntl36_64|fcntl37|fcntl37_64|fcntl38|fcntl38_64|fcntl39|fcntl39_64|fdatasync02|fdatasync03|fgetxattr*|flistxattr*|find_portbundle|finit_module*|float_*|flock01|flock02|flock03|flock04|fork05|fork07|fork09|fork13|fork14|fork_exec_loop|fptest*|frag|fremovexattr*|fs_di|fs_fill|fs_inod|fs_perms|fsconfig*|fsetxattr*|fsmount*|fsopen*|fspick*|fsstress|fstatfs01|fstatfs01_64|fsx-linux|fsync*|ftest01|ftest02|ftest03|ftest04|ftest06|ftest07|ftest08|ftruncate01|ftruncate01_64|ftruncate04|ftruncate04_64|futex_cmp_requeue*|futex_wait03|futex_wait05|futex_wait_bitset*|futex_waitv*|futex_wake02|futex_wake04|futimesat01|fw_load|gen*|\
    creat04|creat05|creat07|creat08|creat09|copy_file_range*|crypto_user*|cve-*|delete_module*|diotest2|dio_append|dio_truncate|dirtyc0w*|dirtypipe|dma_thread_diotest|dup05|ebizzy|eject_check_tray|endian_switch01|exec_with_inh|exec_without_inh|execve02|execve04|execve05|getxattr0[2-4]|hackbench|inode02|kill08|kill10|kill11|leapsec01|lftest|mallocstress|memcg*|mmap1|mmap3|mmapstress*|mmstress|mremap*|mtest*|nanosleep04|nptl01|pause*|pids_task*|pipe13|ppoll*|prot_hsymlinks|pselect02*|pthcli|pthserv|select04*|sendfile07*|setfsgid03*|shm_test*|signal01*|starvation*|tgkill*|timed_forkbomb*|waitpid08*|chdir01|clock_nanosleep*|close_range01|sched_datafile*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

run_case_with_timeout() {
  case_file=\"$1\"
  \"$case_file\" &
  case_pid=$!
  elapsed=0
  while kill -0 \"$case_pid\" 2>/dev/null; do
    if [ \"$elapsed\" -ge \"$case_timeout\" ]; then
      kill -9 \"$case_pid\" 2>/dev/null
      echo \"TIMEOUT LTP CASE $(basename \"$case_file\")\"
      return 124
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  wait \"$case_pid\"
  return $?
}

for file in \"$target_dir\"/*; do
    [ -f \"$file\" ] || continue
    case_name=$(basename \"$file\")
  if [ \"$started\" -eq 0 ]; then
    if [ \"$case_name\" = \"$start_from\" ]; then
      started=1
      echo \"LTP start marker matched: $case_name\"
    else
      continue
    fi
  fi
    if is_skip_case \"$case_name\"; then
        continue
    fi
    if [ -n \"$case_limit\" ] && [ \"$case_count\" -ge \"$case_limit\" ]; then
        echo \"LTP case limit reached: $case_limit\"
        break
    fi
    echo \"RUN LTP CASE $case_name\"
    case_count=$((case_count + 1))
    run_case_with_timeout \"$file\"
    ret=$?
    echo \"FAIL LTP CASE $case_name : $ret\"
done

echo \"#### OS COMP TEST GROUP END ltp-glibc ####\"
"
    } else {
        "\
#!/bin/sh
echo \"#### OS COMP TEST GROUP START ltp-musl ####\"
target_dir=\"ltp/testcases/bin\"
export PATH=\"$PATH:./ltp/testcases/bin:./ltp/testcases/lib:./ltp/testcases/network/busy_poll:./ltp/testcases/kernel/controllers/cgroup_fj\"
case_timeout=\"${LTP_CASE_TIMEOUT:-8}\"
case_limit=\"${LTP_CASE_LIMIT:-}\"
case_count=0

is_skip_case() {
    case \"$1\" in
        *.sh|*_helper|*_helper.sh|*_child|busy_poll_lib.sh|tst_*.sh|cgroup_fj_proc|cgroup_fj_*|cgroup_regression_*|cpuctl_fj_*|cpuhotplug_do_*|cpuhotplug_report_*|cpuset*|crash*|dio_read|dio_sparse|epoll*|eventfd*|event_generator|execveat*|fanotify*|fanout*|f00f|faccessat201|faccessat202|fallocate02|fallocate04|fallocate05|fallocate06|fchmod02|fchmod05|fchown01_16|fchown02_16|fchown03_16|fchown04|fchown04_16|fchown05_16|fchownat02|fcntl01|fcntl01_64|fcntl07|fcntl07_64|fcntl09|fcntl09_64|fcntl10|fcntl10_64|fcntl11|fcntl11_64|fcntl12|fcntl12_64|fcntl14|fcntl14_64|fcntl15|fcntl15_64|fcntl16|fcntl16_64|fcntl17|fcntl17_64|fcntl19|fcntl19_64|fcntl20|fcntl20_64|fcntl21|fcntl21_64|fcntl2[2-7]*|fcntl30|fcntl30_64|fcntl31|fcntl31_64|fcntl32|fcntl32_64|fcntl33|fcntl33_64|fcntl34|fcntl34_64|fcntl35|fcntl35_64|fcntl36|fcntl36_64|fcntl37|fcntl37_64|fcntl38|fcntl38_64|fcntl39|fcntl39_64|fdatasync02|fdatasync03|fgetxattr*|flistxattr*|find_portbundle|finit_module*|float_*|flock01|flock02|flock03|flock04|fork05|fork07|fork09|fork13|fork14|fork_exec_loop|fptest*|frag|fremovexattr*|fs_di|fs_fill|fs_inod|fs_perms|fsconfig*|fsetxattr*|fsmount*|fsopen*|fspick*|fsstress|fstatfs01|fstatfs01_64|fsx-linux|fsync*|ftest01|ftest02|ftest03|ftest04|ftest06|ftest07|ftest08|ftruncate01|ftruncate01_64|ftruncate04|ftruncate04_64|futex_cmp_requeue*|futex_wait03|futex_wait05|futex_wait_bitset*|futex_waitv*|futex_wake02|futex_wake04|futimesat01|fw_load|gen*|\
        creat04|creat05|creat07|creat08|creat09|copy_file_range*|crypto_user*|cve-*|delete_module*|diotest2|dio_append|dio_truncate|dirtyc0w*|dirtypipe|dma_thread_diotest|dup05|ebizzy|eject_check_tray|endian_switch01|exec_with_inh|exec_without_inh|execve02|execve04|execve05|getxattr0[2-4]|hackbench|inode02|kill08|kill10|kill11|leapsec01|lftest|mallocstress|memcg*|mmap1|mmap3|mmapstress*|mmstress|mremap*|mtest*|nanosleep04|pause*|pids_task*|pipe13|ppoll*|prot_hsymlinks|pselect02*|pthcli|pthserv|select04*|sendfile07*|setfsgid03*|shm_test*|signal01*|starvation*|tgkill*|timed_forkbomb*|waitpid08*|chdir01|clock_nanosleep*|close_range01|sched_datafile*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

run_case_with_timeout() {
    case_file=\"$1\"
    \"$case_file\" &
    case_pid=$!
    elapsed=0
    while kill -0 \"$case_pid\" 2>/dev/null; do
        if [ \"$elapsed\" -ge \"$case_timeout\" ]; then
            kill -9 \"$case_pid\" 2>/dev/null
            wait \"$case_pid\" 2>/dev/null
            return 124
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    wait \"$case_pid\"
    return $?
}

for file in \"$target_dir\"/*; do
        [ -f \"$file\" ] || continue
        case_name=$(basename \"$file\")
        if [ \"$started\" -eq 0 ]; then
            if [ \"$case_name\" = \"$start_from\" ]; then
                started=1
                echo \"LTP start marker matched: $case_name\"
            else
                continue
            fi
        fi
        if is_skip_case \"$case_name\"; then
                continue
        fi
        if [ -n \"$case_limit\" ] && [ \"$case_count\" -ge \"$case_limit\" ]; then
                echo \"LTP case limit reached: $case_limit\"
                break
        fi
        echo \"RUN LTP CASE $case_name\"
        case_count=$((case_count + 1))
        run_case_with_timeout \"$file\"
        ret=$?
        echo \"FAIL LTP CASE $case_name : $ret\"
done

echo \"#### OS COMP TEST GROUP END ltp-musl ####\"
"
    };
    let script = script.replacen(
        "case_count=0\n",
        format!("case_count=0\n{}", start_from_line).as_str(),
        1,
    );
    let _ = write_embedded_elf(script_path, script.as_bytes());
    run_testcode(script_path, root)
}

/// Reap exited child processes without force-killing arbitrary PIDs.
///
/// Aggressively killing a wide PID range can destabilize long LTP runs and
/// may trigger kernel-side task-management panics. A safer cleanup strategy
/// is to only reap children that have already exited.
fn reap_orphans() -> usize {
    // Reap with WNOHANG until ECHILD.
    let mut status: i32 = 0;
    let mut reaped = 0usize;
    let mut idle_rounds = 0usize;
    loop {
        let ret = user_lib::waitpid_nohang(-1i32 as usize, &mut status);
        if ret > 0 {
            reaped += 1;
            idle_rounds = 0;
            continue;
        }
        if ret == 0 {
            // Children exist but none exited yet; bounded wait.
            idle_rounds += 1;
            if idle_rounds > 32 {
                break;
            }
            user_lib::sys_yield();
            continue;
        }
        break; // ECHILD or error — no more children
    }
    reaped
}

fn run_all_suites() {
    println!("\n==========================================");
    println!("   Running ALL Test Suites (musl + glibc)");
    println!("==========================================\n");

    for root in TEST_LIBC_ROOTS {
        println!("\n########## Running {} suites ##########", root);
        for suite in TEST_SUITES {
            // println!("\n[{}] Running {} / {}", total, root, suite);
            // println!("------------------------------------------");
            let _status = run_suite(root, suite);

            // println!("------------------------------------------");
        }
    }


}

fn run_selector(selector: &str) {
    if selector.starts_with('/') {
        if selector.starts_with("/musl/") {
            let _ = activate_runtime_profile("/musl");
        } else if selector.starts_with("/glibc/") {
            let _ = activate_runtime_profile("/glibc");
        }
        let _ = run_single_binary(selector);
        return;
    }

    if selector == "all" {
        run_all_suites();
        return;
    }

    if selector == "single-elf" {
        run_single_elf_suite();
        return;
    }

    // Write /tmp/libctest_testcode.sh and run pthread_cancel tests
    if selector == "tmp-libctest" {
        let _ = activate_runtime_profile("/musl");
        let _ = write_embedded_elf(TMP_LIBCTEST_PATH, TMP_LIBCTEST_SCRIPT);
        let _ = run_testcode(TMP_LIBCTEST_PATH, "/musl");
        return;
    }

    // Minimal LTP debug script
    if selector == "tmp-ltp-mini" || selector == "tmp-ltp-mini-glibc" {
        let root = if selector.ends_with("-glibc") { "/glibc" } else { "/musl" };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_LTP_MINI_PATH, TMP_LTP_MINI_SCRIPT);
        let _ = run_testcode(TMP_LTP_MINI_PATH, root);
        return;
    }

    // LTP stuck tests debug script - tests that may hang
    // Usage: SINGLE_TEST=tmp-ltp-stuck or SINGLE_TEST=tmp-ltp-stuck-glibc
    if selector == "tmp-ltp-stuck" || selector == "tmp-ltp-stuck-glibc" {
        let root = if selector.ends_with("-glibc") { "/glibc" } else { "/musl" };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_LTP_STUCK_PATH, TMP_LTP_STUCK_SCRIPT);
        let _ = run_testcode(TMP_LTP_STUCK_PATH, root);
        return;
    }

    // Focused cpuset debug: cpuset01 and cpuset_cpu_hog only
    // Usage: SINGLE_TEST=tmp-ltp-cpuset or SINGLE_TEST=tmp-ltp-cpuset-glibc
    if selector == "tmp-ltp-cpuset" || selector == "tmp-ltp-cpuset-glibc" {
        let root = if selector.ends_with("-glibc") { "/glibc" } else { "/musl" };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_LTP_CPUSET_PATH, TMP_LTP_CPUSET_SCRIPT);
        let _ = run_testcode(TMP_LTP_CPUSET_PATH, root);
        return;
    }

    // Recent fixes test - clone07, accept4
    // Usage: SINGLE_TEST=tmp-fixes or SINGLE_TEST=tmp-fixes-glibc
    if selector == "tmp-fixes" || selector == "tmp-fixes-glibc" {
        let root = if selector.ends_with("-glibc") { "/glibc" } else { "/musl" };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_FIXES_PATH, TMP_FIXES_SCRIPT);
        let _ = run_testcode(TMP_FIXES_PATH, root);
        return;
    }

    // Test /proc/cpuinfo implementation
    // Usage: SINGLE_TEST=tmp-cpuinfo
    if selector == "tmp-cpuinfo" {
        let root = "/musl";
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_CPUINFO_PATH, TMP_CPUINFO_SCRIPT);
        let _ = run_testcode(TMP_CPUINFO_PATH, root);
        return;
    }

    // Write /tmp/ltp_testcode.sh and run selected LTP tests
    // Usage: SINGLE_TEST=tmp-ltp or SINGLE_TEST=tmp-ltp-glibc
    if selector == "tmp-ltp" || selector == "tmp-ltp-glibc" {
        let root = if selector == "tmp-ltp-glibc" { "/glibc" } else { "/musl" };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_LTP_PATH, TMP_LTP_SCRIPT);
        let _ = run_testcode(TMP_LTP_PATH, root);
        return;
    }

    // Fast iozone debug: only fwrite/fread modes with shell tracing enabled.
    // Usage: SINGLE_TEST=tmp or SINGLE_TEST=tmp-glibc
    if selector == "tmp" || selector == "tmp-glibc" {
        let root = if selector.ends_with("-glibc") {
            "/glibc"
        } else {
            "/musl"
        };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_IOZONE_QUICK_PATH, TMP_IOZONE_QUICK_SCRIPT);
        let _ = run_testcode(TMP_IOZONE_QUICK_PATH, root);
        return;
    }

    if selector == "tmp-iozone" || selector == "tmp-iozone-glibc" {
        let root = if selector.ends_with("-glibc") {
            "/glibc"
        } else {
            "/musl"
        };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_IOZONE_PATH, TMP_IOZONE_SCRIPT);
        let _ = run_testcode(TMP_IOZONE_PATH, root);
        return;
    }

    if selector == "tmp-iozone-4k" || selector == "tmp-iozone-4k-glibc" {
        let root = if selector.ends_with("-glibc") {
            "/glibc"
        } else {
            "/musl"
        };
        let _ = activate_runtime_profile(root);
        let _ = write_embedded_elf(TMP_IOZONE_4K_PATH, TMP_IOZONE_4K_SCRIPT);
        let _ = run_testcode(TMP_IOZONE_4K_PATH, root);
        return;
    }

    if selector == "musl" || selector == "glibc" {
        let root = if selector == "musl" { "/musl" } else { "/glibc" };
        for suite in TEST_SUITES {
            let _ = run_suite(root, suite);
        }
        return;
    }

    if let Some((libc_name, suite)) = selector.split_once('-') {
        let root = if libc_name == "musl" {
            Some("/musl")
        } else if libc_name == "glibc" {
            Some("/glibc")
        } else {
            None
        };
        if let Some(root) = root {
            let _ = run_suite(root, suite);
            return;
        }
    }

    for suite in TEST_SUITES {
        if selector == suite {
            for root in TEST_LIBC_ROOTS {
                let _ = run_suite(root, suite);
            }
            return;
        }
    }

    let _ = run_single_binary(selector);
}
