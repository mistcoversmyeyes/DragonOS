# Bug Fix: Waiters/WaitSpecificChildTest.AfterChildExecve

## 根因分析
`wait(pid)` 在 DragonOS 修复前会先把目标 pid 解析成一个固定的 `ProcessControlBlock`，随后在阻塞等待期间一直持有这个旧对象。  
但 `execve` 里的 `de_thread` 会把线程组中执行 `exec` 的线程提升为新的 leader，并和旧 leader 交换 pid/TID。这样父进程最初绑定的旧 PCB 不再对应用户态看到的那个 pid，最终就会在 `AfterChildExecve/{0,1}` 中出现等待不到目标或错误返回。

## 测试行为

**gVisor 测试**: `../gvisor/test/syscalls/linux/wait.cc`

测试逻辑：
- 子进程创建多线程/线程组场景。
- 非 leader 线程执行 `execve`，触发 Linux `de_thread` 路径。
- 父进程使用固定 `pid` 调用 `waitpid(pid, ...)`。
- 期望无论内部 leader 如何切换，只要用户态 pid 没变，父进程都能等待到最终退出事件。

**实际错误**:
- `waitpid(pid, ...)` 阻塞后继续盯住旧 leader。
- `de_thread` 完成 pid 交换后，wait 无法跟随真实目标，导致 `AfterChildExecve/{0,1}` 失败。

## 参考实现（Linux）

Linux `do_wait()` 对指定 pid 的等待，本质上是围绕 pid 命名空间下的 task 关系持续判定，而不是把最初命中的 task 指针永久缓存为唯一目标。  
当 `de_thread` 改变了该 pid 对应的 task 时，后续 wait 仍应围绕“这个 pid 当前映射到谁”继续推进。

## DragonOS 修复前实现

**文件**: `kernel/src/process/exit.rs`

修复前的问题集中在两个点：
- 指定 pid 的 wait 先解析一次目标对象。
- 进入睡眠后没有在每次唤醒时重新按 pid 查询。

因此，只要 `de_thread` 在等待期间发生 pid/TID 交换，父进程就会继续围绕过期对象做资格判断。

## 实际修复

1. 在 `kernel/src/process/exit.rs` 中新增 `resolve_wait_pid_target()`，把“按 pid 重新解析当前目标”收敛成单独逻辑。
2. `PidConverter::Pid(pid)` 路径下的 fast path 和 `wait_event_interruptible` 唤醒回调都改为每次重新执行 `resolve_wait_pid_target(&pid, kwo.options)`。
3. 这样无论 `de_thread` 之前还是之后，wait 语义始终绑定到“当前这个 pid 对应的 task”，而不是旧 leader 的 PCB。

## 变更范围

- **文件**: `kernel/src/process/exit.rs`
- **关联语义**: `waitpid(pid, ...)`, mt-exec, `de_thread`
- **风险**: 影响指定 pid 的 wait 行为，因此需要回归 `wait_test` 中所有 `WaitSpecificChildTest` 子用例

## 验证结果

- 修复前，在前两批修复完成后，`wait_test` 仅剩：
  - `Waiters/WaitSpecificChildTest.AfterChildExecve/0`
  - `Waiters/WaitSpecificChildTest.AfterChildExecve/1`
- 修复后，`wait_test` 达到 `63/63` 全通过。
