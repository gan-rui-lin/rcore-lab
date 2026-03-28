# lmbench pipe/signal 后续重构路线图（2026-03-24）

## 1. 背景

`lat_pipe` 卡死问题已通过在 `pipe` 阻塞循环中增加 signal escape hatch 临时收敛，但当前实现仍有结构性短板：

1. `File::read/write` 返回 `usize`，无法表达 `EINTR/ERESTART` 等中断语义。  
2. 阻塞路径中的“信号可中断”逻辑分散在多个子系统（pipe/futex/net/poll），容易再次出现行为不一致。  
3. 当前方案在部分场景中将“被信号打断”退化成“返回 0”，可能与 EOF/短读语义混淆。  

目标是把这次修复从“补丁”提升为“可复用的阻塞与信号处理框架”。

---

## 2. 重构目标与边界

### 2.1 目标

1. 明确并统一阻塞 syscall 的 `EINTR/ERESTART/SA_RESTART` 语义。  
2. 让 pipe/futex/net/poll 的阻塞等待都具备一致的信号逃逸机制。  
3. 降低未来“发了 SIGKILL 但任务仍像没死”的复发概率。  

### 2.2 非目标（本轮不做）

1. 不在本轮直接切换到全量 async runtime。  
2. 不一次性重写全部 VFS/网络代码路径。  

---

## 3. 分阶段路线

## Phase A（短期，1~2 天）：语义止血

### A1. 统一 pipe 被信号打断行为

建议将 `pipe` 阻塞等待中的“检测到未屏蔽 signal”从“返回 0/partial”改为可区分的中断返回（优先 `ERESTART`，由后续信号框架决定转重启还是 `EINTR`）。

涉及文件（当前仓库）：

- `os/src/fs/pipe.rs`
- `os/src/syscall/fs.rs`

### A2. 保留 `SIGCHLD` 特殊策略

参考对照仓库经验，阻塞读写建议继续忽略“仅 `SIGCHLD` pending”的中断触发，避免 wait 子进程场景导致 pipe 误打断。

---

## Phase B（中期，3~5 天）：接口升级

### B1. 升级 `File` 读写接口

将：

- `fn read(&self, buf: UserBuffer) -> usize`
- `fn write(&self, buf: UserBuffer) -> usize`

升级为：

- `fn read(&self, buf: UserBuffer) -> Result<usize, SysErrNo>`
- `fn write(&self, buf: UserBuffer) -> Result<usize, SysErrNo>`

核心收益：

1. 内核内部能直接表达 `EINTR/ERESTART/EPIPE/EAGAIN`。  
2. 减少“特殊文件靠 side-channel 传错误”的隐式行为。  
3. 后续对 poll/splice/sendfile 的一致化更容易。  

### B2. syscall 入口统一错误翻译

在 `sys_read/sys_write/readv/writev/splice/sendfile` 处统一把 `Result` 翻译为用户态 errno，避免各路径各自“手写负值协议”。

---

## Phase C（中期，3~4 天）：阻塞等待抽象

提炼统一 helper（示例）：

- `block_until_ready_or_signal(...)`

该 helper 负责：

1. 检查 pending&mask（含可选 `ignore_sigchld`）；  
2. 决定返回 `ERESTART/EINTR`；  
3. 统一 `suspend_current_and_run_next()` 前后的状态处理。  

首批接入：

1. `os/src/fs/pipe.rs`
2. `os/src/task/futex.rs`
3. `os/src/net/syscall.rs`
4. `sys_ppoll/sys_pselect` 等轮询等待路径

---

## Phase D（长期，可选）：事件驱动化评估

参考 `oskernel2025-chronix-retest` 的经验，可评估从“同步循环 + yield”迁移到“Pending + waker”的事件驱动模型（尤其是 pipe/poll/futex 热路径）。

建议先做 PoC，不直接全量替换：

1. 先选 pipe 子系统单独实验。  
2. 在不改变 syscall 外部语义前提下验证性能/稳定性。  
3. 验证通过后再扩展到 poll/futex/net。  

---

## 4. 验收标准（每阶段必须满足）

1. `musl-lmbench`：`lat_sig`、`lat_pipe` 在设定超时内稳定结束。  
2. 强制 `SIGKILL` 压测：不出现“任务不可回收”或 zombie 长驻。  
3. `ppoll/pselect/futex_wait` 被信号打断时语义一致（`EINTR/ERESTART` 预期可解释）。  
4. 进程退出后 fd 关闭行为可观测（pipe 对端正确收到 `HUP/ERR`）。  

---

## 5. 风险与回滚策略

1. `File` trait 签名升级影响面大（VFS、设备文件、socket、pipe 全覆盖）。  
2. 推荐“先加兼容层再切换调用方”，避免一次性大爆炸。  
3. 每阶段独立提交，任一阶段出现回归可单独回滚。  

---

## 6. 推荐执行顺序

1. 先做 Phase A（立即降低语义歧义风险）。  
2. 再做 Phase B（把错误语义正式化）。  
3. 再做 Phase C（消除重复逻辑与漏改风险）。  
4. 最后视收益决定是否进入 Phase D（架构级升级）。  

该顺序能在最小风险下，逐步把“卡死修复”演进为“可维护的阻塞/信号框架”。
