# 变更文件清单

## 修改的文件 (8个)

```
 docs/HWTDocs/run.md         |   4 +-    # 运行文档小更新
 os/src/fs/mod.rs            |  36 ++-   # File trait 新增 unix socket 方法
 os/src/fs/vfs/procfs.rs     | 129 +++   # procfs 新增文件节点
 os/src/net/mod.rs           |   1 +     # 引入 unix_socket 模块
 os/src/net/syscall.rs       | 163 +++   # AF_UNIX bind/socket 支持
 os/src/syscall/mod.rs       |  39 ++-   # 新 syscall 编号注册
 os/src/syscall/process.rs   | 617 +++   # 新 syscall 实现
 os/src/task/process.rs      |  19 +    # PCB 新增 saved_uid/gid/name
```

## 新增的文件 (1个)

```
 os/src/net/unix_socket.rs            # AF_UNIX 域套接字实现
```

## 对测试分数的影响

### 直接提分项
- cgroup_core01: TBROK(0分) -> TCONF(skipped=1) — 不再报错
- cgroup_core02: TBROK(0分) -> TCONF(skipped=1) — 不再报错
- cgroup_core03: TBROK(0分) -> TCONF(skipped=1) — 不再报错
- bind01: 0/7 -> 5/7 (+5) — AF_UNIX socket 创建和部分 bind 场景通过

### 间接提分项 (未测试确认)
- accept03: 7/8 -> 可能 8/8 — 取决于 AF_UNIX accept 是否被该测试覆盖
- 各种需要 /proc/self/status 的测试 — 不再 TBROK

## 各 bind 测试分析 (allrv00 结果)

| 测试 | 结果 | 说明 |
|------|------|------|
| bind01 | 5pass/2fail | 2个失败: AF_UNIX 地址相关 case (需要完善 unix bind 细节) |
| bind02 | 1pass | inet bind 正常 |
| bind03 | 2pass | 已全部通过 |
| bind04 | 0pass | 需要 AF_UNIX connect 支持 |
| bind05 | 0pass/1fail | 需要 AF_UNIX socketpair 或更多功能 |
| bind06 | 0pass | 需要 IPV6 支持 |
