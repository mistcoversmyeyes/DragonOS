# wait_test 修复结果

## 最终验证

- 内核编译：`make kernel` 通过
- 回归方式：`make qemu-nographic`
- 测试入口：DragonOS 启动后自动执行 `/opt/tests/gvisor/tests/wait_test`
- 最终结果：`63/63` 通过
- 最近一次整机复跑：`2026-03-25 01:55-01:57 CST`

## 修复批次

| 修复批次 | 关联用例 | 根因 | 关键改动 | 变更后通过的用例 | 是否引发回归 | 回归位置/现象 |
|---|---|---|---|---|---|---|
| 1 | `WaitTest.Wait4Rusage`, `WaitTest.WaitidRusage`, `Waiters/WaitAnyChildTest.WaitedChildRusage/{0,1}`, `Waiters/WaitAnyChildTest.IgnoredChildRusage/{0,1}` | wait 成功路径未回填 `rusage`；`RUSAGE_CHILDREN` 未累计；`SIGCHLD=SIG_IGN`/`SA_NOCLDWAIT` 语义不完整 | 补齐线程/线程组 CPU 时间统计、`get_rusage()`、wait 返回 `rusage`、waited-child 记账、auto-reap 语义，并修正 `SIGCHLD` 默认处置与显式 `SIG_IGN` 的建模区别 | 上述 6 个用例通过 | 中途出现过，最终已消除 | 曾出现基础 wait 用例广泛 `ECHILD`，根因是把 `SIGCHLD` 默认处置错误建模为显式 `SIG_IGN` |
| 2 | `WaitTest.TraceeWALL` | 缺少 `ptrace(PTRACE_TRACEME)`，tracee 未按 `__WALL` 语义匹配 | 增加最小 `PTRACE_TRACEME` 支持，并为 tracee 增加 `PTRACED` 标记 | `TraceeWALL` 通过 | 否 | 无 |
| 3 | `Waiters/WaitSpecificChildTest.AfterChildExecve/{0,1}` | `wait(pid)` 固定绑定旧 leader 的 PCB；`de_thread` 后 pid 已交换到 exec 线程 | `wait(pid)` 改为每次按 pid 重新解析当前 task，跟随 pid/TID 交换后的真实目标 | `AfterChildExecve/{0,1}` 通过 | 否 | 无 |

## 分阶段回归结果

| 阶段 | 对应提交内容 | 验证结果 |
|---|---|---|
| 阶段 1 | rusage/children-rusage/auto-reap/SIGCHLD 语义修复 | `wait_test` 剩余失败收敛到 `TraceeWALL`、`AfterChildExecve/{0,1}` |
| 阶段 2 | `PTRACE_TRACEME` 与 tracee wait 语义修复 | `wait_test` 仅剩 `AfterChildExecve/{0,1}` 失败 |
| 阶段 3 | `wait(pid)` 跟随 `de_thread` 后 pid 交换 | `wait_test` `63/63` 全通过 |

## 备注

- 回归测试期间仍可见一条 `process_cputime_ns fallback` 告警，但未导致 `wait_test` 失败。
- 测试 harness 使用当前镜像中的自启动入口，开机后直接执行 `wait_test`。
- 测试全部通过后，系统仍会输出 `Init process (pid=1) attempted to group_exit with code 0`；该问题发生在 `wait_test` 汇总成功之后，不构成本次 wait 语义修复的回归。
