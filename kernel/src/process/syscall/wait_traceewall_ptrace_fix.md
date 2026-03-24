# Bug Fix: WaitTest.TraceeWALL

## 根因分析

有两个根因：

1. `SYS_PTRACE` 没有注册 syscall handler，`ptrace(PTRACE_TRACEME)` 直接返回 `ENOSYS`。
2. wait 子进程匹配逻辑没有 “traced child 视为 __WALL” 语义，导致 tracee 仍被当成普通 clone/non-clone 子进程分类。

## 测试行为

**gVisor 测试**: `../gvisor/test/syscalls/linux/wait.cc:869`

测试逻辑：
- 子进程调用 `ptrace(PTRACE_TRACEME, 0, nullptr, nullptr)`。
- 父进程执行 `wait4(child, &status, __WCLONE, nullptr)`。
- 在 gVisor/Linux 新语义下应成功回收子进程。

```c
// ../gvisor/test/syscalls/linux/wait.cc:869
TEST(WaitTest, TraceeWALL) {
  pid_t child = fork();
  if (child == 0) {
    TEST_PCHECK(ptrace(PTRACE_TRACEME, 0, nullptr, nullptr) == 0);
    _exit(0);
  }
  ASSERT_THAT(Wait4(child, &status, __WCLONE, nullptr),
              SyscallSucceedsWithValue(child));
}
```

**修复前实际错误**:
- 日志报 `Unsupported syscall ID: 101 -> SYS_PTRACE`。
- 后续出现调度断言 panic（连锁失败）。

## 参考实现（Linux）

**文件**: `../linux/kernel/ptrace.c`, `../linux/kernel/exit.c`

```c
// ptrace.c:1284
if (request == PTRACE_TRACEME) {
    ret = ptrace_traceme();
    goto out;
}
```

```c
// exit.c:1073
/* Wait for all children if __WALL or if traced by us. */
if (ptrace || (wo->wo_flags & __WALL))
    return 1;
```

关键语义：
- `PTRACE_TRACEME` 必须可用。
- traced child 的等待资格不受普通 clone/non-clone 分类限制。

## DragonOS 当前实现

**文件**: `kernel/src/syscall/mod.rs`

```rust
// 145-152
_ => {
    log::error!("Unsupported syscall ID: {} -> {}, args: {:?}", ...);
    Err(SystemError::ENOSYS)
}
```

**文件**: `kernel/src/process/exit.rs`

```rust
// 65-76
fn child_matches_wait_options(...) -> bool {
    if options.contains(WaitOption::WALL) {
        return true;
    }
    let is_clone_child = child_exit_signal != Signal::SIGCHLD;
    let wants_clone = options.contains(WaitOption::WCLONE);
    is_clone_child == wants_clone
}
```

修复前问题：
- 缺少 `sys_ptrace`，`PTRACE_TRACEME` 无入口。
- 匹配逻辑缺少 trace 维度，`__WCLONE` 下 tracee 可能被误判为不可等待。

## 实际修复

1. 新增 `kernel/src/process/syscall/sys_ptrace.rs`，最小支持 `PTRACE_TRACEME`。
2. 在 `kernel/src/process/syscall/mod.rs` 注册 `mod sys_ptrace;`。
3. 在 `ProcessFlags` 中使用 `PTRACED` 标记，`PTRACE_TRACEME` 成功时置位。
4. 在 `child_matches_wait_options()` 中增加 traced-child fast path：若子进程带 `PTRACED`，则直接按 `__WALL` 语义匹配。

## 变更范围

- **文件**: `kernel/src/process/syscall/mod.rs`, `kernel/src/process/syscall/sys_ptrace.rs`（新增）, `kernel/src/process/mod.rs`, `kernel/src/process/exit.rs`
- **风险**: ptrace 子系统尚未完整，建议仅启用 `PTRACE_TRACEME` 最小语义并返回其余 request `ENOSYS`
- **依赖**: wait 资格逻辑要与 ptrace 状态字段一致

## 验证结果

- `WaitTest.TraceeWALL` 通过。
- 不再出现 `SYS_PTRACE` unsupported。
- 最终 `wait_test` 为 `63/63` 通过。
