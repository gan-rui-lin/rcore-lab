#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::format;
use alloc::vec::Vec;
use user_lib::{
    chdir, close, dup, execve, exit, fork, kill, link, mkdir, open, shutdown, sleep, unlink, wait,
    waitpid, write, OpenFlags,
};

const ENABLE_SINGLE_ELF_SUITE: bool = false;
const ENABLE_BASIC_TEST: bool = false;
const ENABLE_BUSYBOX_TEST: bool = false;
const ENABLE_LUA_TEST: bool = false;
const ENABLE_LIBC_TEST: bool = false;
const ENABLE_DYNAMIC_TEST: bool = false;
const ENABLE_LTP_TEST: bool = true;
const ENABLE_IPERF_TEST: bool = false;
const ENABLE_ALL_TESTS: bool = false;
const ENABLE_FAT32_TESTS: bool = false;

const SINGLE_TEST: Option<&str> = option_env!("SINGLE_TEST");
const LTP_PROFILE: Option<&str> = option_env!("LTP_PROFILE");

const SH: &[u8] = b"sh\0";
const PATH_ENV: &[u8] = b"PATH=/bin:/usr/bin:/musl:/glibc\0";
const TEST_LIBC_ROOTS: [&str; 2] = ["/musl", "/glibc"];
const TEST_SUITES: [&str; 1] = ["ltp"];
#[allow(dead_code)]
const RUN_EMBEDDED_PTHREAD: bool = option_env!("RUN_EMBEDDED_PTHREAD").is_some();
const PTHREAD_TEST_PATH: &str = "/tmp/pthread_cancel_small";
const EMBEDDED_PTHREAD_ELF: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../pthread_cancel_small"
));

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
        return;
    }
    let link_c = cstring(link_path);
    let link_name = unsafe { core::str::from_utf8_unchecked(&link_c) };
    let target_c = cstring(target_path);
    let target = unsafe { core::str::from_utf8_unchecked(&target_c) };
    let _ = unlink(link_name);
    let _ = link(target, link_name);
}

fn select_busybox_for_root(root: &str) -> Option<&'static str> {
    match root {
        "/musl" if file_exists("/musl/busybox") => Some("/musl/busybox"),
        "/glibc" if file_exists("/glibc/busybox") => Some("/glibc/busybox"),
        _ => None,
    }
}

fn activate_runtime_profile(root: &str) -> bool {
    let Some(busybox_path) = select_busybox_for_root(root) else {
        println!("[initcode] profile {} busybox missing", root);
        return false;
    };

    force_link("/bin/sh", busybox_path);
    force_link("/bin/basename", busybox_path);
    force_link("/bin/ls", busybox_path);
    force_link("/bin/sleep", busybox_path);
    force_link("/usr/bin/basename", busybox_path);
    force_link("/usr/bin/ls", busybox_path);
    force_link("/usr/bin/sleep", busybox_path);

    #[cfg(target_arch = "riscv64")]
    {
        if root == "/glibc" {
            force_link(
                "/lib/ld-linux-riscv64-lp64d.so.1",
                "/glibc/lib/ld-linux-riscv64-lp64d.so.1",
            );
        } else {
            force_link("/lib/ld-linux-riscv64-lp64d.so.1", "/musl/lib/libc.so");
            force_link("/lib/ld-musl-riscv64.so.1", "/musl/lib/libc.so");
            force_link("/lib/ld-musl-riscv64-sf.so.1", "/musl/lib/libc.so");
        }
    }

    #[cfg(target_arch = "loongarch64")]
    {
        if root == "/glibc" {
            force_link(
                "/lib64/ld-linux-loongarch-lp64d.so.1",
                "/glibc/lib/ld-linux-loongarch-lp64d.so.1",
            );
        } else {
            force_link("/lib64/ld-linux-loongarch-lp64d.so.1", "/musl/lib/libc.so");
            force_link("/lib64/ld-musl-loongarch-lp64d.so.1", "/musl/lib/libc.so");
        }
    }

    true
}

fn runtime_root_for_path(path: &str) -> Option<&'static str> {
    if path.starts_with("/musl/") {
        Some("/musl")
    } else if path.starts_with("/glibc/") {
        Some("/glibc")
    } else {
        None
    }
}

#[no_mangle]
fn main() -> i32 {
    let _ = open("console\0", OpenFlags::RDWR);
    let _ = dup(0);
    let _ = dup(0);

    println!("\n=== rCore initcode ===");

    if RUN_EMBEDDED_PTHREAD {
        println!(
            "[initcode] RUN_EMBEDDED_PTHREAD enabled, writing {}",
            PTHREAD_TEST_PATH
        );
        let _ = write_embedded_elf(PTHREAD_TEST_PATH, EMBEDDED_PTHREAD_ELF);
        let _ = run_single_binary(PTHREAD_TEST_PATH);
        shutdown();
    }

    if let Some(test_path) = SINGLE_TEST {
        run_selector(test_path);
    } else if ENABLE_ALL_TESTS {
        test_all_tests();
    } else if ENABLE_SINGLE_ELF_SUITE {
        run_single_elf_suite();
    } else {
        if ENABLE_DYNAMIC_TEST {
            test_dynamic();
        }
        if ENABLE_BASIC_TEST {
            test_basic();
        }
        if ENABLE_BUSYBOX_TEST {
            test_busybox();
        }
        if ENABLE_LUA_TEST {
            test_lua();
        }
        if ENABLE_LIBC_TEST {
            test_libc();
        }
        if ENABLE_LTP_TEST {
            test_ltp();
        }
        if ENABLE_IPERF_TEST {
            test_iperf();
        }
        if ENABLE_FAT32_TESTS {
            test_fat32_suite();
        }
    }

    println!("\n=== All tests completed ===");
    shutdown();
}

fn run_single_binary(path: &str) -> i32 {
    println!("=== Running {} ===", path);

    if let Some(root) = runtime_root_for_path(path) {
        let _ = activate_runtime_profile(root);
    }

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
        let envp = [PATH_ENV.as_ptr(), core::ptr::null()];
        let ret = execve(path_str, &argv, &envp);
        println!("Exec {} failed (ret={})!", path, ret);
        exit(-1);
    } else {
        let mut status: i32 = 0;
        let _ = wait(&mut status);
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

    for path in tests.iter() {
        total += 1;
        println!("------------------------------------------");
        let status = run_single_binary(path);
        if status == 0 {
            passed += 1;
        } else {
            failed += 1;
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
        if root == "/glibc" {
            let _ = chdir("/glibc/\0");
        } else {
            let _ = chdir("/musl/\0");
        }
        let script = cstring(script_path);
        let busybox = cstring(busybox_path);
        let argv = [SH.as_ptr(), script.as_ptr(), core::ptr::null()];
        let envp = [PATH_ENV.as_ptr(), core::ptr::null()];
        let ret = execve(
            unsafe { core::str::from_utf8_unchecked(&busybox) },
            &argv,
            &envp,
        );
        println!("Exec {} failed (ret={})!", script_path, ret);
        exit(-1);
    } else {
        let mut status: i32 = 0;
        let _ = wait(&mut status);
        println!(
            "=== {} completed (status=0x{:x}) ===\n",
            script_path, status
        );
        status
    }
}

fn run_suite(root: &str, suite: &str) -> i32 {
    if !activate_runtime_profile(root) {
        println!(
            "=== Skipped {} / {} (profile activate failed) ===\n",
            root, suite
        );
        return -1;
    }
    let script = format!("{}/{}_testcode.sh", root, suite);
    run_testcode(script.as_str(), root)
}

fn run_all_suites() {
    println!("\n==========================================");
    println!("   Running ALL Test Suites (musl + glibc)");
    println!("==========================================\n");

    for root in TEST_LIBC_ROOTS {
        println!("\n########## Running {} suites ##########", root);
        for suite in TEST_SUITES {
            let _ = run_suite(root, suite);
        }
    }
}

fn run_musl_script(script_path: &str) -> i32 {
    if !activate_runtime_profile("/musl") {
        return -1;
    }
    run_testcode(script_path, "/musl")
}

fn test_busybox() {
    let _ = run_musl_script("/musl/busybox_testcode.sh");
}

fn test_lua() {
    let _ = run_musl_script("/musl/lua_testcode.sh");
}

fn test_libc() {
    let _ = run_musl_script("/musl/libctest_testcode.sh");
}

fn test_basic() {
    let _ = run_musl_script("/musl/basic_testcode.sh");
}

const LTP_TIMEOUT_MS: usize = 30_000;
const ENABLE_LTP_WATCHDOG: bool = true;

fn wait_for_ltp_child(test_pid: isize, watchdog_pid: isize, status: &mut i32, name: &str) -> isize {
    loop {
        *status = 0;
        let finished_pid = wait(status);
        if finished_pid == test_pid || finished_pid == watchdog_pid {
            return finished_pid;
        }
        if finished_pid < 0 {
            return finished_pid;
        }
        println!(
            "[LTP] REAP {} orphan pid={} (status=0x{:x})",
            name, finished_pid, *status
        );
    }
}

fn test_ltp() {
    const LTP_BIN_PREFIX: &[u8] = b"/musl/ltp/testcases/bin/";
    const LTP_PATH_MAX: usize = 128;

    let _ = mkdir("/tmp\0");
    let _ = mkdir("/dev\0");
    let _ = mkdir("/dev/shm\0");
    let _ = activate_runtime_profile("/musl");
    let _ = chdir("/musl\0");

    println!("=== LTP Test Start ===");

    const LTP_TESTS_STABLE: &[&str] = &[
        "getpid02",
        "fork01",
        "fork03",
        "wait01",
        "wait02",
        "wait401",
        "waitpid01",
        "waitpid03",
        "clone01",
        "clone02",
        "clone03",
        "pipe01",
        "read01",
        "read02",
        "read04",
        "write01",
        "write02",
        "write03",
        "write05",
        "close01",
        "close02",
        "dup01",
        "dup02",
        "dup201",
        "dup202",
        "dup203",
        "open01",
        "lseek01",
        "mmap01",
        "munmap01",
        "mprotect01",
        "mprotect02",
        "mprotect03",
    ];

    const LTP_TESTS_CLONE_REPRO: &[&str] = &["clone01", "clone02", "clone03"];
    const LTP_TESTS_BATCH_REPRO: &[&str] = &[
        "waitpid01",
        "waitpid03",
        "clone01",
        "clone02",
        "clone03",
        "read04",
        "write05",
        "dup02",
    ];

    let (profile_name, ltp_tests) = match LTP_PROFILE {
        Some("clone-repro") => ("clone-repro", LTP_TESTS_CLONE_REPRO),
        Some("batch-repro") => ("batch-repro", LTP_TESTS_BATCH_REPRO),
        Some(other) => {
            println!("[LTP] Unknown profile {}, fallback to stable", other);
            ("stable", LTP_TESTS_STABLE)
        }
        None => ("stable", LTP_TESTS_STABLE),
    };

    println!("[LTP] profile={} tests={}", profile_name, ltp_tests.len());

    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;
    let mut timed_out = 0;

    for name in ltp_tests {
        total += 1;
        let name_bytes = name.as_bytes();
        let path_len = LTP_BIN_PREFIX.len() + name_bytes.len();
        if path_len + 1 > LTP_PATH_MAX || name_bytes.len() + 1 > LTP_PATH_MAX {
            println!("[LTP] SKIP {} (path too long)", name);
            failed += 1;
            continue;
        }

        let mut path_buf = [0u8; LTP_PATH_MAX];
        path_buf[..LTP_BIN_PREFIX.len()].copy_from_slice(LTP_BIN_PREFIX);
        path_buf[LTP_BIN_PREFIX.len()..path_len].copy_from_slice(name_bytes);
        let path_str = unsafe { core::str::from_utf8_unchecked(&path_buf[..path_len + 1]) };

        let mut name_buf = [0u8; LTP_PATH_MAX];
        name_buf[..name_bytes.len()].copy_from_slice(name_bytes);

        println!("[LTP] RUN  {}", name);

        let test_pid = fork();
        if test_pid == 0 {
            let argv = [name_buf.as_ptr(), core::ptr::null()];
            let envp_path = b"PATH=/musl:/bin:/usr/bin\0";
            let envp_tmpdir = b"TMPDIR=/tmp\0";
            let envp_home = b"HOME=/tmp\0";
            let envp_ltproot = b"LTPROOT=/musl/ltp\0";
            let envp = [
                envp_path.as_ptr(),
                envp_tmpdir.as_ptr(),
                envp_home.as_ptr(),
                envp_ltproot.as_ptr(),
                core::ptr::null(),
            ];
            let _ = execve(path_str, &argv, &envp);
            println!("[LTP] EXEC_FAIL {}", name);
            exit(-1);
        } else if test_pid < 0 {
            println!("[LTP] FORK_FAIL {}", name);
            failed += 1;
            continue;
        }

        if !ENABLE_LTP_WATCHDOG {
            let mut status: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut status);
            if status == 0 {
                println!("[LTP] PASS {}", name);
                passed += 1;
            } else {
                println!("[LTP] FAIL {} (status=0x{:x})", name, status);
                failed += 1;
            }
            continue;
        }

        let watchdog_pid = fork();
        if watchdog_pid == 0 {
            sleep(LTP_TIMEOUT_MS);
            exit(0);
        } else if watchdog_pid < 0 {
            let mut status: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut status);
            if status == 0 {
                println!("[LTP] PASS {}", name);
                passed += 1;
            } else {
                println!("[LTP] FAIL {} (status=0x{:x})", name, status);
                failed += 1;
            }
            continue;
        }

        let mut status: i32 = 0;
        let finished_pid = wait_for_ltp_child(test_pid, watchdog_pid, &mut status, name);

        if finished_pid < 0 {
            println!("[LTP] WAIT_FAIL {} (ret={})", name, finished_pid);
            let _ = kill(test_pid as usize, 9);
            let _ = kill(watchdog_pid as usize, 9);
            let mut test_status: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut test_status);
            let mut watchdog_status: i32 = 0;
            let _ = waitpid(watchdog_pid as usize, &mut watchdog_status);
            failed += 1;
            continue;
        }

        if finished_pid == test_pid {
            let _ = kill(watchdog_pid as usize, 9);
            let mut watchdog_status: i32 = 0;
            let _ = waitpid(watchdog_pid as usize, &mut watchdog_status);
            if status == 0 {
                println!("[LTP] PASS {}", name);
                passed += 1;
            } else {
                println!("[LTP] FAIL {} (status=0x{:x})", name, status);
                failed += 1;
            }
        } else {
            let _ = kill(test_pid as usize, 9);
            let mut test_status: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut test_status);
            println!("[LTP] TIMEOUT {} (>{}ms)", name, LTP_TIMEOUT_MS);
            timed_out += 1;
            failed += 1;
        }
    }

    println!("=== LTP Test End ===");
    println!(
        "Total: {}, Passed: {}, Failed: {} (Timeout: {})",
        total, passed, failed, timed_out
    );
}

fn test_iperf() {
    let _ = run_musl_script("/musl/iperf_testcode.sh");
}

fn test_dynamic() {
    let _ = run_musl_script("/musl/run-dynamic.sh");
}

fn test_all_tests() {
    run_all_suites();
}

fn run_selector(selector: &str) {
    match selector {
        "all" => {
            run_all_suites();
            return;
        }
        "single-elf" => {
            run_single_elf_suite();
            return;
        }
        "ltp" | "musl-ltp" => {
            test_ltp();
            return;
        }
        "musl" | "glibc" => {
            let root = if selector == "musl" {
                "/musl"
            } else {
                "/glibc"
            };
            for suite in TEST_SUITES {
                let _ = run_suite(root, suite);
            }
            return;
        }
        _ => {}
    }

    if selector.starts_with('/') {
        let _ = run_single_binary(selector);
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
            if suite == "ltp" && root == "/musl" {
                test_ltp();
            } else {
                let _ = run_suite(root, suite);
            }
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

fn test_fat32_suite() {
    let tests = [
        "brk",
        "chdir",
        "clone",
        "close",
        "dup",
        "dup2",
        "execve",
        "exit",
        "fork",
        "fstat",
        "getcwd",
        "getdents",
        "getpid",
        "getppid",
        "gettimeofday",
        "mkdir_",
        "mmap",
        "mount",
        "munmap",
        "open",
        "openat",
        "pipe",
        "read",
        "sleep",
        "chdir",
        "mkdir_",
        "times",
        "umount",
        "uname",
        "unlink",
        "wait",
        "waitpid",
        "write",
        "yield",
    ];

    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;

    println!("\n==========================================");
    println!("   Running Single-ELF Suite (/musl/basic)");
    println!("==========================================\n");

    for path in tests.iter() {
        total += 1;
        let status = run_single_binary(path);
        if status == 0 {
            passed += 1;
        } else {
            failed += 1;
        }
    }

    println!("\n==========================================");
    println!("   Single-ELF Suite Summary");
    println!("==========================================");
    println!("Total:  {} tests", total);
    println!("Passed: {} tests", passed);
    println!("Failed: {} tests", failed);
    println!("==========================================\n");
}
