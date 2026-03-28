# netperf-glibc 全测序列失败修复

日期: 2026/3/26

## 问题

单测 netperf 通过，但完整序列（iperf → netperf）中 netperf-glibc 全部失败：

```
enable_enobufs failed: getprotobyname
error starting alarm timer, ret 5 errno 2
```

后续 lua-glibc 阶段触发 `alloc_error` 内核 panic。

## 修复内容

### 1. 缺少 `/etc/protocols`（`os/src/fs/mod.rs`）

`getprotobyname("ip"/"tcp"/"udp")` 依赖 `/etc/protocols`，内核 `ensure_basic_paths()` 未创建此文件，导致 `enable_enobufs failed: getprotobyname`，同时 errno=2（ENOENT）残留，即 alarm 错误中 `errno 2` 的来源。

```rust
// ensure_basic_paths() 中添加
write_file_if_missing(
    "/etc/protocols",
    "ip\t0\tIP\nicmp\t1\tICMP\ntcp\t6\tTCP\nudp\t17\tUDP\n",
);
```

### 2. `reap_orphans` 不充分（`user/src/bin/initcode.rs`）

旧代码只循环 100 次 `waitpid_nohang`，每次 yield 一次。iperf daemon 子进程被 SIGKILL 后需要多个调度轮次才能退出，100 次不够。未回收的僵尸 PCB（PageTable frames、BTreeMap 等）在内核堆上累积，4 轮测试后耗尽 32MB 内核堆，触发 `alloc_error` panic。

改为：kill 范围 `2..512`，循环至 `ECHILD`，安全阀 500 次 yield。

```rust
fn reap_orphans() {
    let my_pid = user_lib::getpid();
    for p in 2..512usize {
        if p as isize != my_pid {
            let _ = kill(p, SIGKILL);
        }
    }
    let mut status: i32 = 0;
    let mut yields_without_reap = 0usize;
    loop {
        let ret = user_lib::waitpid_nohang(-1i32 as usize, &mut status);
        if ret > 0 { yields_without_reap = 0; continue; }
        if ret == 0 {
            yields_without_reap += 1;
            if yields_without_reap > 500 { break; }
            user_lib::sys_yield();
            continue;
        }
        break; // ECHILD
    }
}
```

### 3. `sys_setitimer` 诊断日志（`os/src/syscall/process.rs`）

`alarm(5)` 返回 5 表示进程已有一个 5 秒 alarm 待处理。netperf 的 `start_timer()` 将非零返回视为致命错误并 `exit(-1)`（这是 netperf 自身的代码 bug——POSIX `alarm()` 非零返回只表示前一个 alarm 被替换）。`errno 2` 是 `getprotobyname` 打开 `/etc/protocols` 失败残留的 ENOENT。

为定位 `ret 5` 来源，在 `sys_setitimer` 读取旧值时添加 WARN 日志：

```rust
if remaining > 0 {
    log::warn!(
        "[setitimer] pid={} OLD remaining={}ms expire={} interval={} now={}",
        process.pid.0, remaining, expire, interval, now
    );
}
```

## 测试

```bash
# 单测 netperf
touch os/src/task/initcode.rs
SINGLE_TEST=netperf LOG=WARN make rv
bash run.sh -f sdcard-rv.img -t all 2>&1 | tee test-netperf.log

# 全序列（iperf + netperf）
make clean && touch os/src/task/initcode.rs
LOG=WARN make rv
bash run.sh -f sdcard-rv.img -t all 2>&1 | tee test-all.log

# 检查诊断
grep '\[setitimer\]' test-all.log
grep 'alloc_error' test-all.log
```

## 涉及文件

| 文件 | 修改 |
|------|------|
| `os/src/fs/mod.rs` | `ensure_basic_paths()` 添加 `/etc/protocols` |
| `user/src/bin/initcode.rs` | `reap_orphans()` 循环至 ECHILD，kill 范围 2..512 |
| `os/src/syscall/process.rs` | `sys_setitimer` 旧值非零时输出 WARN 日志 |
