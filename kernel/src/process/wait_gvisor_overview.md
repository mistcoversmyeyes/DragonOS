# 修复 WaitTest 测试（Overview）

## 一、测试范围理解

### 1.1 测试目标
- `wait4/waitid` 返回的 `rusage` 必须反映真实的子进程 CPU 消耗。
- 已被 wait 回收的子进程资源使用量必须累计到父进程 `RUSAGE_CHILDREN`。
- `SIGCHLD` 的默认处置、显式 `SIG_IGN` 与 `SA_NOCLDWAIT` 必须按 Linux 6.6 语义区分，不能错误触发 auto-reap。
- tracee（`PTRACE_TRACEME`）必须可通过 wait 家族接口按 traced-child 语义匹配。
- `wait(pid)` 在 `execve` 触发 `de_thread` 后，必须继续跟随 pid 交换后的真实目标。

### 1.2 关键测试用例
| 用例名称 | 验证行为 | 最终状态 |
|---|---|---|
| `WaitTest.Wait4Rusage` | `wait4(..., &rusage)` 返回子进程 CPU 时间 | PASS |
| `WaitTest.WaitidRusage` | `sys_waitid(..., &rusage)` 返回子进程 CPU 时间 | PASS |
| `Waiters/WaitAnyChildTest.WaitedChildRusage/{0,1}` | 已 wait 的子进程资源应累计到 `RUSAGE_CHILDREN` | PASS |
| `Waiters/WaitAnyChildTest.IgnoredChildRusage/{0,1}` | `SIGCHLD=SIG_IGN`/`SA_NOCLDWAIT` 下 auto-reap 后资源仍应累计 | PASS |
| `WaitTest.TraceeWALL` | `PTRACE_TRACEME` 后 wait 可按 traced-child 规则匹配子进程 | PASS |
| `Waiters/WaitSpecificChildTest.AfterChildExecve/{0,1}` | `wait(pid)` 可跟随 `de_thread` 后的 pid/TID 交换 | PASS |

---

## 二、内核子系统现状

### 2.1 相关子系统
- wait/exit 路径：`kernel/src/process/exit.rs`
- rusage 统计：`kernel/src/process/resource.rs`, `kernel/src/process/cputime.rs`
- 进程退出/auto-reap 判定：`kernel/src/process/mod.rs`
- 信号默认处置：`kernel/src/ipc/sighand.rs`
- ptrace syscall：`kernel/src/process/syscall/mod.rs`, `kernel/src/process/syscall/sys_ptrace.rs`
- mt-exec / `de_thread`：`kernel/src/process/exec.rs`, `kernel/src/process/exit.rs`

### 2.2 修复前的真实问题
- `wait4/waitid` 成功返回时没有把 `ret_rusage` 填回用户态结构。
- `get_rusage()` 只返回默认值，且没有 `RUSAGE_CHILDREN` 累计路径。
- 曾将 `SIGCHLD` 的“默认动作忽略”错误建模为显式 `SIG_IGN`，导致普通子进程被提前 auto-reap，扩散成 `ECHILD` 类回归。
- `SYS_PTRACE` 虽有号值，但没有实际 handler，`PTRACE_TRACEME` 直接失败。
- wait 的子进程匹配逻辑缺少 traced-child 特判。
- `wait(pid)` 在进入阻塞后固定绑定旧 PCB，`de_thread` 完成 pid 交换后会指向过期对象。

---

## 三、根因分析

| 测试点 | Linux 期望 | DragonOS 修复前实际 | 差距 |
|---|---|---|---|
| `wait4/waitid -> rusage` 填充 | 命中 wait 事件时返回对应子进程 `RUSAGE_BOTH` | 仅保存 `ret_rusage` 指针，不写回实际数据 | wait 事件上报链缺失（2 个测试） |
| `RUSAGE_CHILDREN` | 父进程累计已 wait 子进程及其后代 CPU 时间 | 没有 waited-child 累计记账 | 子进程资源记账缺失（4 个测试共享根因） |
| `SIGCHLD` 默认处置 vs 显式 `SIG_IGN` | 默认处置仍可 wait；仅显式 `SIG_IGN`/`SA_NOCLDWAIT` 才 auto-reap | 默认处置被错误等同于显式忽略 | 引入了广泛 `ECHILD` 回归风险 |
| `ptrace(PTRACE_TRACEME)` 可用性 | syscall 存在且 tracee 进入 traced 状态 | 直接 `ENOSYS`/unsupported | ptrace 入口缺失（1 个测试） |
| traced child 与 wait 匹配 | traced child 按 `__WALL` 语义处理 | 仅按 `exit_signal` 进行 clone/non-clone 分类 | traced-child 匹配语义缺失（1 个测试） |
| `wait(pid)` 与 `de_thread` | 通过 pid 重新定位当前 task，pid 交换后仍可继续等待 | 阻塞前绑定旧 PCB，交换后可能落到旧 leader | pid 解析时机错误（2 个测试） |

---

## 四、修复方案与实际落地

### 4.1 关键改动
| 文件 | 实际改动 | 原因 |
|---|---|---|
| `kernel/src/process/resource.rs` | 实现 `get_rusage()`，支持 `RUsageSelf/RUsageChildren/RUsageBoth/RusageThread` | 补齐真实 rusage 数据源 |
| `kernel/src/process/cputime.rs` | 提供线程、线程组级 CPU 时间统计接口 | 为 `get_rusage()` 和 waited-child 记账提供基础数据 |
| `kernel/src/process/exit.rs` | wait 成功路径填充 `ret_rusage`；回收时累计 waited-child rusage；`wait(pid)` 每轮重新解析 pid；traced child 按 `__WALL` 匹配 | 覆盖 wait 结果写回、子进程记账和 `de_thread` 场景 |
| `kernel/src/process/mod.rs` | 增加 waited-child 资源累计与 auto-reap 判定逻辑 | 修复 `RUSAGE_CHILDREN` 与 `SIGCHLD` 语义 |
| `kernel/src/ipc/sighand.rs` | 默认信号处置恢复为 `SIG_DFL`，不把默认 `SIGCHLD` 视为显式 `SIG_IGN` | 对齐 Linux 6.6 信号默认语义 |
| `kernel/src/process/syscall/mod.rs` + `kernel/src/process/syscall/sys_ptrace.rs` | 注册并实现最小 `PTRACE_TRACEME` 支持 | 满足 `TraceeWALL` 基线 |

### 4.2 依赖关系
1. 先修正 `SIGCHLD` 默认处置建模，否则 wait 基础语义会被回归打穿。
2. 再补齐 `get_rusage()`、wait 写回和 waited-child 记账，使 `wait4/waitid/RUSAGE_CHILDREN` 同时收敛。
3. 之后补最小 `PTRACE_TRACEME` 与 traced-child 匹配逻辑，独立解决 `TraceeWALL`。
4. 最后修 `wait(pid)` 在 `de_thread` 后的 pid 重解析，解决 `AfterChildExecve`。

### 4.3 验证结论
- `make kernel` 通过。
- `make qemu-nographic` 启动后自动运行 `/opt/tests/gvisor/tests/wait_test`。
- 最终结果：`63/63` 通过。
- 另有一条测试结束后的 `Init process (pid=1) attempted to group_exit with code 0` 日志，发生在 `wait_test` 汇总成功之后，不属于本轮 wait 语义回归。
