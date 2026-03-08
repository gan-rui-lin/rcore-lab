#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::vec::Vec;
use user_lib::{chdir, dup, execve, exit, fork, open, shutdown, wait, OpenFlags};

const ENABLE_SINGLE_ELF_SUITE: bool = false;
const ENABLE_BASIC_TEST: bool = false;
const ENABLE_BUSYBOX_TEST: bool = false;
const ENABLE_LUA_TEST: bool = false;
const ENABLE_LIBC_TEST: bool = false;
const ENABLE_DYNAMIC_TEST: bool = false;
const ENABLE_LTP_TEST: bool = false;
const ENABLE_IPERF_TEST: bool = true;
const ENABLE_ALL_TESTS: bool = false;
const ENABLE_FAT32_TESTS: bool = false;
const SINGLE_TEST: Option<&str> = option_env!("SINGLE_TEST");

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

fn test_ltp() {
    run_testcode("/musl/ltp_testcode.sh");
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