# 建议的 commit 方式

可以整体提交一次，也可以拆分成多个 commit。以下是两种方案:

## 方案一: 单次 commit (简洁)

```
feat(ltp): procfs 完善 + AF_UNIX 骨架 + 新 syscall，提分 307->314

- procfs: 新增 /proc/self/{mounts,mountinfo,status,cgroup,...}
  修复 cgroup_core01-03 TBROK
- net: AF_UNIX 域套接字创建 + bind (unix_socket.rs)
- syscall: rt_sigsuspend, capget/set, setresuid/getresuid,
  setresgid/getresgid, setreuid/setregid, prctl, adjtimex
- accept: 支持 SOCK_CLOEXEC/SOCK_NONBLOCK flags
- process: PCB 新增 saved_uid/gid/name 字段
```

## 方案二: 拆分 commit (清晰)

```
# commit 1
feat(procfs): 新增 /proc/self/{mounts,mountinfo,status,cgroup} 等文件

修复 cgroup_core01-03 因 /proc/self/mounts 不存在而 TBROK 的问题。
新增 /proc/cgroups, /proc/filesystems, /proc/mountinfo 根级文件。

# commit 2
feat(syscall): 新增 rt_sigsuspend/capget/capset/setresuid 等系统调用

支持 LTP 测试需要的 UID/GID 管理和信号处理系统调用。
PCB 新增 saved_uid/saved_gid/name 字段。

# commit 3
feat(net): AF_UNIX 域套接字骨架实现

新建 unix_socket.rs，支持 socket(AF_UNIX, ...) 创建和 bind。
sys_bind 支持 AF_UNIX 地址解析和 EADDRINUSE 检测。
sys_accept 新增 flags 参数 (SOCK_CLOEXEC/SOCK_NONBLOCK)。
```

推荐方案二，便于后续 git bisect 和代码审查。
