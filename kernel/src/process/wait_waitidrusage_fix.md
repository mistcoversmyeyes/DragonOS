# Bug Fix: WaitTest.WaitidRusage

## 根因分析
与 `Wait4Rusage` 同根因：`waitid` 的第 5 个参数虽然传到了内核，但成功等待后没有任何路径填充 `ret_rusage`，同时 `get_rusage()` 修复前仍返回默认值。  
此外，实测表明这一批问题还会连带影响 `RUSAGE_CHILDREN` 相关用例，因此需要作为一组根因统一修复。

## 测试行为

**gVisor 测试**: `../gvisor/test/syscalls/linux/wait.cc:841`

测试逻辑：
- 子进程 `ForkSpinAndExit(27, 3)`。
- 父进程执行原始系统调用：`syscall(SYS_waitid, P_PID, child, &si, WEXITED, &rusage)`。
- 校验 `si_code == CLD_EXITED`、`si_status == 27`，且 `RusageCpuTime(rusage) >= 3s`。

```c
// ../gvisor/test/syscalls/linux/wait.cc:841
TEST(WaitTest, WaitidRusage) {
  pid_t child;
  constexpr absl::Duration kSpin = absl::Seconds(3);
  ASSERT_THAT(child = ForkSpinAndExit(27, absl::ToInt64Seconds(kSpin)),
              SyscallSucceeds());

  siginfo_t si = {};
  struct rusage rusage = {};
  EXPECT_THAT(
      RetryEINTR(syscall)(SYS_waitid, P_PID, child, &si, WEXITED, &rusage),
      SyscallSucceeds());
  EXPECT_GE(RusageCpuTime(rusage), kSpin);
}
```

**实际错误**: `RusageCpuTime(rusage) == 0`，期望 `>= 3s`。

## 参考实现（Linux）

**文件**: `../linux/kernel/exit.c`

```c
// 1700
wo.wo_rusage = ru;
// 1717
long err = kernel_waitid(..., ru ? &r : NULL);
```

关键语义：
- `waitid` 原始 syscall 第 5 参数与 `wait4` 一样，通过 `do_wait` 路径返回 `rusage`。

## DragonOS 当前实现

**文件**: `kernel/src/process/syscall/sys_waitid.rs`

```rust
// 80-83
let mut tmp_rusage = if rusage_ptr.is_null() { None } else { Some(RUsage::default()) };
// 106
let _ = kernel_waitid(pid_selector, infop_writer, options, tmp_rusage.as_mut())?;
// 110-112
rusage_writer.copy_one_to_user(&tmp_rusage.unwrap(), 0)?;
```

**文件**: `kernel/src/process/exit.rs`

```rust
// 149-154
let mut kwo = KernelWaitOption::new(pid_selector, options);
kwo.ret_rusage = rusage_buf;
let _ = do_wait(&mut kwo)?;
```

修复前问题：
- `tmp_rusage` 仅被用户态 copy，未由内核 wait 事件填充。
- 与 Linux 语义要求的“waitid 第五参数 = wait4 资源语义”不一致。

## 实际修复

1. `waitid` 与 `wait4` 共用 `kernel/src/process/exit.rs` 中的 `fill_wait_rusage()`。
2. `kernel/src/process/resource.rs` 的 `get_rusage()` 改为从真实 cputime 统计填充 `ru_utime/ru_stime`。
3. waited-child 退出回收时同步累计到父进程 `RUSAGE_CHILDREN`，避免相关用例继续失败。

## 变更范围

- **文件**: `kernel/src/process/exit.rs`, `kernel/src/process/resource.rs`, `kernel/src/process/cputime.rs`
- **风险**: waitid/getrusage 行为变化，可能暴露资源统计缺陷
- **依赖**: 与 `Wait4Rusage`、`WaitedChildRusage`、`IgnoredChildRusage` 共用同一基础改动

## 验证结果

- `WaitTest.WaitidRusage` 通过。
- `waitid` 的 `rusage` 与 `wait4` 语义一致。
- 最终 `wait_test` 为 `63/63` 通过。
