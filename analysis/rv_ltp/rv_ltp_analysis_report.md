# RV LTP musl/glibc 日志分析报告

- 日志: `ltp_for_analysis_RV.log`
- LTP root: `/home/grl/codeRepo/testsuits-for-oskernel/ltp-full-20240524`
- libc 判定: `both`；该日志没有 judge score key，因此当前得分按日志内 `TPASS` 计数。
- 注意：若 case 先报 `TCONF` 后提前退出，当前 `TPASS=0` 可能掩盖源码中后续可获得的 TPASS；报告用 `masked_by_tconf` 单独标记，并要求源码级校验。

## 当前得分与结果概览
| libc | cases | TPASS | TFAIL | TBROK | TCONF | TWARN | 0 TPASS | masked TCONF | clean | partial |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| musl | 1523 | 3457 | 514 | 350 | 895 | 130 | 893 | 275 | 520 | 88 |
| glibc | 1523 | 2962 | 451 | 512 | 818 | 143 | 986 | 227 | 443 | 78 |

## 不应该加入测试的条目
| case | musl | glibc | reason | diagnostic |
| --- | --- | --- | --- | --- |
| check_icmpv4_connectivity | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L1585: Usage: /musl/ltp/testcases/bin/check_icmpv4_connectivity source_interface_name destionation_ipv4_address |
| check_icmpv6_connectivity | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L1588: Usage: /musl/ltp/testcases/bin/check_icmpv6_connectivity source_interface_name destionation_ipv6_address |
| cpuacct_task | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L2491: Usage: ltp/testcases/bin/cpuacct_task /cgroup/.../tasks |
| create_datafile | TPASS=0, rc=3, exclude_from_runlist | TPASS=0, rc=3, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L2555: usage: |
| create_file | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L2560: Usage: create_file filename filesize |
| data | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L2563: /musl/ltp/testcases/bin/data: line 1: xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx: not found \| L2564: /musl/ltp/testcases/bin/data: ... |
| dirty | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| growfiles | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| kernbench | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L6676: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/kernbench: not found |
| libcgroup_freezer | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L6984: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/libcgroup_freezer: not found |
| locktests | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| ltpServer | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| mc_member_test | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L7500: usage: ltp/testcases/bin/mc_member_test [ -j -l ] -g group_list [-s time_to_sleep] -i interface_name (or i.i.i.i) |
| mc_recv | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L7503: usage: ltp/testcases/bin/mc_recv g.g.g.g interface_name (or i.i.i.i) port |
| mc_send | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L7506: usage: ltp/testcases/bin/mc_send g.g.g.g interface_name (or i.i.i.i) port [ttl] |
| mc_verify_opts | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L7509: usage: ltp/testcases/bin/mc_verify_opts interface_name (or i.i.i.i) |
| mc_verify_opts_error | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L7512: usage: ltp/testcases/bin/mc_verify_opts_error interface_name (or i.i.i.i) |
| mmap-corruption01 | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| mmap2 | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| mmstress_dummy | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| nfs01_open_files | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9227: Usage: ltp/testcases/bin/nfs01_open_files <number of files> |
| nfs04_create_file | TPASS=0, rc=3, exclude_from_runlist | TPASS=0, rc=3, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9230: usage: |
| nfs_flock | TPASS=0, rc=2, exclude_from_runlist | TPASS=0, rc=2, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9249: Usage: ltp/testcases/bin/nfs_flock <mac num> <file name> <nchars> <nlines> |
| nfs_flock_dgen | TPASS=0, rc=2, exclude_from_runlist | TPASS=0, rc=2, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9252: usage: <nfs_flock_dgen> <file> <char/line> <lines> <ctype> |
| ns-echoclient | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9331: server name isn't specified. |
| ns-tcpclient | TPASS=0, rc=1, exclude_from_runlist | TPASS=0, rc=1, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L9457: server name isn't specified. |
| openfile | - | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| pm_cpu_consolidation.py | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L10683: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/pm_cpu_consolidation.py: not found |
| pm_ilb_test.py | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L10689: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/pm_ilb_test.py: not found |
| pm_sched_domain.py | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L10692: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/pm_sched_domain.py: not found |
| pm_sched_mc.py | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L10695: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/pm_sched_mc.py: not found |
| print_caps | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| rwtest | TPASS=0, rc=127, exclude_from_runlist | TPASS=0, rc=127, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L14667: /tmp/ltp_testcode_filtered.sh: line 48: ltp/testcases/bin/rwtest: not found |
| sched_tc2 | TPASS=0, rc=0, exclude_from_runlist | TPASS=0, rc=0, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L15190: Usage: ltp/testcases/bin/sched_tc2 [-p priority] [-t sec] [-v] [-d] \| L15191: -t sec execution time (default 1800 sec) ... |
| sched_tc3 | TPASS=0, rc=0, exclude_from_runlist | TPASS=0, rc=0, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L15197: Usage: ltp/testcases/bin/sched_tc3 [-p priority] [-v] [-d] \| L15198: -p priority priority (default variable) |
| sched_tc4 | TPASS=0, rc=0, exclude_from_runlist | TPASS=0, rc=0, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L15203: Usage: ltp/testcases/bin/sched_tc4 [-l log] [-t type] [-p priority] [-v] [-d] |
| sched_tc5 | TPASS=0, rc=0, exclude_from_runlist | TPASS=0, rc=0, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L15211: Usage: ltp/testcases/bin/sched_tc5 [-l log] [-t type] [-p priority] [-v] [-d] |
| stress | TPASS=0, rc=0, exclude_from_runlist | TPASS=0, rc=0, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L19261: Usage: stress [OPTION [ARG]] ... \| L19275: --vm-chunks c malloc c chunks (default is 1) \| L19276: --vm-bytes b malloc c... |
| test_ioctl | TPASS=0, rc=0, empty_or_silent | TPASS=0, rc=0, empty_or_silent | no LTP result markers observed |  |
| testsf_c | TPASS=0, rc=2, exclude_from_runlist | TPASS=0, rc=2, exclude_from_runlist | helper/data/support entry, not a standalone syscall test | L19972: sendfile_client 1 TBROK : testsf_c.c:42: usage: server-ip port client-file server-file file-len \| L19973: sendfile_clie... |

## TWARN/TCONF 中容易修复的条目
| case | musl | glibc | expected TPASS | diagnostic |
| --- | --- | --- | --- | --- |
| add_key05 | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | 6 | L770: tst_cmd.c:257: TCONF: Couldn't find 'useradd' in $PATH |
| ioctl_loop05 | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | 5 | L6400: tst_kernel.c:126: TWARN: expected file /lib/modules/5.10.0/modules.dep does not exist or not a file \| L6401: tst_kernel.... |
| ioctl_loop02 | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | 4 | L6385: tst_kernel.c:126: TWARN: expected file /lib/modules/5.10.0/modules.dep does not exist or not a file \| L6386: tst_kernel.... |
| setegid01 | TPASS=4, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 4 | L16007: setegid01.c:36: TINFO: call setegid(nobody_gid 65534) \| L16009: setegid01.c:43: TPASS: nobody_gid == cur_egid (65534) \|... |
| setreuid04 | TPASS=3, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 4 | L17113: setreuid04.c:36: TPASS: SETREUID(nobody_uid, nobody_uid) passed \| L17114: setreuid04.c:38: TPASS: GETUID() == nobody_ui... |
| access01 | TPASS=199, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 3 | L401: access01.c:245: TPASS: access(accessfile_rwx, F_OK) as root passed \| L402: access01.c:245: TPASS: access(accessfile_rwx, ... |
| access02 | TPASS=16, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 3 | L613: access02.c:139: TPASS: access(file_f, F_OK) as root behaviour is correct. \| L614: access02.c:139: TPASS: access(file_f, F... |
| access03 | TPASS=8, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 3 | L642: access03.c:37: TPASS: invalid address as root : EFAULT (14) \| L643: access03.c:46: TPASS: invalid address as nobody : EFA... |
| ioctl_loop01 | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | 3 | L6380: tst_kernel.c:126: TWARN: expected file /lib/modules/5.10.0/modules.dep does not exist or not a file \| L6381: tst_kernel.... |
| open10 | TPASS=6, rc=1, partial_failure | TPASS=0, rc=2, zero_tpass_failure | 3 | L9674: open10.c:42: TINFO: User nobody: uid = 65534, gid = 65534 \| L9676: open10.c:60: TPASS: dir_a/nosetgid: Owned by correct ... |
| setresuid04 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L38116: setresuid04.c:32: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setresuid04_16 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L38130: setresuid04.c:32: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setreuid05 | TPASS=14, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 3 | L17143: setreuid05.c:91: TPASS: setreuid(nobody, root) passed \| L17144: setreuid05.c:91: TPASS: setreuid(-1, nobody) passed \| L... |
| setreuid07 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L38346: setreuid07.c:33: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setreuid07_16 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L38360: setreuid07.c:33: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| shmctl04 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L38843: shmctl04.c:157: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| stat01 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L39945: stat01.c:56: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| stat01_64 | - | TPASS=0, rc=2, zero_tpass_failure | 6 | L39959: stat01.c:56: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| msgctl06 | - | TPASS=0, rc=2, zero_tpass_failure | 5 | L30411: msgctl06.c:146: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| access04 | TPASS=12, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L660: tst_test.c:1017: TINFO: Cannot resolve the absolute path of mntpoint: ENOENT (2) \| L665: access04.c:68: TPASS: access as ... |
| readlink01 | TPASS=2, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L12819: readlink01.c:45: TPASS: readlink() functionality on 'slink_file' was correct \| L12820: readlink01.c:55: TINFO: Running ... |
| sched_setscheduler03 | TPASS=2, rc=1, partial_failure | TPASS=0, rc=2, zero_tpass_failure | 2 | L15116: sched_setscheduler03.c:136: TINFO: Setting euid to nobody to drop privilege \| L15119: sched_setscheduler03.c:100: TPASS... |
| semctl09 | - | TPASS=0, rc=2, zero_tpass_failure | 4 | L36709: semctl09.c:184: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setgid03 | TPASS=2, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L16250: setgid03.c:21: TPASS: SETGID(nobody->pw_gid) passed \| L16251: setgid03.c:26: TPASS: functionality of getgid() is correct |
| setregid03 | - | TPASS=0, rc=2, zero_tpass_failure | 4 | L37881: setregid03.c:59: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setregid03_16 | - | TPASS=0, rc=2, zero_tpass_failure | 4 | L37895: setregid03.c:59: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setresuid02 | TPASS=4, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L16884: setresuid02.c:72: TPASS: setresuid(-1, -1, other) works as expected \| L16885: setresuid02.c:72: TPASS: setresuid(-1, no... |
| setreuid02 | TPASS=7, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L17038: setreuid02.c:78: TPASS: setreuid(-1, -1) works as expected \| L17039: setreuid02.c:78: TPASS: setreuid(nobody, -1) works... |
| setreuid03 | TPASS=14, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 2 | L17072: setreuid03.c:88: TPASS: setreuid(nobody, nobody) passed \| L17073: setreuid03.c:88: TPASS: setreuid(-1, nobody) passed \|... |
| setreuid04_16 | - | TPASS=0, rc=2, zero_tpass_failure | 4 | L38276: setreuid04.c:26: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| mlock02 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L29636: mlock02.c:89: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setresuid05 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L38144: setresuid05.c:25: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setresuid05_16 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L38158: setresuid05.c:25: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setreuid05_16 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L38304: setreuid05.c:70: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setreuid06 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L38318: setreuid06.c:33: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| setreuid06_16 | - | TPASS=0, rc=2, zero_tpass_failure | 3 | L38332: setreuid06.c:33: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| bind02 | TPASS=1, rc=0, clean_pass | TPASS=0, rc=2, zero_tpass_failure | 1 | L1130: bind02.c:52: TINFO: Switching credentials to user: nobody, group: nogroup \| L1131: bind02.c:39: TPASS: bind() : EACCES (13) |
| chmod03 | - | TPASS=0, rc=2, zero_tpass_failure | 2 | L23252: chmod03.c:64: TBROK: getpwnam(nobody) failed: EFAULT (14) |
| clock_gettime04 | TPASS=6, rc=0, clean_pass | TPASS=6, rc=0, clean_pass | 1 | L2101: sh: systemd-detect-virt: not found \| L2105: clock_gettime04.c:183: TPASS: CLOCK_REALTIME: Difference between successive ... |
| fchmod03 | - | TPASS=0, rc=2, zero_tpass_failure | 2 | L24981: fchmod03.c:47: TBROK: getpwnam(nobody) failed: EFAULT (14) |

## TCONF 掩盖的潜在 TPASS
这些 case 的第一个结果标记是 `TCONF`，但源码中仍能看到 TPASS/TST_EXP 成功点；因此不能只按当前日志的 0 TPASS 判断价值，后续需要打开源码确认修掉配置门槛后能释放多少分。
| case | libc | gate | gate TPASS | file TPASS | cluster | source | diagnostic |
| --- | --- | --- | --- | --- | --- | --- | --- |
| process_vm01 | glibc,musl | kernel_feature_gate | 0 | 16 | process_vm | cma/process_vm01.c | L11381: process_vm01.c:83: TCONF: syscall(271) __NR_process_vm_writev not supported on your arch |
| add_key05 | glibc,musl | env_unmask | 0 | 6 | keyctl | add_key/add_key05.c | L770: tst_cmd.c:257: TCONF: Couldn't find 'useradd' in $PATH |
| migrate_pages02 | glibc,musl | env_unmask | 0 | 6 | lib/userland | migrate_pages/migrate_pages02.c | L7748: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| quotactl09 | glibc,musl | env_unmask | 5 | 5 | lib/userland | quotactl/quotactl09.c | L12679: tst_cmd.c:257: TCONF: Couldn't find 'mkfs.ext4' in $PATH |
| quotactl06 | glibc,musl | env_unmask | 4 | 4 | quotactl06 | quotactl/quotactl06.c | L12670: tst_cmd.c:257: TCONF: Couldn't find 'quotacheck' in $PATH |
| ioctl09 | glibc,musl | env_unmask | 0 | 4 | ioctl09 | ioctl/ioctl09.c | L6377: tst_cmd.c:257: TCONF: Couldn't find 'parted' in $PATH |
| statx05 | glibc,musl | env_unmask | 0 | 4 | lib/userland | statx/statx05.c | L19130: tst_cmd.c:257: TCONF: Couldn't find 'mkfs.ext4' in $PATH |
| prctl03 | glibc,musl | semantic_gate | 6 | 7 | prctl | prctl/prctl03.c | L10987: prctl03.c:71: TCONF: prctl() doesn't support PR_SET_CHILD_SUBREAPER |
| prctl07 | glibc,musl | semantic_gate | 0 | 6 | prctl | prctl/prctl07.c | L11055: prctl07.c:168: TCONF: kernel doesn't support PR_CAP_AMBIENT |
| shmctl04 | musl | semantic_gate | 0 | 6 | ipc_shm | ipc/shmctl/shmctl04.c | L17748: shmctl04.c:168: TCONF: kernel doesn't support SHM_STAT_ANY |
| mbind04 | glibc,musl | env_unmask | 3 | 3 | lib/userland | mbind/mbind04.c | L7497: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| set_mempolicy01 | glibc,musl | env_unmask | 3 | 3 | lib/userland | set_mempolicy/set_mempolicy01.c | L15912: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| set_mempolicy02 | glibc,musl | env_unmask | 3 | 3 | lib/userland | set_mempolicy/set_mempolicy02.c | L15915: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| set_mempolicy04 | glibc,musl | env_unmask | 3 | 3 | lib/userland | set_mempolicy/set_mempolicy04.c | L15921: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| quotactl07 | glibc,musl | env_unmask | 0 | 3 | quotactl07 | quotactl/quotactl07.c | L12673: tst_test.c:1175: TCONF: System doesn't have <xfs/xqm.h> |
| mallopt01 | musl | semantic_gate | 0 | 5 | mallopt01 | mallopt/mallopt01.c | L7471: tst_test.c:1175: TCONF: system doesn't implement non-POSIX mallopt() |
| msgctl06 | musl | semantic_gate | 0 | 5 | ipc_msg | ipc/msgctl/msgctl06.c | L8775: msgctl06.c:156: TCONF: kernel doesn't support MSG_STAT_ANY |
| mbind03 | glibc,musl | env_unmask | 2 | 2 | lib/userland | mbind/mbind03.c | L7494: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| quotactl01 | glibc,musl | env_unmask | 2 | 2 | quotactl01 | quotactl/quotactl01.c | L12655: tst_cmd.c:257: TCONF: Couldn't find 'quotacheck' in $PATH |
| quotactl04 | glibc,musl | env_unmask | 2 | 2 | lib/userland | quotactl/quotactl04.c | L12664: tst_cmd.c:257: TCONF: Couldn't find 'mkfs.ext4' in $PATH |
| quotactl08 | glibc,musl | env_unmask | 2 | 2 | lib/userland | quotactl/quotactl08.c | L12676: tst_cmd.c:257: TCONF: Couldn't find 'mkfs.ext4' in $PATH |
| set_mempolicy03 | glibc,musl | env_unmask | 2 | 2 | lib/userland | set_mempolicy/set_mempolicy03.c | L15918: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| mmap16 | glibc,musl | env_unmask | 0 | 2 | lib/userland | mmap/mmap16.c | L8248: tst_cmd.c:257: TCONF: Couldn't find 'mkfs.ext4' in $PATH |
| prctl08 | glibc,musl | semantic_gate | 0 | 4 | prctl | prctl/prctl08.c | L11069: prctl08.c:119: TCONF: proc doesn't support timerslack_ns interface \| L11071: prctl08.c:107: TFAIL: prctl(PR_SET_TIMERSL... |
| get_mempolicy01 | glibc,musl | env_unmask | 1 | 1 | lib/userland | get_mempolicy/get_mempolicy01.c | L4191: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| get_mempolicy02 | glibc,musl | env_unmask | 1 | 1 | lib/userland | get_mempolicy/get_mempolicy02.c | L4194: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| io_destroy01 | glibc,musl | env_unmask | 1 | 1 | lib/userland | io_destroy/io_destroy01.c | L6214: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| io_pgetevents01 | glibc,musl | env_unmask | 1 | 1 | lib/userland | io_pgetevents/io_pgetevents01.c | L6228: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| io_pgetevents02 | glibc,musl | env_unmask | 1 | 1 | lib/userland | io_pgetevents/io_pgetevents02.c | L6231: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| io_setup01 | glibc,musl | env_unmask | 1 | 2 | lib/userland | io_setup/io_setup01.c | L6234: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| io_submit01 | glibc,musl | env_unmask | 1 | 1 | lib/userland | io_submit/io_submit01.c | L6241: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| mbind01 | glibc,musl | env_unmask | 1 | 1 | lib/userland | mbind/mbind01.c | L7488: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| mbind02 | glibc,musl | env_unmask | 1 | 1 | lib/userland | mbind/mbind02.c | L7491: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| migrate_pages03 | glibc,musl | env_unmask | 1 | 1 | lib/userland | migrate_pages/migrate_pages03.c | L7751: tst_test.c:1175: TCONF: require libnuma >= 2 and it's development packages |
| move_pages12 | glibc,musl | env_unmask | 1 | 2 | lib/userland | move_pages/move_pages12.c | L8504: tst_test.c:1175: TCONF: test requires libnuma development packages with LIBNUMA_API_VERSION >= 2 |
| quotactl02 | glibc,musl | env_unmask | 1 | 1 | quotactl02 | quotactl/quotactl02.c | L12658: tst_test.c:1175: TCONF: System doesn't have <xfs/xqm.h> |
| quotactl03 | glibc,musl | env_unmask | 1 | 1 | quotactl03 | quotactl/quotactl03.c | L12661: tst_test.c:1175: TCONF: System doesn't have <xfs/xqm.h> |
| quotactl05 | glibc,musl | env_unmask | 1 | 1 | quotactl05 | quotactl/quotactl05.c | L12667: tst_test.c:1175: TCONF: This system didn't have <xfs/xqm.h> |
| io_cancel02 | glibc,musl | env_unmask | 0 | 1 | lib/userland | io_cancel/io_cancel02.c | L6211: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| io_getevents02 | glibc,musl | env_unmask | 0 | 1 | lib/userland | io_getevents/io_getevents02.c | L6225: tst_test.c:1175: TCONF: test requires libaio and it's development packages |
| move_pages04 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages04.c | L8476: move_pages04 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages05 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages05.c | L8480: move_pages05 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages06 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages06.c | L8484: move_pages06 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages07 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages07.c | L8488: move_pages07 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages09 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages09.c | L8492: move_pages09 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages10 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages10.c | L8496: move_pages10 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| move_pages11 | glibc,musl | env_unmask | 0 | 1 | lib/userland | move_pages/move_pages11.c | L8500: move_pages11 1 TCONF : move_pages_support.c:411: test requires libnuma development packages with LIBNUMA_API_VERSION >= ... |
| gethostid01 | musl | semantic_gate | 0 | 3 | gethostid01 | gethostid/gethostid01.c | L4590: tst_test.c:1175: TCONF: sethostid is undefined. |
| shmget02 | glibc,musl | semantic_gate | 0 | 3 | ipc_shm | ipc/shmget/shmget02.c | L17856: tst_sys_conf.c:72: TCONF: Path not found: /proc/sys/kernel/shmmax: ENOENT (2) |
| brk01 | musl | semantic_gate | 2 | 2 | brk01 | brk/brk01.c | L1372: brk01.c:35: TCONF: brk() not implemented \| L1375: brk01.c:70: TPASS: brk() works fine |

## masked_release_candidates
优先看 `env_unmask` 和 `semantic_gate`：这些更可能用小环境修复或小语义修复释放被 TCONF 挡住的 TPASS。
| rank | case | score | gate | gate TPASS | cluster | action | source | repro |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | add_key05 | 72 | env_unmask | 0 | keyctl | fix image/userland dependency, then run one-case repro | add_key/add_key05.c | SINGLE_TEST=all LTP_START_FROM=add_key05 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 2 | migrate_pages02 | 72 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | migrate_pages/migrate_pages02.c | SINGLE_TEST=all LTP_START_FROM=migrate_pages02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 3 | quotactl09 | 68 | env_unmask | 5 | lib/userland | fix image/userland dependency, then run one-case repro | quotactl/quotactl09.c | SINGLE_TEST=all LTP_START_FROM=quotactl09 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 4 | quotactl06 | 64 | env_unmask | 4 | quotactl06 | fix image/userland dependency, then run one-case repro | quotactl/quotactl06.c | SINGLE_TEST=all LTP_START_FROM=quotactl06 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 5 | ioctl09 | 64 | env_unmask | 0 | ioctl09 | fix image/userland dependency, then run one-case repro | ioctl/ioctl09.c | SINGLE_TEST=all LTP_START_FROM=ioctl09 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 6 | statx05 | 64 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | statx/statx05.c | SINGLE_TEST=all LTP_START_FROM=statx05 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 7 | prctl03 | 62 | semantic_gate | 6 | prctl | read source gate and implement the small semantic prerequisite first | prctl/prctl03.c | SINGLE_TEST=all LTP_START_FROM=prctl03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 8 | prctl07 | 62 | semantic_gate | 0 | prctl | read source gate and implement the small semantic prerequisite first | prctl/prctl07.c | SINGLE_TEST=all LTP_START_FROM=prctl07 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 9 | shmctl04 | 62 | semantic_gate | 0 | ipc_shm | read source gate and implement the small semantic prerequisite first | ipc/shmctl/shmctl04.c | SINGLE_TEST=all LTP_START_FROM=shmctl04 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 10 | mbind04 | 60 | env_unmask | 3 | lib/userland | fix image/userland dependency, then run one-case repro | mbind/mbind04.c | SINGLE_TEST=all LTP_START_FROM=mbind04 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 11 | set_mempolicy01 | 60 | env_unmask | 3 | lib/userland | fix image/userland dependency, then run one-case repro | set_mempolicy/set_mempolicy01.c | SINGLE_TEST=all LTP_START_FROM=set_mempolicy01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 12 | set_mempolicy02 | 60 | env_unmask | 3 | lib/userland | fix image/userland dependency, then run one-case repro | set_mempolicy/set_mempolicy02.c | SINGLE_TEST=all LTP_START_FROM=set_mempolicy02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 13 | set_mempolicy04 | 60 | env_unmask | 3 | lib/userland | fix image/userland dependency, then run one-case repro | set_mempolicy/set_mempolicy04.c | SINGLE_TEST=all LTP_START_FROM=set_mempolicy04 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 14 | quotactl07 | 60 | env_unmask | 0 | quotactl07 | fix image/userland dependency, then run one-case repro | quotactl/quotactl07.c | SINGLE_TEST=all LTP_START_FROM=quotactl07 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 15 | mallopt01 | 58 | semantic_gate | 0 | mallopt01 | read source gate and implement the small semantic prerequisite first | mallopt/mallopt01.c | SINGLE_TEST=all LTP_START_FROM=mallopt01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 16 | msgctl06 | 58 | semantic_gate | 0 | ipc_msg | read source gate and implement the small semantic prerequisite first | ipc/msgctl/msgctl06.c | SINGLE_TEST=all LTP_START_FROM=msgctl06 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 17 | mbind03 | 56 | env_unmask | 2 | lib/userland | fix image/userland dependency, then run one-case repro | mbind/mbind03.c | SINGLE_TEST=all LTP_START_FROM=mbind03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 18 | quotactl01 | 56 | env_unmask | 2 | quotactl01 | fix image/userland dependency, then run one-case repro | quotactl/quotactl01.c | SINGLE_TEST=all LTP_START_FROM=quotactl01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 19 | quotactl04 | 56 | env_unmask | 2 | lib/userland | fix image/userland dependency, then run one-case repro | quotactl/quotactl04.c | SINGLE_TEST=all LTP_START_FROM=quotactl04 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 20 | quotactl08 | 56 | env_unmask | 2 | lib/userland | fix image/userland dependency, then run one-case repro | quotactl/quotactl08.c | SINGLE_TEST=all LTP_START_FROM=quotactl08 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 21 | set_mempolicy03 | 56 | env_unmask | 2 | lib/userland | fix image/userland dependency, then run one-case repro | set_mempolicy/set_mempolicy03.c | SINGLE_TEST=all LTP_START_FROM=set_mempolicy03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 22 | mmap16 | 56 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | mmap/mmap16.c | SINGLE_TEST=all LTP_START_FROM=mmap16 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 23 | prctl08 | 54 | semantic_gate | 0 | prctl | read source gate and implement the small semantic prerequisite first | prctl/prctl08.c | SINGLE_TEST=all LTP_START_FROM=prctl08 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 24 | get_mempolicy01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | get_mempolicy/get_mempolicy01.c | SINGLE_TEST=all LTP_START_FROM=get_mempolicy01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 25 | get_mempolicy02 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | get_mempolicy/get_mempolicy02.c | SINGLE_TEST=all LTP_START_FROM=get_mempolicy02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 26 | io_destroy01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | io_destroy/io_destroy01.c | SINGLE_TEST=all LTP_START_FROM=io_destroy01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 27 | io_pgetevents01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | io_pgetevents/io_pgetevents01.c | SINGLE_TEST=all LTP_START_FROM=io_pgetevents01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 28 | io_pgetevents02 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | io_pgetevents/io_pgetevents02.c | SINGLE_TEST=all LTP_START_FROM=io_pgetevents02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 29 | io_setup01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | io_setup/io_setup01.c | SINGLE_TEST=all LTP_START_FROM=io_setup01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 30 | io_submit01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | io_submit/io_submit01.c | SINGLE_TEST=all LTP_START_FROM=io_submit01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 31 | mbind01 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | mbind/mbind01.c | SINGLE_TEST=all LTP_START_FROM=mbind01 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 32 | mbind02 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | mbind/mbind02.c | SINGLE_TEST=all LTP_START_FROM=mbind02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 33 | migrate_pages03 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | migrate_pages/migrate_pages03.c | SINGLE_TEST=all LTP_START_FROM=migrate_pages03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 34 | move_pages12 | 52 | env_unmask | 1 | lib/userland | fix image/userland dependency, then run one-case repro | move_pages/move_pages12.c | SINGLE_TEST=all LTP_START_FROM=move_pages12 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 35 | quotactl02 | 52 | env_unmask | 1 | quotactl02 | fix image/userland dependency, then run one-case repro | quotactl/quotactl02.c | SINGLE_TEST=all LTP_START_FROM=quotactl02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 36 | quotactl03 | 52 | env_unmask | 1 | quotactl03 | fix image/userland dependency, then run one-case repro | quotactl/quotactl03.c | SINGLE_TEST=all LTP_START_FROM=quotactl03 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 37 | quotactl05 | 52 | env_unmask | 1 | quotactl05 | fix image/userland dependency, then run one-case repro | quotactl/quotactl05.c | SINGLE_TEST=all LTP_START_FROM=quotactl05 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 38 | io_cancel02 | 52 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | io_cancel/io_cancel02.c | SINGLE_TEST=all LTP_START_FROM=io_cancel02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 39 | io_getevents02 | 52 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | io_getevents/io_getevents02.c | SINGLE_TEST=all LTP_START_FROM=io_getevents02 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |
| 40 | move_pages04 | 52 | env_unmask | 0 | lib/userland | fix image/userland dependency, then run one-case repro | move_pages/move_pages04.c | SINGLE_TEST=all LTP_START_FROM=move_pages04 LTP_CASE_LIMIT=1 LTP_CASE_TIMEOUT=8 bash run.sh |

## masked_dead_or_defer
这些 case 多数需要大功能簇或当前 RV/image 设置变化；先按 feature cluster 聚合，不按单个 case 冲刺。
| rank | case | score | gate | gate TPASS | cluster | source | gate source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | process_vm01 | 84 | kernel_feature_gate | 0 | process_vm | cma/process_vm01.c |  |
| 2 | io_uring01 | 44 | kernel_feature_gate | 0 | io_uring | io_uring/io_uring01.c |  |
| 3 | madvise09 | 40 | kernel_feature_gate | 5 | cgroup | madvise/madvise09.c | tst_brk(TCONF \| TERRNO, "MADV_FREE is not supported"); |
| 4 | bpf_map01 | 40 | kernel_feature_gate | 0 | bpf | bpf/bpf_map01.c |  |
| 5 | pidfd_getfd02 | 40 | kernel_feature_gate | 0 | pidfd | pidfd_getfd/pidfd_getfd02.c |  |
| 6 | pidfd_open04 | 40 | kernel_feature_gate | 0 | pidfd | pidfd_open/pidfd_open04.c | tst_brk(TCONF, "PIDFD_NONBLOCK was supported since linux 5.10"); |
| 7 | pkey01 | 40 | kernel_feature_gate | 0 | pkey | pkeys/pkey01.c |  |
| 8 | request_key03 | 36 | kernel_feature_gate | 4 | keyctl | request_key/request_key03.c | tst_res(TCONF, "kernel doesn't support key type '%s'", |
| 9 | inotify12 | 36 | kernel_feature_gate | 0 | inotify12 | inotify/inotify12.c | TST_TEST_TCONF("system doesn't have required inotify support"); |
| 10 | inotify_init1_01 | 36 | kernel_feature_gate | 0 | inotify | inotify_init/inotify_init1_01.c |  |
| 11 | inotify_init1_02 | 36 | kernel_feature_gate | 0 | inotify | inotify_init/inotify_init1_02.c |  |
| 12 | process_vm_readv03 | 36 | kernel_feature_gate | 0 | process_vm | cma/process_vm_readv03.c |  |
| 13 | process_vm_writev02 | 36 | kernel_feature_gate | 0 | process_vm | cma/process_vm_writev02.c |  |
| 14 | sched_setparam02 | 36 | kernel_feature_gate | 0 | sched | sched_setparam/sched_setparam02.c |  |
| 15 | sched_setparam03 | 36 | kernel_feature_gate | 0 | sched | sched_setparam/sched_setparam03.c |  |
| 16 | setns02 | 36 | kernel_feature_gate | 0 | ipc_shm | setns/setns02.c | tst_res(TCONF, "CLONE_NEWUTS is not supported"); |
| 17 | keyctl05 | 32 | kernel_feature_gate | 3 | keyctl | keyctl/keyctl05.c | tst_res(TCONF, "key size not allowed in FIPS mode"); |
| 18 | keyctl07 | 32 | kernel_feature_gate | 0 | keyctl | keyctl/keyctl07.c | if (WIFEXITED(status) && WEXITSTATUS(status) == TCONF) |
| 19 | pidfd_getfd01 | 32 | kernel_feature_gate | 0 | pidfd | pidfd_getfd/pidfd_getfd01.c |  |
| 20 | pidfd_open03 | 32 | kernel_feature_gate | 0 | pidfd | pidfd_open/pidfd_open03.c |  |
| 21 | pidfd_send_signal03 | 32 | kernel_feature_gate | 0 | pidfd | pidfd_send_signal/pidfd_send_signal03.c | tst_brk(TCONF, "%s does not exist, cannot set PIDs", |
| 22 | process_vm_readv02 | 32 | kernel_feature_gate | 0 | process_vm | cma/process_vm_readv02.c |  |
| 23 | readahead01 | 32 | kernel_feature_gate | 0 | readahead01 | readahead/readahead01.c | TST_TEST_TCONF("System doesn't support __NR_readahead"); |
| 24 | timer_gettime01 | 32 | kernel_feature_gate | 0 | timer | timer_gettime/timer_gettime01.c |  |
| 25 | add_key01 | 28 | kernel_feature_gate | 2 | keyctl | add_key/add_key01.c | tst_res(TCONF, "skipping unsupported logon key"); |
| 26 | add_key04 | 28 | kernel_feature_gate | 0 | keyctl | add_key/add_key04.c |  |
| 27 | bpf_prog02 | 28 | kernel_feature_gate | 0 | bpf | bpf/bpf_prog02.c |  |
| 28 | inotify06 | 28 | kernel_feature_gate | 0 | inotify06 | inotify/inotify06.c | TST_TEST_TCONF("system doesn't have required inotify support"); |
| 29 | inotify11 | 28 | kernel_feature_gate | 0 | inotify11 | inotify/inotify11.c | TST_TEST_TCONF("system doesn't have required inotify support"); |
| 30 | io_uring02 | 28 | kernel_feature_gate | 0 | io_uring | io_uring/io_uring02.c |  |
| 31 | ioprio_set02 | 28 | kernel_feature_gate | 0 | ioprio | ioprio/ioprio_set02.c |  |
| 32 | ioprio_set03 | 28 | kernel_feature_gate | 0 | ioprio | ioprio/ioprio_set03.c |  |
| 33 | kcmp01 | 28 | kernel_feature_gate | 0 | kcmp01 | kcmp/kcmp01.c |  |
| 34 | keyctl01 | 28 | kernel_feature_gate | 0 | keyctl | keyctl/keyctl01.c |  |
| 35 | openat202 | 28 | kernel_feature_gate | 0 | openat202 | openat2/openat202.c |  |
| 36 | pidfd_open01 | 28 | kernel_feature_gate | 0 | pidfd | pidfd_open/pidfd_open01.c |  |
| 37 | pidfd_send_signal01 | 28 | kernel_feature_gate | 0 | pidfd | pidfd_send_signal/pidfd_send_signal01.c |  |
| 38 | rt_sigqueueinfo01 | 28 | kernel_feature_gate | 0 | rt | rt_sigqueueinfo/rt_sigqueueinfo01.c | TST_TEST_TCONF( |
| 39 | sched_setparam05 | 28 | kernel_feature_gate | 0 | sched | sched_setparam/sched_setparam05.c |  |
| 40 | semop03 | 28 | kernel_feature_gate | 0 | ipc_sem | ipc/semop/semop03.c |  |

## 当前设置下不可能通过或不值得优先追的条目
| class | records |
| --- | --- |
| current_setup_impossible | 447 |
| arch_or_kernel_feature_missing | 109 |

| case | class | musl | glibc | diagnostic |
| --- | --- | --- | --- | --- |
| process_vm01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11381: process_vm01.c:83: TCONF: syscall(271) __NR_process_vm_writev not supported on your arch |
| semctl01 | arch_or_kernel_feature_missing | TPASS=0, rc=2, zero_tpass_failure | TPASS=0, rc=2, zero_tpass_failure | L15366: semctl01.c:269: TBROK: semget(0, 10, 780) failed: ENOSYS (38) |
| statx01 | arch_or_kernel_feature_missing | TPASS=10, rc=0, pass_with_warn_conf | TPASS=10, rc=0, pass_with_warn_conf | L19053: statx01.c:105: TPASS: statx(AT_FDCWD, test_file, 0, 0, &buff) \| L19054: statx01.c:112: TPASS: stx_uid(0) is correct \| L... |
| semctl07 | arch_or_kernel_feature_missing | TPASS=0, rc=2, zero_tpass_failure | - | L15446: semctl07.c:138: TBROK: semget(1627455500, 1, 380) failed: ENOSYS (38) |
| setreuid01_16 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L17024: /code/ltp-full-20240524/testcases/kernel/syscalls/setreuid/../utils/compat_tst_16.h:124: TCONF: 16-bit version of setre... |
| io_uring01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6253: tst_sys_conf.c:66: TINFO: Path not found: /proc/sys/kernel/io_uring_disabled: ENOENT (2) \| L6258: ../../../../include/la... |
| mq_notify03 | arch_or_kernel_feature_missing | TPASS=0, rc=2, zero_tpass_failure | TPASS=0, rc=2, zero_tpass_failure | L8572: mq_notify03.c:80: TBROK: mq_open(/ltp_mq_notify03,194,0600,0x7fffffb90) failed: ENOSYS (38) |
| prctl07 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11055: prctl07.c:168: TCONF: kernel doesn't support PR_CAP_AMBIENT |
| prctl10 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11099: tst_test.c:1201: TCONF: This arch 'unknown' is not supported for test! |
| bpf_map01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L1236: ../../../../include/lapi/bpf.h:623: TCONF: syscall(280) __NR_bpf not supported on your arch |
| madvise09 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L7417: madvise09.c:308: TCONF: '/sys/fs/cgroup/memory/' not present, CONFIG_MEMCG missing? |
| pidfd_getfd02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10085: ../../../../include/lapi/pidfd.h:38: TCONF: syscall(434) __NR_pidfd_open not supported on your arch |
| pidfd_open04 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10141: ../../../../include/lapi/pidfd.h:38: TCONF: syscall(434) __NR_pidfd_open not supported on your arch |
| pkey01 | arch_or_kernel_feature_missing,current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10673: pkey.h:27: TCONF: syscall(289) __NR_pkey_alloc not supported on your arch |
| getgroups01 | arch_or_kernel_feature_missing | TPASS=3, rc=0, pass_with_warn_conf | TPASS=3, rc=0, pass_with_warn_conf | L4551: getgroups01 1 TPASS : getgroups failed as expected with EINVAL \| L4552: getgroups01 2 TPASS : getgroups did not modify t... |
| getgroups01_16 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L4557: getgroups01_16 1 TCONF : /code/ltp-full-20240524/testcases/kernel/syscalls/getgroups/../utils/compat_16.h:82: 16-bit ver... |
| inotify12 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6133: inotify12.c:75: TCONF: syscall(26) __NR_inotify_init1 not supported on your arch |
| inotify_init1_01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6147: inotify_init1_01.c:26: TCONF: syscall(26) __NR_inotify_init1 not supported on your arch |
| inotify_init1_02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6161: inotify_init1_02.c:26: TCONF: syscall(26) __NR_inotify_init1 not supported on your arch |
| process_vm_readv03 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11409: process_vm_readv03.c:142: TCONF: syscall(270) __NR_process_vm_readv not supported on your arch |
| process_vm_writev02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11423: process_vm_writev02.c:75: TCONF: syscall(271) __NR_process_vm_writev not supported on your arch |
| request_key03 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L13242: ../../../../include/lapi/keyctl.h:54: TCONF: syscall(219) __NR_keyctl not supported on your arch |
| sched_setparam02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L14993: ../../../../include/tst_sched.h:43: TCONF: sched_setparam not supported \| L14996: ../../../../include/tst_sched.h:23: T... |
| sched_setparam03 | current_setup_impossible | TPASS=0, rc=33, zero_tpass_failure | TPASS=0, rc=33, zero_tpass_failure | L15011: ../../../../include/tst_sched.h:43: TCONF: sched_setparam not supported \| L15012: sched_setparam03.c:41: TFAIL: got pri... |
| setns02 | current_setup_impossible | TPASS=0, rc=36, config_skip | TPASS=0, rc=36, config_skip | L16514: setns02.c:154: TCONF: syscall(268) __NR_setns not supported on your arch \| L16515: setns02.c:173: TWARN: close(0) faile... |
| signalfd01 | arch_or_kernel_feature_missing | TPASS=0, rc=1, zero_tpass_failure | TPASS=0, rc=1, zero_tpass_failure | L18152: signalfd01 1 TFAIL : signalfd01.c:108: signalfd() Failed, errno=38 : Function not implemented |
| arch_prctl01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L1045: tst_test.c:1201: TCONF: This arch 'unknown' is not supported for test! |
| keyctl05 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6748: ../../../../include/lapi/keyctl.h:54: TCONF: syscall(219) __NR_keyctl not supported on your arch |
| keyctl07 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L6777: ../../../../include/lapi/keyctl.h:38: TCONF: syscall(218) __NR_request_key not supported on your arch \| L6778: keyctl07.... |
| madvise06 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L7371: tst_cgroup.c:706: TINFO: Mounted V2 CGroups on /tmp/cgroup_unified \| L7372: tst_cgroup.c:764: TINFO: Mounted V1 memory C... |
| mq_notify01 | arch_or_kernel_feature_missing | TPASS=0, rc=2, zero_tpass_failure | TPASS=0, rc=2, zero_tpass_failure | L8543: /code/ltp-full-20240524/testcases/kernel/syscalls/mq_notify/../utils/mq.h:48: TBROK: mq_open(/test_mqueue,194,0700,0) fa... |
| msgstress01 | current_setup_impossible | TPASS=0, rc=2, zero_tpass_failure | TPASS=0, rc=2, zero_tpass_failure | L8981: tst_pid.c:84: TINFO: Cannot read session user limits from '/sys/fs/cgroup/user.slice/user-0.slice/pids.max' \| L8982: tst... |
| pidfd_getfd01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10071: ../../../../include/lapi/pidfd.h:38: TCONF: syscall(434) __NR_pidfd_open not supported on your arch |
| pidfd_open03 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10127: ../../../../include/lapi/pidfd.h:38: TCONF: syscall(434) __NR_pidfd_open not supported on your arch |
| pidfd_send_signal03 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L10183: ../../../../include/lapi/pidfd.h:24: TCONF: syscall(424) __NR_pidfd_send_signal not supported on your arch |
| process_vm_readv02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L11395: process_vm_readv02.c:67: TCONF: syscall(270) __NR_process_vm_readv not supported on your arch |
| readahead01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L12760: readahead01.c:79: TCONF: syscall(213) __NR_readahead not supported on your arch |
| select03 | arch_or_kernel_feature_missing | TPASS=16, rc=0, pass_with_warn_conf | TPASS=16, rc=0, pass_with_warn_conf | L15291: select03.c:65: TPASS: Negative nfds: select() failed as expected: EINVAL (22) \| L15292: select03.c:65: TPASS: Invalid r... |
| setresuid04_16 | current_setup_impossible | TPASS=0, rc=32, config_skip | - | L16961: /code/ltp-full-20240524/testcases/kernel/syscalls/setresuid/../utils/compat_tst_16.h:133: TCONF: 16-bit version of setr... |
| setreuid07_16 | current_setup_impossible | TPASS=0, rc=32, config_skip | - | L17230: /code/ltp-full-20240524/testcases/kernel/syscalls/setreuid/../utils/compat_tst_16.h:124: TCONF: 16-bit version of setre... |
| shmctl04 | current_setup_impossible | TPASS=0, rc=32, config_skip | - | L17748: shmctl04.c:168: TCONF: kernel doesn't support SHM_STAT_ANY |
| timer_gettime01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L20109: timer_gettime01.c:40: TCONF: syscall(107) __NR_timer_create not supported on your arch |
| msgctl06 | current_setup_impossible | TPASS=0, rc=32, config_skip | - | L8775: msgctl06.c:156: TCONF: kernel doesn't support MSG_STAT_ANY |
| add_key01 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L717: ../../../../include/lapi/keyctl.h:29: TCONF: syscall(217) __NR_add_key not supported on your arch |
| add_key04 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L760: ../../../../include/lapi/keyctl.h:54: TCONF: syscall(219) __NR_keyctl not supported on your arch |
| bind04 | arch_or_kernel_feature_missing | TPASS=10, rc=0, pass_with_warn_conf | TPASS=10, rc=0, pass_with_warn_conf | L1161: bind04.c:149: TPASS: Communication successful \| L1163: bind04.c:149: TPASS: Communication successful \| L1165: bind04.c:1... |
| bind05 | arch_or_kernel_feature_missing | TPASS=8, rc=0, pass_with_warn_conf | TPASS=8, rc=0, pass_with_warn_conf | L1196: bind05.c:167: TPASS: Communication successful \| L1198: bind05.c:167: TPASS: Communication successful \| L1200: bind05.c:1... |
| bpf_prog02 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L1269: ../../../../include/lapi/bpf.h:623: TCONF: syscall(280) __NR_bpf not supported on your arch |
| getcwd04 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L4288: tst_test.c:1242: TCONF: Test needs at least 2 CPUs online |
| getuid03_16 | current_setup_impossible | TPASS=0, rc=32, config_skip | TPASS=0, rc=32, config_skip | L5431: /code/ltp-full-20240524/testcases/kernel/syscalls/getuid/../utils/compat_tst_16.h:89: TCONF: 16-bit version of getuid() ... |

## 可能高效的得分点来源
排序偏向 partial pass、musl/glibc 同时受影响、源码 TPASS 点较多、且不属于应排除或当前配置不可能通过的 case。
| rank | case | ROI | expected TPASS | observed TPASS | gain est. | recommendation | source |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | semctl07 | 56 | 16 | 0 | 32 | arch_or_kernel_feature_missing,real_semantic_bug | ipc/semctl/semctl07.c |
| 2 | getrusage03 | 53 | 15 | 0 | 30 | real_semantic_bug | getrusage/getrusage03.c |
| 3 | semctl01 | 50 | 14 | 0 | 28 | arch_or_kernel_feature_missing | ipc/semctl/semctl01.c |
| 4 | mount03 | 47 | 13 | 0 | 26 | real_semantic_bug | mount/mount03.c |
| 5 | times03 | 46 | 14 | 14 | 14 | real_semantic_bug | times/times03.c |
| 6 | mount07 | 41 | 11 | 0 | 22 | real_semantic_bug | mount/mount07.c |
| 7 | prctl04 | 35 | 9 | 0 | 18 | real_semantic_bug | prctl/prctl04.c |
| 8 | statx08 | 35 | 9 | 0 | 18 | real_semantic_bug | statx/statx08.c |
| 9 | openat02 | 34 | 6 | 2 | 10 | real_semantic_bug | openat/openat02.c |
| 10 | add_key05 | 32 | 6 | 0 | 12 | easy_env_fix | add_key/add_key05.c |
| 11 | rename01 | 32 | 8 | 0 | 16 | real_semantic_bug | rename/rename01.c |
| 12 | rename03 | 32 | 8 | 0 | 16 | real_semantic_bug | rename/rename03.c |
| 13 | llseek01 | 31 | 5 | 2 | 8 | real_semantic_bug | llseek/llseek01.c |
| 14 | statx01 | 31 | 11 | 20 | 2 | arch_or_kernel_feature_missing | statx/statx01.c |
| 15 | utime07 | 31 | 5 | 2 | 8 | real_semantic_bug | utime/utime07.c |
| 16 | ioctl_loop05 | 29 | 5 | 0 | 10 | easy_env_fix | ioctl/ioctl_loop05.c |
| 17 | mkdir09 | 29 | 7 | 0 | 14 | real_semantic_bug | mkdir/mkdir09.c |
| 18 | prctl03 | 29 | 7 | 0 | 14 | real_semantic_bug | prctl/prctl03.c |
| 19 | rmdir03 | 29 | 2 | 1 | 3 | easy_env_fix,real_semantic_bug | rmdir/rmdir03.c |
| 20 | mmap001 | 28 | 4 | 2 | 6 | real_semantic_bug | mmap/mmap001.c |
| 21 | sched_setscheduler03 | 28 | 2 | 2 | 2 | easy_env_fix | sched_setscheduler/sched_setscheduler03.c |
| 22 | shmctl07 | 28 | 4 | 2 | 6 | real_semantic_bug | ipc/shmctl/shmctl07.c |
| 23 | open10 | 27 | 3 | 6 | 0 | easy_env_fix | open/open10.c |
| 24 | setregid02 | 27 | 2 | 3 | 1 | easy_env_fix,real_semantic_bug | setregid/setregid02.c |
| 25 | fstat02 | 26 | 6 | 10 | 2 | real_semantic_bug | fstat/fstat02.c |
| 26 | fstat02_64 | 26 | 6 | 10 | 2 | real_semantic_bug | fstat/fstat02.c |
| 27 | io_setup02 | 26 | 6 | 0 | 12 | real_semantic_bug | io_setup/io_setup02.c |
| 28 | ioctl_loop02 | 26 | 4 | 0 | 8 | easy_env_fix | ioctl/ioctl_loop02.c |
| 29 | migrate_pages02 | 26 | 6 | 0 | 12 | real_semantic_bug | migrate_pages/migrate_pages02.c |
| 30 | mmap18 | 26 | 4 | 4 | 4 | real_semantic_bug | mmap/mmap18.c |

## 完全没通过的测试
| class | count | examples |
| --- | --- | --- |
| both_zero | 879 | acct01, acct02, acl1, add_ipv6addr, add_key01, add_key02, add_key03, add_key04, add_key05, af_alg01, af_alg02, af_alg03, af_alg04, af_alg05, af_alg06, af_alg07, aio-stress, aio01, aio02, aiocp, aiodio_append, aiodio_sparse, arch_prctl01, asapi_02, asapi_03, aslr01, autogroup01, bind06, block_dev, bpf_map01 |
| musl_only_zero | 14 | getcontext01, gethostid01, gethostname02, mallinfo01, mallinfo02, mallinfo2_01, mallopt01, profil01, pwritev201, pwritev201_64, pwritev202, pwritev202_64, shmt09, sigrelse01 |
| glibc_only_zero | 107 | access01, access02, access03, access04, adjtimex01, adjtimex02, adjtimex03, bind02, chmod03, chmod05, chmod06, chmod07, chown03, chown04, chroot01, chroot04, creat06, execve03, fchdir03, fchmod03, fchmod04, fchmod06, fchown03, getaddrinfo_01, getegid02, getegid02_16, getgid01, getgid03, getresgid02, getresgid03 |

## 复核点
| check | result |
| --- | --- |
| ltp-musl case count == 1523 | True |
| ltp-glibc case count == 1523 | True |
| musl TPASS == 3457 | True |
| glibc TPASS == 2962 | True |
| data is excluded | True |
| acct01 is configuration/feature blocked | True |
| accept03 keeps pass-with-TCONF detail | True |
| access01 musl/glibc states preserved | True |
| masked_by_tconf cases are flagged | True |
| masked gate classes assigned | True |
| masked release/defer rankings populated | True |
