#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::vec::Vec;
use user_lib::{chdir, dup, execve, exit, fork, kill, mkdir, open, shutdown, sleep, wait, waitpid, OpenFlags};

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

const BUSYBOX: &str = "/musl/busybox\0";
const SH: &[u8] = b"sh\0";
const PATH_ENV: &[u8] = b"PATH=/bin:/musl:/usr/bin\0";
#[allow(dead_code)]
const RUN_EMBEDDED_PTHREAD: bool = false;

#[cfg(feature = "embedded_pthread")]
const PTHREAD_TEST_PATH: &str = "/tmp/pthread_cancel_test";

#[cfg(feature = "embedded_pthread")]
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

#[cfg(feature = "embedded_pthread")]
use user_lib::{close, write};

#[cfg(feature = "embedded_pthread")]
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

#[no_mangle]
fn main() -> i32 {
    let _ = open("console\0", OpenFlags::RDWR);
    let _ = dup(0);
    let _ = dup(0);

    println!("\n=== rCore initcode ===");

    #[cfg(feature = "embedded_pthread")]
    if RUN_EMBEDDED_PTHREAD {
        let _ = write_embedded_elf(PTHREAD_TEST_PATH, EMBEDDED_PTHREAD_ELF);
        let _ = run_single_binary(PTHREAD_TEST_PATH);
        shutdown();
    }

    if let Some(test_path) = SINGLE_TEST {
        let _ = run_single_binary(test_path);
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

fn run_testcode(script_name: &str) {
    println!("=== Running {} ===", script_name);

    let pid = fork();
    if pid < 0 {
        println!("Fork failed!");
        return;
    }

    if pid == 0 {
        let _ = chdir("/musl/\0");
        let script = cstring(script_name);
        let argv = [SH.as_ptr(), script.as_ptr(), core::ptr::null()];
        let envp = [PATH_ENV.as_ptr(), core::ptr::null()];
        let ret = execve(BUSYBOX, &argv, &envp);
        println!("Exec {} failed (ret={})!", script_name, ret);
        exit(-1);
    } else {
        let mut status: i32 = 0;
        let _ = wait(&mut status);
        println!("=== {} completed (status=0x{:x}) ===\n", script_name, status);
    }
}

fn test_busybox() {
    run_testcode("/musl/busybox_testcode.sh");
}

fn test_lua() {
    run_testcode("/musl/lua_testcode.sh");
}

fn test_libc() {
    run_testcode("/musl/libctest_testcode.sh");
}

fn test_basic() {
    run_testcode("/musl/basic_testcode.sh");
}

/// 每个 LTP 测试的超时时间（毫秒）
const LTP_TIMEOUT_MS: usize = 30_000; // 30秒
const ENABLE_LTP_WATCHDOG: bool = false;

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
    let _ = chdir("/musl\0");

    println!("=== LTP Test Start ===");

    const LTP_TESTS_STABLE: &[&str] = &[
        // Process management
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
        // Basic I/O
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
    ];

    const LTP_TESTS_CLONE_REPRO: &[&str] = &[
        "clone01",
        "clone02",
        "clone03",
    ];

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

    println!(
        "[LTP] profile={} tests={}",
        profile_name,
        ltp_tests.len()
    );

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
            // 测试子进程
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

        // 当前这批用例是手工筛过的“稳定不过度阻塞”集合，
        // 顺序等待比额外拉一个 watchdog 更稳定。
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

        // 父进程：fork 一个 watchdog 子进程做超时
        let watchdog_pid = fork();
        if watchdog_pid == 0 {
            // watchdog 子进程：sleep 后退出
            sleep(LTP_TIMEOUT_MS);
            exit(0);
        } else if watchdog_pid < 0 {
            // watchdog fork 失败，回退到无超时等待
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

        // 父进程：等待任意一个子进程先退出
        let mut status: i32 = 0;
        let finished_pid = wait_for_ltp_child(test_pid, watchdog_pid, &mut status, name);

        if finished_pid < 0 {
            println!("[LTP] WAIT_FAIL {} (ret={})", name, finished_pid);
            let _ = kill(test_pid as usize, 9);
            let _ = kill(watchdog_pid as usize, 9);
            let mut _ts: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut _ts);
            let mut _ws: i32 = 0;
            let _ = waitpid(watchdog_pid as usize, &mut _ws);
            failed += 1;
            continue;
        }

        if finished_pid == test_pid {
            // 测试先结束，杀掉 watchdog
            let _ = kill(watchdog_pid as usize, 9);
            let mut _ws: i32 = 0;
            let _ = waitpid(watchdog_pid as usize, &mut _ws);
            if status == 0 {
                println!("[LTP] PASS {}", name);
                passed += 1;
            } else {
                println!("[LTP] FAIL {} (status=0x{:x})", name, status);
                failed += 1;
            }
        } else {
            // watchdog 先结束 = 超时，杀掉测试进程
            let _ = kill(test_pid as usize, 9);
            let mut _ts: i32 = 0;
            let _ = waitpid(test_pid as usize, &mut _ts);
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
    run_testcode("/musl/iperf_testcode.sh");
}

fn test_dynamic() {
    run_testcode("/musl/run-dynamic.sh");
}

fn test_all_tests() {
    println!("\n==========================================");
    println!("   Running ALL Test Suites");
    println!("==========================================\n");

    let test_scripts = [
        "/musl/basic_testcode.sh",
        "/musl/busybox_testcode.sh",
        "/musl/lua_testcode.sh",
        "/musl/libctest_testcode.sh",
        "/musl/iozone_testcode.sh",
        "/musl/unixbench_testcode.sh",
        "/musl/iperf_testcode.sh",
        "/musl/libcbench_testcode.sh",
        "/musl/lmbench_testcode.sh",
        "/musl/netperf_testcode.sh",
        "/musl/cyclictest_testcode.sh",
        "/musl/ltp_testcode.sh",
    ];

    let mut total = 0;
    let mut passed = 0;
    let mut failed = 0;

    for (idx, script) in test_scripts.iter().enumerate() {
        total += 1;
        println!("\n[{}/{}] Running test: {}", idx + 1, test_scripts.len(), script);
        println!("------------------------------------------");

        let pid = fork();
        if pid < 0 {
            println!("ERROR: Fork failed for {}", script);
            failed += 1;
            continue;
        }

        if pid == 0 {
            let _ = chdir("/musl/\0");
            let script_c = cstring(script);
            let argv = [SH.as_ptr(), script_c.as_ptr(), core::ptr::null()];
            let envp = [PATH_ENV.as_ptr(), core::ptr::null()];
            let ret = execve(BUSYBOX, &argv, &envp);
            println!("ERROR: Failed to exec {} (ret={})", script, ret);
            exit(-1);
        } else {
            let mut status: i32 = 0;
            let _ = wait(&mut status);
            if status == 0 {
                println!("✓ Test PASSED: {}", script);
                passed += 1;
            } else {
                println!("✗ Test FAILED: {} (status=0x{:x})", script, status);
                failed += 1;
            }
        }

        println!("------------------------------------------");
    }

    println!("\n==========================================");
    println!("   Test Suite Summary");
    println!("==========================================");
    println!("Total:  {} tests", total);
    println!("Passed: {} tests", passed);
    println!("Failed: {} tests", failed);
    println!("==========================================\n");
}

fn test_fat32_suite(){
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
        // "test_echo",
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

    for (_idx, path) in tests.iter().enumerate() {
        total += 1;
        // println!("\n[{}/{}] Running test: {}", idx + 1, tests.len(), path);
        // println!("------------------------------------------");
        let status = run_single_binary(path);
        if status == 0 {
            passed += 1;
            // println!("✓ Test PASSED: {}", path);
        } else {
            failed += 1;
            // println!("✗ Test FAILED: {} (status=0x{:x})", path, status);
        }
        // println!("------------------------------------------");
    }

    println!("\n==========================================");
    println!("   Single-ELF Suite Summary");
    println!("==========================================");
    println!("Total:  {} tests", total);
    println!("Passed: {} tests", passed);
    println!("Failed: {} tests", failed);
    println!("==========================================\n");
}
