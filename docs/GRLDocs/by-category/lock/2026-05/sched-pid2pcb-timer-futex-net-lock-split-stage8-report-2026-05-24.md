# 调度队列/PID2PCB/Timer/Futex/Net 全局锁拆分与访问点收敛报告（stage8）

## 摘要

本次 stage8 聚焦内核并发/阻塞唤醒路径的热点全局锁：调度就绪队列、PID2PCB、timer、futex 与 net stack。参照 stage7 的“访问点收敛 + 锁粒度贴近语义”方法，本次将多个全局 UPIntrFreeCell / spin::Mutex 重构为 UPIntrRwLock / UPIntrMutex，并通过 helper 收敛访问入口，缩短临界区，同时保证阻塞前释放锁、唤醒路径不持锁跨调度。

整体仍保持单核 + 中断屏蔽并发模型，不引入 SMP 级 spinlock/sleep lock，目标是：
- 让读多写少路径拿读锁或短持锁；
- 把唤醒动作与阻塞动作移出全局锁范围；
- 为未来进一步锁分层/统计提供统一入口。

## 背景问题

在 stage8 之前，多个全局结构呈现以下典型问题：

1. **调度就绪队列与 PID2PCB 均为 UPIntrFreeCell**
   - 读路径（snapshot / len / 诊断）与写路径（入队/出队/插入/删除）共用独占访问。
   - 调用点容易在持锁状态下扩展工作量。

2. **Timer 全局堆在中断与 syscall 路径上共享**
   - check_timer 在持锁状态下直接唤醒任务，扩大临界区。

3. **Futex 使用全局 spin::Mutex**
   - futex_wake / requeue 在持锁状态下调用 wakeup_task，存在锁持有跨唤醒路径的风险。

4. **Net stack 全局锁过粗**
   - NET_STACK 以 UPIntrFreeCell 保护，所有 socket/syscall 访问都走独占锁。
   - 阻塞收发路径若持锁过久，容易阻塞其他网络操作。

一句话根因：多个热点全局状态以“独占锁 + 分散调用点”的形式存在，临界区过大，阻塞/唤醒边界不清晰。

## 改动范围

涉及文件：
- os/src/task/manager.rs
- os/src/timer.rs
- os/src/task/futex.rs
- os/src/net/mod.rs
- os/src/net/socket_file.rs
- os/src/net/syscall.rs

## 证据与定位路径（可复现）

```bash
cd rcore-lab
# 查看改动文件列表与 diff
git status

git diff -- os/src/task/manager.rs \
  os/src/timer.rs \
  os/src/task/futex.rs \
  os/src/net/mod.rs \
  os/src/net/socket_file.rs \
  os/src/net/syscall.rs

# 编译验证
make all
```

## 核心改动

### 1) 调度就绪队列：UPIntrRwLock + helper 收敛

改动前：
- TASK_MANAGER: UPIntrFreeCell<TaskManager>
- 读/写均走 exclusive_access

改动后：
- TASK_MANAGER 改为 UPIntrRwLock
- 新增 with_task_manager_read/write helper，统一访问入口
- ready_queue_snapshot / ready_queue_len 使用读锁
- add_task / remove_task / fetch_task 使用写锁

收益：
- 读路径不再被强制独占；
- 调用点收敛，避免扩张临界区。

### 2) PID2PCB：UPIntrRwLock + helper 收敛

改动前：
- PID2PCB: UPIntrFreeCell<BTreeMap<..>>
- 所有路径独占访问

改动后：
- PID2PCB 改为 UPIntrRwLock
- with_pid2pcb_read/write helper
- pid2process / snapshot / len 使用读锁
- insert/remove 使用写锁

收益：
- 读路径更轻量；
- 便于后续插入统计或调试逻辑。

### 3) Timer：UPIntrRwLock + 唤醒脱锁

改动前：
- TIMERS: UPIntrFreeCell<BinaryHeap>
- check_timer 持锁 pop + wakeup_task

改动后：
- TIMERS 改为 UPIntrRwLock
- with_timers_read/write helper
- check_timer 在锁内收集过期任务，锁外 wakeup_task

收益：
- 缩小 timer 临界区；
- 避免唤醒路径持锁。

### 4) Futex：UPIntrMutex + 唤醒脱锁

改动前：
- FUTEX_Q: spin::Mutex<BTreeMap<..>>
- futex_wake/requeue 持锁 wakeup_task

改动后：
- FUTEX_Q 改为 UPIntrMutex
- with_futex_q helper
- futex_wake/requeue 先收集待唤醒任务，锁外 wakeup_task
- futex_wait 保持“入队后 drop 锁再阻塞”的语义

收益：
- 减少锁持有跨唤醒路径的风险；
- 访问点统一，便于将来做桶化或统计。

### 5) Net stack：UPIntrRwLock + helper 收敛

改动前：
- NET_STACK: UPIntrFreeCell<Option<NetStack>>
- 各 socket/syscall 直接 exclusive_access

改动后：
- NET_STACK 改为 UPIntrRwLock
- 新增 with_net_stack_read/write/try_write helper
- poll_net / poll_net_if_available / poll_net_force 走 helper
- socket_file / syscall 统一改为 helper，并在阻塞前释放锁

说明：
- 由于 smoltcp socket set 需要可变访问，当前多数路径仍使用 write 锁。
- 访问入口收敛后，未来可逐步区分真正读路径。

## 收益总结

1. **锁语义更清晰**：读多写少路径使用读锁，写路径短持锁。
2. **阻塞/唤醒边界更安全**：futex 和 timer 的唤醒在锁外完成；socket 阻塞前不持 net 锁。
3. **访问入口收敛**：统一 helper 入口，为后续统计、调试或进一步拆分提供锚点。

## 已知边界与风险

1. **单核模型下 RwLock 的并行收益有限**：主要价值在语义清晰与临界区收敛。
2. **Net stack 仍以 write 锁为主**：smoltcp 的可变访问要求使读锁收益暂有限。
3. **锁顺序约定仍需文档化**：timer/pid2pcb、futex/task 管理之间的锁顺序需要后续明确，以免未来扩展引入死锁。

## 验证结果

- 编译验证：未执行。
- 建议验证：
  - make all
  - futex/timer/net 相关 syscall 回归
  - 阻塞读写 + SIGALRM/信号中断路径

## 后续建议

1. **补充锁顺序约定文档**：明确跨子系统的锁获取顺序。
2. **Futex 未来可考虑桶化**：在 helper 入口基础上做 shard 分桶，降低 contention。
3. **Net stack 读写路径进一步拆分**：识别真正只读路径，为 read 锁铺路。
