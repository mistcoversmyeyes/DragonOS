# gVisor shm 测试失败分析

本文记录一次在 DragonOS 上手动运行 gVisor `shm_test` 时发现的失败现象、最小复现、Linux 参考语义和 DragonOS 当前实现差异。

## 分析对象

- gVisor 测试代码：`../gvisor/test/syscalls/linux/shm.cc`
- DragonOS SHM 生命周期实现：
  - `kernel/src/mm/ucontext.rs`
  - `kernel/src/ipc/shm.rs`
  - `kernel/src/ipc/syscall/sys_shmat.rs`
  - `kernel/src/ipc/syscall/sys_shmdt.rs`
- Linux 参考实现：`../linux/ipc/shm.c`

## 一、测试范围理解

### 1.1 测试目标

这组 gVisor 测试主要验证 SysV SHM 的以下语义：

- `shmat/shmdt` 后的映射可访问性
- `SHM_RDONLY` 只读 attach 的写保护行为
- `IPC_RMID` 之后“已标记删除但仍有 attachment”对象的生命周期
- `fork`、异常退出、`execve` 交织时，SHM attachment 与 VMA 生命周期是否被正确回收

### 1.2 关键测试用例

| 用例名称 | 验证行为 | 状态 |
|---|---|---|
| `ShmDeathTest.ReadonlySegment` | 只读 attach 后写入应触发 `SIGSEGV` | 单跑通过 |
| `ShmDeathTest.SegmentNotAccessibleAfterDetach` | `shmdt` 后再次访问应触发 `SIGSEGV` | 单跑通过 |
| `ShmDeathTest.*` | 连续 death test 下的 `fork/exit/execve` 清理 | 失败 |
| `ShmTest.SegmentsSizeFixedOnCreation` | 普通 SHM lookup/attach/size 语义 | 单跑通过 |

### 1.3 实际复现矩阵

在 DragonOS 客体内实际执行如下命令：

```bash
cd /opt/tests/gvisor
./tests/shm_test --gtest_filter=ShmTest.SegmentsSizeFixedOnCreation
./tests/shm_test --gtest_filter=ShmDeathTest.SegmentNotAccessibleAfterDetach
./tests/shm_test --gtest_filter=ShmDeathTest.*
```

观察结果：

- `ShmTest.SegmentsSizeFixedOnCreation` 单独运行通过
- `ShmDeathTest.SegmentNotAccessibleAfterDetach` 单独运行通过
- `ShmDeathTest.*` 运行失败，并触发内核 panic

失败时宿主串口日志中出现：

```text
[ ERROR ] (src/mm/page.rs:110) phys page: PhysAddr(...) already exists.
Kernel Panic Occurred.
Location:
    File: src/mm/ucontext.rs
    Line: 2555, Column: 18
Message:
    Failed to map zero, may be OOM error
```

这说明问题不是普通 `shmat/shmdt` 语义错误，而是连续 death test 后状态被污染，最终在后续 `execve -> map_anonymous -> VMA::zeroed` 上暴露出来。

## 二、内核子系统现状

### 2.1 相关子系统

- SysV SHM attachment 生命周期封装：`kernel/src/mm/ucontext.rs:1596`
- VMA `open/close` 抽象与 `Drop`：`kernel/src/mm/ucontext.rs:1825`、`kernel/src/mm/ucontext.rs:2170`
- 进程地址空间退出回收：`kernel/src/mm/ucontext.rs:1210`、`kernel/src/mm/ucontext.rs:1348`
- `IPC_RMID` 实现：`kernel/src/ipc/shm.rs:323`
- `shmat`：`kernel/src/ipc/syscall/sys_shmat.rs:34`
- `shmdt`：`kernel/src/ipc/syscall/sys_shmdt.rs:45`

### 2.2 当前实现的问题

当前 DragonOS 的 SysV SHM 销毁职责被拆成了两段：

- `IPC_RMID` 时在 `kernel/src/ipc/shm.rs:323` 决定是否立即释放物理页
- 最后一个 attachment 关闭时在 `kernel/src/mm/ucontext.rs:1652` 只做 `map_count` 更新和 `free_id`

这一点和 Linux 不同。Linux 把 attachment 的打开、关闭和进程退出清理统一放在：

- `../linux/ipc/shm.c:300` 的 `shm_open()`
- `../linux/ipc/shm.c:368` 的 `__shm_close()`
- `../linux/ipc/shm.c:439` 的 `exit_shm()`

DragonOS 当前的地址空间退出路径则是：

```rust
// kernel/src/mm/ucontext.rs:1348
impl Drop for InnerAddressSpace {
    fn drop(&mut self) {
        unsafe {
            self.unmap_all();
        }
    }
}
```

而 `unmap_all()` 只做页表解除映射，不显式执行 `close_once()`：

```rust
// kernel/src/mm/ucontext.rs:1210
pub unsafe fn unmap_all(&mut self) {
    let mut flusher: PageFlushAll<MMArch> = PageFlushAll::new();
    for vma in self.mappings.iter_vmas() {
        if vma.mapped() {
            vma.unmap(&mut self.user_mapper.utable, &mut flusher);
        }
    }
}
```

这意味着异常退出时，SHM 的最终 close 依赖 `Arc<LockedVMA>` 的后续 drop 时机，而不是像 Linux 一样由专门的 SHM 退出路径统一收尾。

## 三、根因分析

| 测试点 | Linux 期望 | DragonOS 实际 | 差距 |
|---|---|---|---|
| SHM 最终销毁 | 最后一个 attach 关闭时统一决定 destroy | `IPC_RMID` 和最后 close 分裂 | 销毁职责分散 |
| task exit 下 SHM 清理 | `exit_shm()` 和 `vm_ops.close` 共同收敛 | 依赖 `unmap_all` 之后的隐式 drop 顺序 | 异常退出清理不稳 |
| 物理页元数据一致性 | 页帧释放与页元数据删除同步 | `page_manager` 遗留旧 `PhysAddr` | 导致后续匿名页分配撞车 |

### 3.1 原发故障：`ShmDeathTest.*`

`ShmDeathTest.*` 是本次失败的原发故障，而不是普通 `ShmTest` 用例。

证据：

- 单独运行 `ShmDeathTest.SegmentNotAccessibleAfterDetach` 通过
- 单独运行 `ShmTest.SegmentsSizeFixedOnCreation` 通过
- 组合运行 `ShmDeathTest.*` 时，内核在后续 `execve` 中触发 `phys page already exists` 和 panic

这说明 death test 组合执行时破坏了 SHM 或页元数据状态，后续普通用例只是更早踩到了污染后的内核状态。

### 3.2 gVisor 测试语义

`ShmDeathTest.SegmentNotAccessibleAfterDetach` 的关键逻辑如下：

```c
// ../gvisor/test/syscalls/linux/shm.cc:371
TEST(ShmDeathTest, SegmentNotAccessibleAfterDetach) {
  const auto rest = [&] {
    ShmSegment shm = TEST_CHECK_NO_ERRNO_AND_VALUE(
        Shmget(IPC_PRIVATE, kAllocSize, IPC_CREAT | 0777));
    char* addr = TEST_CHECK_NO_ERRNO_AND_VALUE(Shmat(shm.id(), nullptr, 0));

    TEST_CHECK_NO_ERRNO(shm.Rmid());
    addr[0] = 'x';
    TEST_CHECK_NO_ERRNO(Shmdt(addr));
    addr[0] = 'x';
  };

  EXPECT_THAT(InForkedProcess(rest), ... SIGSEGV ...);
}
```

该测试要求：

- `IPC_RMID` 之后，segment 不应因仍有 attachment 而立刻被销毁
- `shmdt` 之后，访问旧地址必须触发 `SIGSEGV`
- 子进程异常退出后，不应污染后续测试和 `execve` 路径

### 3.3 Linux 参考实现

Linux 在 `shm_open()` 中为每个新的 VMA open 计数：

```c
// ../linux/ipc/shm.c:300
static void shm_open(struct vm_area_struct *vma)
{
    ...
    err = __shm_open(sfd);
    WARN_ON_ONCE(err);
}
```

Linux 在 `__shm_close()` 中统一处理最后一个 attach 的关闭与销毁：

```c
// ../linux/ipc/shm.c:368
static void __shm_close(struct shm_file_data *sfd)
{
    ...
    shp->shm_nattch--;
    if (shm_may_destroy(shp))
        shm_destroy(ns, shp);
    else
        shm_unlock(shp);
}
```

Linux 还提供 task 级别的退出回收：

```c
// ../linux/ipc/shm.c:439
void exit_shm(struct task_struct *task)
{
    for (;;) {
        if (list_empty(&task->sysvshm.shm_clist))
            break;
        ...
    }
}
```

关键语义：

- `IPC_RMID` 只设置“待销毁”状态，不代表立刻释放
- 真实销毁由“最后一个 attach 关闭”统一完成
- task 异常退出由 `exit_shm()` 补齐收尾，不依赖 VMA drop 的偶然顺序

### 3.4 DragonOS 当前实现差异

DragonOS 在 `SysvShmAttachment::on_close_segment()` 中：

```rust
// kernel/src/mm/ucontext.rs:1652
if let Some(kernel_shm) = shm_manager_guard.get_mut(&self.shm_id) {
    kernel_shm.update_dtim();
    kernel_shm.decrease_count();

    if kernel_shm.map_count() == 0 && kernel_shm.mode().contains(ShmFlags::SHM_DEST) {
        shm_manager_guard.free_id(&self.shm_id);
    }
}
```

而 `ipc_rmid()` 中真正释放物理页和删除 `PAGE_MANAGER` 元数据是在另一条路径：

```rust
// kernel/src/ipc/shm.rs:323
pub fn ipc_rmid(&mut self, id: ShmId) -> Result<usize, SystemError> {
    kernel_shm.set_mode(ShmFlags::SHM_DEST, true);
    ...
    if map_count > 0 {
        self.free_key(&key);
    } else {
        LockedFrameAllocator.free(paddr, PageFrameCount::new(1));
        page_manager_guard.remove_page(&paddr);
        self.free_id(&id);
    }
}
```

这里的核心偏差有两点：

- `IPC_RMID` 和 “最后一个 attach 关闭” 并没有走同一个 destroy helper
- 最后一个 attachment 关闭后只做 `free_id`，没有复用完整的物理页释放与 page 元数据移除逻辑

这会导致 removed-but-attached 场景在异常退出中更容易出现元数据不一致。

### 3.5 页元数据污染如何暴露

失败最终暴露在匿名页分配路径：

```rust
// kernel/src/mm/page.rs:104
fn insert(&mut self, page: &Arc<Page>) -> Result<Arc<Page>, SystemError> {
    let phys = page.phys_address();
    if !self.phys2page.contains_key(&phys) {
        self.phys2page.insert(phys, page.clone());
        Ok(page.clone())
    } else {
        log::error!("phys page: {phys:?} already exists.");
        Err(SystemError::EINVAL)
    }
}
```

随后 `execve` 中匿名映射建立失败：

```rust
// kernel/src/mm/ucontext.rs:2554
let r = unsafe { mapper.map(cur_dest.virt_address(), flags) }
    .expect("Failed to map zero, may be OOM error");
```

因此，`ShmTest.SegmentsSizeFixedOnCreation` 在全集中失败只是次生症状。真正的问题是前序 death test 组合把页元数据状态打坏了。

## 四、详细单个测试修复文档

### 4.1 Bug Fix: `ShmDeathTest.*`

#### 根因分析

`ShmDeathTest.*` 的原发故障是：DragonOS 当前没有把 SysV SHM 的最后关闭、`IPC_RMID` 后销毁、task 异常退出收尾统一到一条稳定的生命周期路径上。

`VmaOps.close()` 已经接管了 attachment 计数，但当进程异常退出时，地址空间回收依赖 `unmap_all()` 之后的隐式 drop 顺序来触发 `close`。这与 Linux 的 `shm_close() + exit_shm()` 语义相比更脆弱。

#### 测试行为

gVisor 测试位置：`../gvisor/test/syscalls/linux/shm.cc:371`

测试逻辑：

1. 创建私有 SHM segment
2. `shmat` 建立 attachment
3. `IPC_RMID` 标记删除
4. `shmdt` 解除 attachment
5. 再次访问旧地址，预期收到 `SIGSEGV`
6. 整个过程运行在 forked child 中，要求退出后不污染父进程后续测试

#### 实际错误

- `kernel/src/mm/page.rs:110`: `phys page: PhysAddr(...) already exists`
- `kernel/src/mm/ucontext.rs:2555`: `Failed to map zero, may be OOM error`

#### 参考实现（Linux）

- `../linux/ipc/shm.c:300`：`shm_open()`
- `../linux/ipc/shm.c:368`：`__shm_close()`
- `../linux/ipc/shm.c:439`：`exit_shm()`

Linux 的关键语义是：

- 每个新的 SHM VMA open 都计入 attachment
- 每个关闭都统一走 `__shm_close()`
- 最后一个 close 决定是否 destroy
- task 异常退出由 `exit_shm()` 补齐收尾

#### DragonOS 当前实现

- `kernel/src/mm/ucontext.rs:1652`：`SysvShmAttachment::on_close_segment()`
- `kernel/src/ipc/shm.rs:323`：`ShmManager::ipc_rmid()`
- `kernel/src/mm/ucontext.rs:1348`：`InnerAddressSpace::drop()`

问题：

- destroy 逻辑分裂
- exit 路径未显式 close SHM VMA
- page manager 状态可能在最后关闭后未与页帧回收保持一致

#### 修复方案

1. 在 `ShmManager` 中抽出统一的 “removed + unattached => destroy” helper。
2. `ipc_rmid()` 与 `SysvShmAttachment::on_close_segment()` 共用这条 helper。
3. 调整地址空间退出路径，确保 SHM VMA 在 task exit 时显式走 `close_once()`，不要只依赖 `Drop` 的时机。
4. 修复后回归验证 `./tests/shm_test --gtest_filter=ShmDeathTest.*`。

#### 变更范围

- `kernel/src/mm/ucontext.rs`
- `kernel/src/ipc/shm.rs`

风险：

- 会影响 `fork`、`munmap`、`execve`、`IPC_RMID` 的 SHM 生命周期交互

### 4.2 Bug Fix: `ShmTest.SegmentsSizeFixedOnCreation`（次生症状）

#### 根因分析

这个 case 单独运行通过，因此它不是原发 bug。它只是全集里较早踩到“前序 death test 已经污染内核状态”的普通 SHM 用例。

#### 测试行为

gVisor 测试位置：`../gvisor/test/syscalls/linux/shm.cc:438`

测试逻辑：

1. 用 key 创建一个 base segment
2. 用同一个 key、较小 size 再次 `shmget`，应成功返回同一 segment
3. 用更大 size `shmget`，应返回 `EINVAL`
4. 分别 `shmat` 两次
5. 两个映射都应按原始 segment size 可访问

#### DragonOS 当前实现

- `kernel/src/ipc/syscall/sys_shmget.rs:40` 的 size 判定本身是正确的
- `kernel/src/ipc/syscall/sys_shmat.rs:34` 的普通 attach 逻辑单跑也没有暴露问题

因此这个用例的失败不能单独修，应作为上游 lifecycle bug 修复后的回归验证项。

#### 修复方案

1. 先修 `ShmDeathTest.*` 的生命周期问题。
2. 再用 `ShmTest.SegmentsSizeFixedOnCreation` 验证全集执行下状态是否恢复一致。

## 五、修复建议

### 5.1 关键改动

| 文件 | 改动 | 原因 |
|---|---|---|
| `kernel/src/ipc/shm.rs` | 抽出统一的 removed-and-unattached destroy helper | 消除 `IPC_RMID` 与 last-close 分裂 |
| `kernel/src/mm/ucontext.rs` | 在 SHM close 和地址空间退出中统一走该 helper | 稳定 task exit 下的清理顺序 |
| `kernel/src/mm/page.rs` | 增强页元数据一致性检查 | 防止 stale `PhysAddr` 留在 `PAGE_MANAGER` |

### 5.2 实现细节

- 推荐先统一 destroy helper，再改退出路径；否则容易出现重复释放或仍旧漏清理。
- 修复后建议至少回归以下命令：

```bash
cd /opt/tests/gvisor
./tests/shm_test --gtest_filter=ShmDeathTest.*
./tests/shm_test
```

- 还应补充一个更窄的回归场景：`IPC_RMID` 后最后一个 attachment 在异常退出中释放，确保不会污染后续 `execve`。

## 六、结论

本次失败的核心不是普通 `shmat/shmdt` 语义错误，而是 SysV SHM 在 `IPC_RMID`、最后一个 attachment 关闭、task 异常退出三者交织时，生命周期回收路径不统一。

Linux 通过 `shm_open()`、`__shm_close()`、`exit_shm()` 把这条路径收敛到了稳定语义；DragonOS 当前实现仍依赖 `unmap_all()` 和 `LockedVMA::Drop` 的隐式执行顺序，因此在 gVisor death test 组合下更容易出现页元数据污染，最终以 `phys page already exists` 和后续 `execve` panic 的形式暴露出来。
