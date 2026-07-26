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

---

## 五、2026-07-27 复验（master `ae28b352`，含 PR #2156）

### 5.1 结论
- 上游 PR #2156（"serialize non-leader exec handoff"）重写了 wait/exit/signal/ptrace 路径，**已完整覆盖本报告第四节的全部 6 个语义点**（rusage 写回、RUSAGE_CHILDREN 记账、SIGCHLD 默认处置区分、PTRACE_TRACEME、traced-child 匹配、wait(pid) 每轮按 pid 重解析）。本分支原两个内核修复提交退役，保存于 `origin/fix/wait_test`（`c7826515`、`a4fab1fe`）。
- 纯 master 实测 61/63；修复下述两个非 wait 语义问题后 **63/63 稳定通过**。

### 5.2 残余失败 1：`AfterChildExecve/{0,1}` —— rootfs 缺 `/bin/true`
- 现象：wait 得到 status 512（WEXITSTATUS=2），期望 0。
- 机制：测试在子进程 `clone(CLONE_THREAD|CLONE_VFORK)` 的线程里 `execve("/bin/true")`；rootfs 无该文件 → execve 返回 ENOENT → errno 经共享 TLS 传回 leader → `_exit(2)`。与 wait 语义无关。
- 修复：`user/apps/busybox/Makefile` install 目标增加 `cp $(bin) $(DADK_CURRENT_BUILD_DIR)/true`（与 `sh` 同模式；gvisor 白名单内仅 `wait_test` 引用 `/bin/true`）。

### 5.3 残余失败 2：`ForkBlock/0` 偶发早返 —— nanosleep 时基漂移
- 现象：一次全量运行中父进程测得 4.648s < 5s；单独 `--gtest_repeat=6` 12/12 通过，非系统性。
- 根因：`nanosleep` 纯按 LAPIC tick 计数到期（HZ=250，5s=1250 tick，`kernel/src/time/sleep.rs`），而 `clock_gettime` 走 kvm-clock 时钟源。WSL2 嵌套 KVM 下 vCPU steal 使 tick 积压后突发补注入，tick 域短暂快于真实时间，1250 tick 可在 4.648s 内数满；原 `sleep.rs` 正常到期分支不校验 deadline 直接返回，构成 POSIX 违规（无信号早返）。串口 clocksource watchdog 日志（`cs_dev_nsec=508000000`=127×4ms tick vs `wd_dev_nsec=579223209`，jiffies 被标 unstable）是同一 tick 抖动的反相印证。
- 修复：`nanosleep` 改为 deadline 循环——每轮 timer 到期后用 `monotonic_now()` 对照 `sleep_deadline`，不足按余量续睡；信号中断且 deadline 未到才返回 `ERESTARTSYS`。
- 长线方向（未做）：TSC fallback 校准窗缺陷（PIT 路径恒失败）与 nanosleep 迁移到 clocksource 基 hrtimer 语义，见时间子系统，与本轮无耦合。

### 5.4 复验数据
| 轮次 | 内核 | 条件 | 结果 |
|---|---|---|---|
| A | master | 无 /bin/true | 61/63（AfterChildExecve ×2 失败，exit code 2） |
| B | master | 补 /bin/true | 62/63（ForkBlock/0 偶发 4.648s）；ForkBlock 单测 ×6 轮 12/12 |
| C | master | 补 /bin/true | 63/63 |
| D | master+nanosleep 修复 | 补 /bin/true | ForkBlock ×6 轮 12/12（5011–5046ms，离散度收敛）；全量 **63/63** |
