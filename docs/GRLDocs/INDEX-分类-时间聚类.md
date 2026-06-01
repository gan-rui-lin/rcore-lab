# GRLDocs 分类与时间聚类

生成日期：2026-05-23

规则：
- 先按类别，再按时间桶：2025/无日期、2026-01/02/03、2026-04、2026-05、2026-06+。
- 对无日期文件：先查文内“日期：...”，再用 `git log --diff-filter=A` 获取创建日期。
- 标注 `source`：filename（文件名含日期）、doc（文内日期）、git（git log）。

## 进程/线程/执行

### 2026-01/02/03
- [docs/GRLDocs/by-category/process/2026-01-03/initproc_exit_borrowmuterror_debug.md](docs/GRLDocs/by-category/process/2026-01-03/initproc_exit_borrowmuterror_debug.md) (date: 2026-02-08, source: git)
- [docs/GRLDocs/by-category/process/2026-01-03/execvfs-trace-debug-report.md](docs/GRLDocs/by-category/process/2026-01-03/execvfs-trace-debug-report.md) (date: 2026-02-08, source: git)
- [docs/GRLDocs/by-category/process/2026-01-03/thread-model-design.md](docs/GRLDocs/by-category/process/2026-01-03/thread-model-design.md) (date: 2026-02-10, source: git)
- [docs/GRLDocs/by-category/process/2026-01-03/pthread_cancel_final_conclusion_2026-03-01.md](docs/GRLDocs/by-category/process/2026-01-03/pthread_cancel_final_conclusion_2026-03-01.md) (date: 2026-03-01, source: filename)
- [docs/GRLDocs/by-category/process/2026-01-03/UserContext字段适配调试记录.md](docs/GRLDocs/by-category/process/2026-01-03/UserContext字段适配调试记录.md) (date: 2026-03-05, source: doc)
- [docs/GRLDocs/by-category/process/2026-01-03/dlopen当前问题记录.md](docs/GRLDocs/by-category/process/2026-01-03/dlopen当前问题记录.md) (date: 2026-03-06, source: doc)

### 2026-04
- [docs/GRLDocs/by-category/process/2026-04/LA-fork-exec-hang-bug-2026-04-03.md](docs/GRLDocs/by-category/process/2026-04/LA-fork-exec-hang-bug-2026-04-03.md) (date: 2026-04-03, source: filename)

## 进程间通信/同步

### 2026-01/02/03
- [docs/GRLDocs/by-category/ipc/2026-01-03/upintrfreecell-design.md](docs/GRLDocs/by-category/ipc/2026-01-03/upintrfreecell-design.md) (date: 2026-02-11, source: git)
- [docs/GRLDocs/by-category/ipc/2026-01-03/irq_condvar_borrowmuterror_debug.md](docs/GRLDocs/by-category/ipc/2026-01-03/irq_condvar_borrowmuterror_debug.md) (date: 2026-02-11, source: git)
- [docs/GRLDocs/by-category/ipc/2026-01-03/futex-mprotect-clone-stack-perm-debug-2026-02-22.md](docs/GRLDocs/by-category/ipc/2026-01-03/futex-mprotect-clone-stack-perm-debug-2026-02-22.md) (date: 2026-02-22, source: filename)
- [docs/GRLDocs/by-category/ipc/2026-01-03/futex-debug-progress-2026-02-23.md](docs/GRLDocs/by-category/ipc/2026-01-03/futex-debug-progress-2026-02-23.md) (date: 2026-02-23, source: filename)
- [docs/GRLDocs/by-category/ipc/2026-01-03/iozone_shm_fix_notes_2026-03-25.md](docs/GRLDocs/by-category/ipc/2026-01-03/iozone_shm_fix_notes_2026-03-25.md) (date: 2026-03-25, source: filename)

## 信号/异常/定时

### 2026-01/02/03
- [docs/GRLDocs/by-category/signal/2026-01-03/sepc0-debug-case.md](docs/GRLDocs/by-category/signal/2026-01-03/sepc0-debug-case.md) (date: 2026-02-12, source: git)
- [docs/GRLDocs/by-category/signal/2026-01-03/SIGILL_权宜修复说明.md](docs/GRLDocs/by-category/signal/2026-01-03/SIGILL_权宜修复说明.md) (date: 2026-02-17, source: git)
- [docs/GRLDocs/by-category/signal/2026-01-03/动态ELF导致SIGILL调试记录.md](docs/GRLDocs/by-category/signal/2026-01-03/动态ELF导致SIGILL调试记录.md) (date: 2026-02-17, source: git)
- [docs/GRLDocs/by-category/signal/2026-01-03/指令页异常与SIGILL根因分析报告.md](docs/GRLDocs/by-category/signal/2026-01-03/指令页异常与SIGILL根因分析报告.md) (date: 2026-02-17, source: git)
- [docs/GRLDocs/by-category/signal/2026-01-03/sigchld-sigtimedwait-debug-2026-02-22.md](docs/GRLDocs/by-category/signal/2026-01-03/sigchld-sigtimedwait-debug-2026-02-22.md) (date: 2026-02-22, source: filename)

### 2026-04
- [docs/GRLDocs/by-category/signal/2026-04/glibc-setitimer01-页故障调试记录-2026-04-22.md](docs/GRLDocs/by-category/signal/2026-04/glibc-setitimer01-页故障调试记录-2026-04-22.md) (date: 2026-04-22, source: filename)

## 文件系统/存储

### 2026-01/02/03
- [docs/GRLDocs/by-category/fs/2026-01-03/ext4-adaptation-report.md](docs/GRLDocs/by-category/fs/2026-01-03/ext4-adaptation-report.md) (date: 2026-02-08, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/ext4-debug-report-fork-hang.md](docs/GRLDocs/by-category/fs/2026-01-03/ext4-debug-report-fork-hang.md) (date: 2026-02-08, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/fat32_vfs_调试报告.md](docs/GRLDocs/by-category/fs/2026-01-03/fat32_vfs_调试报告.md) (date: 2026-02-09, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/vfs_mkdir_chdir_pipe_调试报告.md](docs/GRLDocs/by-category/fs/2026-01-03/vfs_mkdir_chdir_pipe_调试报告.md) (date: 2026-02-09, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/vfs设计理念.md](docs/GRLDocs/by-category/fs/2026-01-03/vfs设计理念.md) (date: 2026-02-09, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/device-layer-design.md](docs/GRLDocs/by-category/fs/2026-01-03/device-layer-design.md) (date: 2026-02-11, source: git)
- [docs/GRLDocs/by-category/fs/2026-01-03/iozone-rv-vs-la-slow-analysis-2026-03-27.md](docs/GRLDocs/by-category/fs/2026-01-03/iozone-rv-vs-la-slow-analysis-2026-03-27.md) (date: 2026-03-27, source: filename)

### 2026-04
- [docs/GRLDocs/by-category/fs/2026-04/proc-cpuinfo实现记录-2026-04-03.md](docs/GRLDocs/by-category/fs/2026-04/proc-cpuinfo实现记录-2026-04-03.md) (date: 2026-04-03, source: filename)
- [docs/GRLDocs/by-category/fs/2026-04/iozone-Bad-address根因与修复-2026-04-08.md](docs/GRLDocs/by-category/fs/2026-04/iozone-Bad-address根因与修复-2026-04-08.md) (date: 2026-04-08, source: filename)

### 2026-05
- [docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-authoritative-refactor-2026-05-15.md](docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-authoritative-refactor-2026-05-15.md) (date: 2026-05-15, source: filename)
- [docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-cache-ltp-hotspot-optimization-2026-05-23.md](docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-cache-ltp-hotspot-optimization-2026-05-23.md) (date: 2026-05-23, source: filename)
- [docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-csum-disable-tradeoff-2026-05-23.md](docs/GRLDocs/by-category/fs/2026-05/ext4-metadata-csum-disable-tradeoff-2026-05-23.md) (date: 2026-05-23, source: filename)
- [docs/GRLDocs/by-category/fs/2026-05/ext4-runtime-paths-image-corruption-debug-2026-05-23.md](docs/GRLDocs/by-category/fs/2026-05/ext4-runtime-paths-image-corruption-debug-2026-05-23.md) (date: 2026-05-23, source: filename)

## 内存/地址空间/VM

### 2026-01/02/03
- [docs/GRLDocs/by-category/mm/2026-01-03/rcore地址空间与mmap说明.md](docs/GRLDocs/by-category/mm/2026-01-03/rcore地址空间与mmap说明.md) (date: 2026-02-17, source: git)
- [docs/GRLDocs/by-category/mm/2026-01-03/mm_address_unwrap_调试记录.md](docs/GRLDocs/by-category/mm/2026-01-03/mm_address_unwrap_调试记录.md) (date: 2026-03-06, source: doc)
- [docs/GRLDocs/by-category/mm/2026-01-03/RV高半内核路径与页表翻译修复复盘-2026-03-17.md](docs/GRLDocs/by-category/mm/2026-01-03/RV高半内核路径与页表翻译修复复盘-2026-03-17.md) (date: 2026-03-17, source: filename)

### 2026-04
- [docs/GRLDocs/by-category/mm/2026-04/LA-musl-heap-corruption-fix-2026-04-03.md](docs/GRLDocs/by-category/mm/2026-04/LA-musl-heap-corruption-fix-2026-04-03.md) (date: 2026-04-03, source: filename)
- [docs/GRLDocs/by-category/mm/2026-04/调试方法论-LA-musl-heap-crash-2026-04-03.md](docs/GRLDocs/by-category/mm/2026-04/调试方法论-LA-musl-heap-crash-2026-04-03.md) (date: 2026-04-03, source: filename)
- [docs/GRLDocs/by-category/mm/2026-04/rcore-lab内存布局说明.md](docs/GRLDocs/by-category/mm/2026-04/rcore-lab内存布局说明.md) (date: 2026-04-07, source: git)
- [docs/GRLDocs/by-category/mm/2026-04/三内核虚拟地址布局对比分析.md](docs/GRLDocs/by-category/mm/2026-04/三内核虚拟地址布局对比分析.md) (date: 2026-04-07, source: git)
- [docs/GRLDocs/by-category/mm/2026-04/COW与DemandFault顺序与合理性对比说明.md](docs/GRLDocs/by-category/mm/2026-04/COW与DemandFault顺序与合理性对比说明.md) (date: 2026-04-08, source: doc)
- [docs/GRLDocs/by-category/mm/2026-04/rcore与chronix的VMA及COW_Demand设计对比与改进建议-2026-04-09.md](docs/GRLDocs/by-category/mm/2026-04/rcore与chronix的VMA及COW_Demand设计对比与改进建议-2026-04-09.md) (date: 2026-04-09, source: filename)
- [docs/GRLDocs/by-category/mm/2026-04/user_mem设计哲学与Policy分析-2026-04-22.md](docs/GRLDocs/by-category/mm/2026-04/user_mem设计哲学与Policy分析-2026-04-22.md) (date: 2026-04-22, source: filename)

## 架构/平台

### 2026-01/02/03
- [docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_fs_net_stub_说明.md](docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_fs_net_stub_说明.md) (date: 2026-03-08, source: doc)
- [docs/GRLDocs/by-category/arch/2026-01-03/loongarch-bss-stack-clear-debug-2026-03-08.md](docs/GRLDocs/by-category/arch/2026-01-03/loongarch-bss-stack-clear-debug-2026-03-08.md) (date: 2026-03-08, source: filename)
- [docs/GRLDocs/by-category/arch/2026-01-03/loongarch64-task-switch-pte-fix-2026-03-09.md](docs/GRLDocs/by-category/arch/2026-01-03/loongarch64-task-switch-pte-fix-2026-03-09.md) (date: 2026-03-09, source: filename)
- [docs/GRLDocs/by-category/arch/2026-01-03/LoongArch64-ELF加载异常调试分析-2026-03-10.md](docs/GRLDocs/by-category/arch/2026-01-03/LoongArch64-ELF加载异常调试分析-2026-03-10.md) (date: 2026-03-10, source: filename)
- [docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_pthread_tls_调试报告_2026-03-14.md](docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_pthread_tls_调试报告_2026-03-14.md) (date: 2026-03-14, source: filename)
- [docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_本次改动提交说明_2026-03-14.md](docs/GRLDocs/by-category/arch/2026-01-03/LoongArch_本次改动提交说明_2026-03-14.md) (date: 2026-03-14, source: filename)
- [docs/GRLDocs/by-category/arch/2026-01-03/target_arch存量评估与Trap统一方案_2026-03-18.md](docs/GRLDocs/by-category/arch/2026-01-03/target_arch存量评估与Trap统一方案_2026-03-18.md) (date: 2026-03-18, source: filename)

## 性能/基准/评测

### 2026-01/02/03
- [docs/GRLDocs/by-category/perf/2026-01-03/lmbench计时准确性问题记录-2026-03-20.md](docs/GRLDocs/by-category/perf/2026-01-03/lmbench计时准确性问题记录-2026-03-20.md) (date: 2026-03-20, source: filename)
- [docs/GRLDocs/by-category/perf/2026-01-03/lmbench-lat_pipe卡死调试报告-2026-03-24.md](docs/GRLDocs/by-category/perf/2026-01-03/lmbench-lat_pipe卡死调试报告-2026-03-24.md) (date: 2026-03-24, source: filename)
- [docs/GRLDocs/by-category/perf/2026-01-03/lmbench_pipe_signal后续重构路线图-2026-03-24.md](docs/GRLDocs/by-category/perf/2026-01-03/lmbench_pipe_signal后续重构路线图-2026-03-24.md) (date: 2026-03-24, source: filename)
- [docs/GRLDocs/by-category/perf/2026-01-03/lmbench-rv-latfs与memory_set1018-panic调试报告-2026-03-25.md](docs/GRLDocs/by-category/perf/2026-01-03/lmbench-rv-latfs与memory_set1018-panic调试报告-2026-03-25.md) (date: 2026-03-25, source: filename)
- [docs/GRLDocs/by-category/perf/2026-01-03/libcbench-smaps-implementation-review.md](docs/GRLDocs/by-category/perf/2026-01-03/libcbench-smaps-implementation-review.md) (date: 2026-03-25, source: git)
- [docs/GRLDocs/by-category/perf/2026-01-03/lmbench-glibc-alloc_error调试-2026-03-26.md](docs/GRLDocs/by-category/perf/2026-01-03/lmbench-glibc-alloc_error调试-2026-03-26.md) (date: 2026-03-26, source: filename)
- [docs/GRLDocs/by-category/perf/2026-01-03/all-bylm-lmbench-alloc_error综合分析-2026-03-28.md](docs/GRLDocs/by-category/perf/2026-01-03/all-bylm-lmbench-alloc_error综合分析-2026-03-28.md) (date: 2026-03-28, source: filename)

## 锁/并发

### 2026-05
- [docs/GRLDocs/by-category/lock/2026-05/tcb-lock-split-stage3-report-2026-05-24.md](docs/GRLDocs/by-category/lock/2026-05/tcb-lock-split-stage3-report-2026-05-24.md) (date: 2026-05-24, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/pcb-tcb-lock-refactor-2026-05-24.md](docs/GRLDocs/by-category/lock/2026-05/pcb-tcb-lock-refactor-2026-05-24.md) (date: 2026-05-24, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/fs-vfs-hot-lock-split-stage7-report-2026-05-24.md](docs/GRLDocs/by-category/lock/2026-05/fs-vfs-hot-lock-split-stage7-report-2026-05-24.md) (date: 2026-05-24, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/sched-pid2pcb-timer-futex-net-lock-split-stage8-report-2026-05-24.md](docs/GRLDocs/by-category/lock/2026-05/sched-pid2pcb-timer-futex-net-lock-split-stage8-report-2026-05-24.md) (date: 2026-05-24, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/vfs-short-lock-stage9-report-2026-05-25.md](docs/GRLDocs/by-category/lock/2026-05/vfs-short-lock-stage9-report-2026-05-25.md) (date: 2026-05-25, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/block-cache-lock-split-stage10-report-2026-05-25.md](docs/GRLDocs/by-category/lock/2026-05/block-cache-lock-split-stage10-report-2026-05-25.md) (date: 2026-05-25, source: filename)
- [docs/GRLDocs/by-category/lock/2026-05/lock-optimization-overview-2026-05-25.md](docs/GRLDocs/by-category/lock/2026-05/lock-optimization-overview-2026-05-25.md) (date: 2026-05-25, source: filename)

## 测试/LTP与用例

### 2026-04
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-stuck-tests-futex-procfs-nanosleep-debug-2026-04-02.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-stuck-tests-futex-procfs-nanosleep-debug-2026-04-02.md) (date: 2026-04-02, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP卡死测试调试记录-futex与信号处理修复.md](docs/GRLDocs/by-category/ltp/2026-04/LTP卡死测试调试记录-futex与信号处理修复.md) (date: 2026-04-02, source: git)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-fork05-07-09-13-14-调试与修复-2026-04-08.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-fork05-07-09-13-14-调试与修复-2026-04-08.md) (date: 2026-04-08, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-Summary-passed-zero-根因定位与修复-2026-04-09.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-Summary-passed-zero-根因定位与修复-2026-04-09.md) (date: 2026-04-09, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-MAP_SHARED-TPASS存在但Summary为0-根因定位与修复-2026-04-13.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-MAP_SHARED-TPASS存在但Summary为0-根因定位与修复-2026-04-13.md) (date: 2026-04-13, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/ftest05-卡住根因与临时跳过记录-2026-04-13.md](docs/GRLDocs/by-category/ltp/2026-04/ftest05-卡住根因与临时跳过记录-2026-04-13.md) (date: 2026-04-13, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-用户内存访问路径收敛重构说明与后续路线-2026-04-14.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-用户内存访问路径收敛重构说明与后续路线-2026-04-14.md) (date: 2026-04-14, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-lchown03-linkat02-卡住根因与修复-2026-04-14.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-lchown03-linkat02-卡住根因与修复-2026-04-14.md) (date: 2026-04-14, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-diotest4-TPASS后panic-根因定位与修复-2026-04-14.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-diotest4-TPASS后panic-根因定位与修复-2026-04-14.md) (date: 2026-04-14, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/RV-LTP-check_netem-illegal-0826-调试记录-2026-04-15.md](docs/GRLDocs/by-category/ltp/2026-04/RV-LTP-check_netem-illegal-0826-调试记录-2026-04-15.md) (date: 2026-04-15, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-sigtimedwait01-getrusage02-兼容方案与调试记录-2026-04-16.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-sigtimedwait01-getrusage02-兼容方案与调试记录-2026-04-16.md) (date: 2026-04-16, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/truncate03_64-suite-wrapper-hang-debug-2026-04-17.md](docs/GRLDocs/by-category/ltp/2026-04/truncate03_64-suite-wrapper-hang-debug-2026-04-17.md) (date: 2026-04-17, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/wait-语义中断修复说明-2026-04-17.md](docs/GRLDocs/by-category/ltp/2026-04/wait-语义中断修复说明-2026-04-17.md) (date: 2026-04-17, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/LTP-TPASS-HTML与源码联合分析方法与展现模板-2026-04-23.md](docs/GRLDocs/by-category/ltp/2026-04/LTP-TPASS-HTML与源码联合分析方法与展现模板-2026-04-23.md) (date: 2026-04-23, source: filename)
- [docs/GRLDocs/by-category/ltp/2026-04/html-9000-当前完全未过LTP样例源码语义分析-2026-04-23.md](docs/GRLDocs/by-category/ltp/2026-04/html-9000-当前完全未过LTP样例源码语义分析-2026-04-23.md) (date: 2026-04-23, source: filename)

## 工具/调试/指南

### 2026-01/02/03
- [docs/GRLDocs/by-category/tools/2026-01-03/23年官方文档.md](docs/GRLDocs/by-category/tools/2026-01-03/23年官方文档.md) (date: 2026-02-07, source: git)
- [docs/GRLDocs/by-category/tools/2026-01-03/如何执行.md](docs/GRLDocs/by-category/tools/2026-01-03/如何执行.md) (date: 2026-02-08, source: git)
- [docs/GRLDocs/by-category/tools/2026-01-03/gdb-debug-notes.md](docs/GRLDocs/by-category/tools/2026-01-03/gdb-debug-notes.md) (date: 2026-02-11, source: git)
- [docs/GRLDocs/by-category/tools/2026-01-03/gdb-debug-guide.md](docs/GRLDocs/by-category/tools/2026-01-03/gdb-debug-guide.md) (date: 2026-02-12, source: git)
- [docs/GRLDocs/by-category/tools/2026-01-03/loongarch64-gdb-debug-guide.md](docs/GRLDocs/by-category/tools/2026-01-03/loongarch64-gdb-debug-guide.md) (date: 2026-03-09, source: git)
- [docs/GRLDocs/by-category/tools/2026-01-03/glibc完整调试手册-2026-03-20.md](docs/GRLDocs/by-category/tools/2026-01-03/glibc完整调试手册-2026-03-20.md) (date: 2026-03-20, source: filename)
