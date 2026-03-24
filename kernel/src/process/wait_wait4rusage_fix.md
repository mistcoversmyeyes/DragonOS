# Bug Fix: WaitTest.Wait4Rusage

## 根因分析
`Wait4Rusage` 的直接失败点是 wait 成功返回后没有填充 `ret_rusage`，同时底层 `get_rusage()` 也还没有接入真实 CPU 统计。  
进一步分析后确认，这不是单个用例独有问题，而是与 `WaitidRusage`、`WaitedChildRusage`、`IgnoredChildRusage` 共享同一组 rusage 根因。

## 测试行为

**gVisor 测试**: `../gvisor/test/syscalls/linux/wait.cc:824`

测试逻辑：
- `ForkSpinAndExit(21, 3)` 创建子进程并自旋约 3 秒 CPU 时间后退出。
- 父进程调用 `wait4(child, &status, 0, &rusage)`。
- 期望 `RusageCpuTime(rusage) >= 3s`。

```c
// ../gvisor/test/syscalls/linux/wait.cc:824
TEST(WaitTest, Wait4Rusage) {
  pid_t child;
  constexpr absl::Duration kSpin = absl::Seconds(3);
  ASSERT_THAT(child = ForkSpinAndExit(21, absl::ToInt64Seconds(kSpin)),
              SyscallSucceeds());

  int status;
  struct rusage rusage = {};
  ASSERT_THAT(Wait4(child, &status, 0, &rusage),
              SyscallSucceedsWithValue(child));
  EXPECT_GE(RusageCpuTime(rusage), kSpin);
}
```

**实际错误**: `RusageCpuTime(rusage) == 0`，期望 `>= 3s`。

## 参考实现（Linux）

**文件**: `../linux/kernel/exit.c`

```c
// 1191
if (wo->wo_rusage)
    getrusage(p, RUSAGE_BOTH, wo->wo_rusage);
```

关键语义：
- `wait_task_zombie()` 成功回收子进程时填充 `wo_rusage`。
- 对 `wait4` 用户态而言，`ru_utime/ru_stime` 反映子任务累计 CPU 时间。

## DragonOS 当前实现

**文件**: `kernel/src/process/syscall/sys_wait4.rs`

```rust
// 59-66
let mut tmp_rusage = if rusage.is_null() { None } else { Some(RUsage::default()) };
let r = kernel_wait4(pid, wstatus_buf, options, tmp_rusage.as_mut())?;
```

**文件**: `kernel/src/process/exit.rs`

```rust
// 125-129
kwo.options.insert(WaitOption::WEXITED);
kwo.ret_rusage = rusage_buf;
let r = do_wait(&mut kwo)?;
```

**文件**: `kernel/src/process/resource.rs`

```rust
// 146-149
pub fn get_rusage(&self, _who: RUsageWho) -> Option<RUsage> {
    let rusage = RUsage::default();
    Some(rusage)
}
```

修复前问题：
- `ret_rusage` 未在 `do_waitpid` 命中事件时写入。
- `get_rusage` 仅返回默认值，不读 `utime/stime`。

## 实际修复

1. 在 `kernel/src/process/exit.rs` 中新增 wait 成功路径的 `fill_wait_rusage()`，由 `wait4/waitid` 统一复用。
2. 在 `kernel/src/process/resource.rs` 中实现 `ProcessControlBlock::get_rusage()`，使用真实的线程/线程组 CPU 时间填充 `ru_utime/ru_stime`。
3. 在 `kernel/src/process/cputime.rs` 中补齐 `process_utime_stime_ns()`/相关统计基础，使 `RUsageBoth` 能返回真实值。

## 变更范围

- **文件**: `kernel/src/process/exit.rs`, `kernel/src/process/resource.rs`, `kernel/src/process/cputime.rs`
- **风险**: 影响 `getrusage(2)` 与 wait 家族资源统计，需联合回归 `wait4/waitid/RUSAGE_CHILDREN`
- **依赖**: 与 `WaitidRusage` 共享同一实现；与 `WaitedChildRusage`/`IgnoredChildRusage` 共用同一统计基础

## 验证结果

- `WaitTest.Wait4Rusage` 通过。
- `WaitTest.WaitidRusage` 同步通过。
- 最终 `wait_test` 为 `63/63` 通过。
