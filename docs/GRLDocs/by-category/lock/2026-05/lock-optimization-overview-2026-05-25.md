# 锁拆分与收敛总览（2026-05-25）

## 背景

这一组文档记录了 rCore-lab 从 PCB/TCB 热点锁，到调度队列、Futex、Timer、VFS 文件层，再到块设备 cache 的一系列锁拆分与访问点收敛工作。它们本质上属于同一条“把粗锁改成更贴近语义的短锁，并尽量把慢路径移到锁外”的演进链路，因此单独归入 `lock` 类别更合适。

## 主线脉络

- Stage3：TCB 物理拆分与访问点强收敛
- Stage7：FS/VFS 文件层热点锁拆分与访问点收敛
- Stage8：调度队列、PID2PCB、Timer、Futex、Net 全局锁拆分
- Stage9：VFS 短锁收敛与文件 I/O 热点微调
- Stage10：块设备 Cache 锁重构与 I/O 脱锁
- PCB/TCB 锁重构：作为更早一层的 PCB/TCB 锁语义整理与 API 收敛说明

## 阅读顺序

如果想快速理解这条线，建议按下面顺序读：

1. [TCB 物理拆分与访问点强收敛](tcb-lock-split-stage3-report-2026-05-24.md)
2. [FS/VFS 文件层热点锁拆分与访问点收敛报告（stage7）](fs-vfs-hot-lock-split-stage7-report-2026-05-24.md)
3. [调度队列/PID2PCB/Timer/Futex/Net 全局锁拆分与访问点收敛报告（stage8）](sched-pid2pcb-timer-futex-net-lock-split-stage8-report-2026-05-24.md)
4. [VFS 短锁收敛与文件 I/O 热点微调报告（stage9）](vfs-short-lock-stage9-report-2026-05-25.md)
5. [第 10 阶段：块设备 Cache 锁重构与 I/O 脱锁报告](block-cache-lock-split-stage10-report-2026-05-25.md)
6. [PCB/TCB 中断安全锁拆分重构说明](pcb-tcb-lock-refactor-2026-05-24.md)

## 共同结论

这些改动都围绕同一个原则展开：

- 将粗粒度独占锁改成更小的、按访问语义划分的锁。
- 把阻塞、I/O、页表遍历、用户缓冲区访问等慢路径移出锁内。
- 保持当前单核 + 中断屏蔽模型，不提前引入 SMP 级锁复杂度。
- 通过 helper 收敛访问入口，减少调用点对内部字段布局的依赖。

## 备注

后续如果锁相关工作继续扩展，可以继续把新的阶段报告放在这个目录下，并按日期继续细分时间桶。
