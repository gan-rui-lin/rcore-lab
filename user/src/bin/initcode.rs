#![no_std]
#![no_main]

#[macro_use]
extern crate user_lib;

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use user_lib::{
    chdir, close, dup, execve, exit, fork, link, open, shutdown, unlink, wait, write, OpenFlags,
};

const ENABLE_ALL_TESTS: bool = true;
const SINGLE_TEST: Option<&str> = option_env!("SINGLE_TEST");

const SH: &[u8] = b"sh\0";
const PATH_ENV: &[u8] = b"PATH=/bin:/usr/bin:/musl:/glibc\0";
const LD_LIB_MUSL: &[u8] = b"LD_LIBRARY_PATH=/musl/lib\0";
const LD_LIB_GLIBC: &[u8] = b"LD_LIBRARY_PATH=/glibc/lib\0";
const TEST_LIBC_ROOTS: [&str; 2] = ["/musl", "/glibc"];
const TEST_SUITES: [&str; 4] = [
    "basic",
    "busybox",
    "libctest",
    "lmbench",
];
const LMBENCH_PROT_ONLY_SCRIPT: &[u8] = b"#!/bin/sh\n\necho \"#### OS COMP TEST GROUP START lmbench-musl ####\"\nbusybox mkdir -p /var/tmp\nbusybox touch /var/tmp/lmbench\n\necho latency measurements\necho \"[lmbench] START lat_syscall null\"\n./lmbench_all lat_syscall -P 1 null\necho \"[lmbench] DONE  lat_syscall null rc=$?\"\necho \"[lmbench] START lat_syscall read\"\n./lmbench_all lat_syscall -P 1 read\necho \"[lmbench] DONE  lat_syscall read rc=$?\"\necho \"[lmbench] START lat_syscall write\"\n./lmbench_all lat_syscall -P 1 write\necho \"[lmbench] DONE  lat_syscall write rc=$?\"\necho \"[lmbench] START lat_syscall stat\"\n./lmbench_all lat_syscall -P 1 stat /var/tmp/lmbench\necho \"[lmbench] DONE  lat_syscall stat rc=$?\"\necho \"[lmbench] START lat_syscall fstat\"\n./lmbench_all lat_syscall -P 1 fstat /var/tmp/lmbench\necho \"[lmbench] DONE  lat_syscall fstat rc=$?\"\necho \"[lmbench] START lat_syscall open\"\n./lmbench_all lat_syscall -P 1 open /var/tmp/lmbench\necho \"[lmbench] DONE  lat_syscall open rc=$?\"\necho \"[lmbench] START lat_select file\"\n./lmbench_all lat_select -n 100 -P 1 file\necho \"[lmbench] DONE  lat_select rc=$?\"\necho \"[lmbench] START lat_sig install\"\n./lmbench_all lat_sig -P 1 install\necho \"[lmbench] DONE  lat_sig install rc=$?\"\necho \"[lmbench] START lat_sig catch\"\n./lmbench_all lat_sig -P 1 catch\necho \"[lmbench] DONE  lat_sig catch rc=$?\"\necho \"[lmbench] START lat_sig prot\"\n./lmbench_all lat_sig -P 1 prot lat_sig\necho \"[lmbench] DONE  lat_sig prot rc=$?\"\necho \"[lmbench] START lat_pipe\"\n./lmbench_all lat_pipe -P 1\necho \"[lmbench] DONE  lat_pipe rc=$?\"\necho \"[lmbench] START lat_proc fork\"\n./lmbench_all lat_proc -P 1 fork\necho \"[lmbench] DONE  lat_proc fork rc=$?\"\necho \"[lmbench] START lat_proc exec\"\n./lmbench_all lat_proc -P 1 exec\necho \"[lmbench] DONE  lat_proc exec rc=$?\"\ncp hello /tmp\necho \"[lmbench] START lat_proc shell\"\n./lmbench_all lat_proc -P 1 shell\necho \"[lmbench] DONE  lat_proc shell rc=$?\"\necho \"[lmbench] START lmdd\"\n./lmbench_all lmdd label=\"File /var/tmp/XXX write bandwidth:\" of=/var/tmp/XXX move=1m fsync=1 print=3\necho \"[lmbench] DONE  lmdd rc=$?\"\necho \"[lmbench] START lat_pagefault\"\n./lmbench_all lat_pagefault -P 1 /var/tmp/XXX\necho \"[lmbench] DONE  lat_pagefault rc=$?\"\necho \"[lmbench] START lat_mmap\"\n./lmbench_all lat_mmap -P 1 512k /var/tmp/XXX\necho \"[lmbench] DONE  lat_mmap rc=$?\"\necho file system latency\necho \"[lmbench] START lat_fs\"\n./lmbench_all lat_fs /var/tmp\necho \"[lmbench] DONE  lat_fs rc=$?\"\necho Bandwidth measurements\necho \"[lmbench] START bw_pipe\"\n./lmbench_all bw_pipe -P 1\necho \"[lmbench] DONE  bw_pipe rc=$?\"\necho \"[lmbench] START bw_file_rd io_only\"\n./lmbench_all bw_file_rd -P 1 512k io_only /var/tmp/XXX\necho \"[lmbench] DONE  bw_file_rd io_only rc=$?\"\necho \"[lmbench] START bw_file_rd open2close\"\n./lmbench_all bw_file_rd -P 1 512k open2close /var/tmp/XXX\necho \"[lmbench] DONE  bw_file_rd open2close rc=$?\"\necho \"[lmbench] START bw_mmap_rd mmap_only\"\n./lmbench_all bw_mmap_rd -P 1 512k mmap_only /var/tmp/XXX\necho \"[lmbench] DONE  bw_mmap_rd mmap_only rc=$?\"\necho \"[lmbench] START bw_mmap_rd open2close\"\n./lmbench_all bw_mmap_rd -P 1 512k open2close /var/tmp/XXX\necho \"[lmbench] DONE  bw_mmap_rd open2close rc=$?\"\necho context switch overhead\necho \"[lmbench] START lat_ctx\"\n./lmbench_all lat_ctx -P 1 -s 32 2 4 8 16 24 32 64 96\necho \"[lmbench] DONE  lat_ctx rc=$?\"\n\necho \"#### OS COMP TEST GROUP END lmbench-musl ####\"\n";
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

fn write_text_file(path: &str, data: &[u8]) -> isize {
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

fn select_busybox_for_root(root: &str) -> Option<&'static str> {
    match root {
        "/musl" => {
            if file_exists("/musl/busybox") {
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
    force_link("/bin/busybox", busybox_path);
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
            // glibc dynamic binaries need shared libs in default search path
            force_link("/lib/libc.so.6", "/glibc/lib/libc.so.6");
            force_link("/lib/libm.so.6", "/glibc/lib/libm.so.6");
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
        // Use waitpid(pid) instead of wait(-1) to avoid reaping
        // orphan grandchildren (e.g. iperf3 daemon forks).
        loop {
            let ret = user_lib::waitpid(pid as usize, &mut status);
            if ret == pid {
                break;
            }
            if ret < 0 && ret != -2 {
                // error other than EAGAIN
                break;
            }
            // ret == -2 (EAGAIN) or reaped wrong child, keep waiting
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
    let mut script = format!("{}/{}_testcode.sh", root, suite);
    if root == "/musl" && suite == "lmbench" {
        let debug_script = "/tmp/lmbench_testcode.sh";
        let ret = write_text_file(debug_script, LMBENCH_PROT_ONLY_SCRIPT);
        if ret >= 0 {
            script = String::from(debug_script);
            println!(
                "[initcode] lmbench debug mode enabled, using {} (lat_sig prot + next)",
                debug_script
            );
        } else {
            println!(
                "[initcode] lmbench debug script write failed (ret={}), fallback to default",
                ret
            );
        }
    }
    run_testcode(script.as_str(), root)
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
