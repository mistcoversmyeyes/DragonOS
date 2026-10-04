# 历史 panic 现场

本文件保留迁移前 Issue 草稿全文，供核对早期文件创建、后台页面回收及用户缺页直接回收的 panic 现场。旧现场的源码提交、挂载布局和完整原始串口没有全部留存；其中关于复现稳定性、可能原因及排查方向的文字反映当时的调查阶段。

当前 Issue 正文见 [index.md](index.md)，具有版本和完整附件的三组实验见 [README.md](README.md)。旧现场不与这些实验合并为同一次运行，历史 ENOSPC 也不作为连续压测中已经复现的结果。

---

# bug(mm): lmbench `ext4_create_delete_files_10k_ops` triggers kernel allocation panics during file creation and page reclaim

关联总跟踪：#2286。

## Bug 表现

在 DragonOS 中运行 LMbench，已观察到三种内核堆分配失败后的 panic：`ext4_create_delete_files_10k_ops` 创建文件时的目录块缓冲区分配失败、反复测试并重新初始化之后后台页面回收线程的分配失败，以及 `mem_read_bw` 缺页触发直接回收时的分配失败。

这些现场说明，**问题不只发生在页面回收函数内部：普通文件创建路径也会在内核对象分配失败后直接 panic；已经进入回收路径时，回收器自身的辅助分配同样可能失败并 panic。**

需要分别定位以下问题：

- **内存压力的来源**：负载及其前序操作为什么使物理页／内核堆分配失败，是否存在容量不足、资源释放或回收滞后、内存泄漏或其他分配限制。
- **普通内核分配路径的失败处理**：文件创建中的不可失败 `Box` 分配最终进入 panic，没有把内存不足作为错误返回给调用者。这条普通分配路径也没有通用的直接回收／OOM 处置过程。
- **回收路径自身的健壮性**：收集待回收页面引用的 `Vec` 需要扩容，导致回收尚未真正释放页面就可能因分配失败而 panic。

普通分配与回收路径的具体失败位置已有证据；系统为何走到这一内存状态，以及后台回收是否及时、是否取得进展，仍需进一步取证。

### 文件创建路径中的对象分配 panic（新增现场）

最新一次串口记录中，先执行了一轮 `--samples 1` 的 `ext4_create_delete_files_10k_ops`，在 120 秒后以 `rc=124` 超时，runner 随后报告清理完成。在同一 guest 中再次设置 `--samples 5`，runner 重新初始化了 1 GiB 的 ext4 fixture，校准得到 `ENOUGH=1000000`。

**这次前四个 sample 均为 `rc=124` 超时，不是四次成功；第五个 sample 期间出现首个 panic。** 这与后文“4/5 有效、随后 ENOSPC”的旧现场是不同的一轮实验。

```text
[lmbench-runner] RUN  ext4_create_delete_files_10k_ops (samples=5, timeout=120s)
[lmbench-runner]      ext4_create_delete_files_10k_ops sample 1: no valid numeric result (rc=124)
[lmbench-runner]      ext4_create_delete_files_10k_ops sample 2: no valid numeric result (rc=124)
[lmbench-runner]      ext4_create_delete_files_10k_ops sample 3: no valid numeric result (rc=124)
[lmbench-runner]      ext4_create_delete_files_10k_ops sample 4: no valid numeric result (rc=124)
Kernel Panic Occurred. raw_pid: 149
Location:
    File: src/mm/allocator/kernel_allocator.rs
    Line: 247, Column: 5
Message:
    global_alloc_error, layout: Layout { size: 4096, align: 1 (1 << 0) }
```

首个 panic 的关键栈帧按调用方向整理如下：

```text
syscall_64
  → syscall_handler
  → SysCreatHandle::handle
  → open_utils::do_open
  → do_sys_openat2
  → MountFSInode::create_file_with_post_commit
  → LockedExt4Inode::create
  → Ext4::create_with_owner_and_attr
  → Ext4::transaction_link_inode
  → Ext4::transaction_dir_add
  → Box<[u8; 4096]>::new_uninit_in
  → alloc::alloc::handle_alloc_error
  → __rust_alloc_error_handler
  → global_alloc_err_handler
  → panic
```

这次位于 `creat` 系统调用经过的文件创建路径，失败的是 **4096 字节的目录块缓冲区分配**，不是回收器的 `Vec` 扩容。首个栈中没有 `do_user_addr_fault`、`PageReclaimer::shrink_list` 或 `page_reclaim_thread`。对应的 `transaction_dir_add()` 中有 `Box::new([0; BLOCK_SIZE])` 等不可失败分配，分配失败不能通过该函数的 `Result` 返回 `ENOMEM`。

核对 `kernel/src/mm/allocator/kernel_allocator.rs`：大于 2048 字节的分配进入 `alloc_in_buddy()`，4096 字节在当前 x86_64 配置下请求一个物理页；底层失败最终返回空指针，不可失败的 `Box` 分配随后调用 `handle_alloc_error()`，进入全局 panic。x86_64 页分配器仅对已经被标记为 OOM victim 的任务提供“唤醒回收线程后再试一次”的特殊分支，这不构成普通分配请求的直接回收或 OOM victim 选择机制。

因此，这次可以确认的是：**普通文件创建路径未能从内核对象分配失败中恢复，且该分配路径没有像用户缺页处理那样进入直接回收。** 这不等于后台回收线程在此前或并发执行期间完全没有运行，也不能仅凭这一栈认定 Buddy 分配器的内部算法存在错误。

首个 panic 后，同一 PID 又打印了相同分配失败，随后 PID 6 也报告 `Layout { size: 4096, align: 8 }`；日志达到 panic 次数上限并 halt。最后这次没有完整调用栈，不能仅凭 PID 将它认定为前述 `shrink_list()` 的同一个失败点。

### 后台内存页回收线程对象分配 panic

在同一 guest 中执行：

```sh
cd /opt/tests/benchmark/lmbench
sh run.sh --only ext4_create_delete_files_10k_ops --samples 5 --timeout 120 --warmup 0
```

观察到的过程如下：

1. 第一轮前四个 sample 有效，第五个 sample 返回 `rc=1`，runner 报告 `incomplete samples (4/5)`。
2. 再执行一轮相同命令，五个 sample 均返回 `rc=1`，runner 报告 `incomplete samples (0/5)`。两轮最终输出都包含 `mkdir failed: No space left on device`，而非有效的性能结果。
3. 后续重新运行时，初始化写入 `/ext4/zero_file` 也出现 `No space left on device`。当时手动删除了 `/ext4` 目录及已有的 `ext4.img`，再尝试运行 runner。此时 `/ext4` 的实际挂载状态未留存，因此这段操作只作为现场历史记录，不作为确定的复现步骤。
4. 重新创建 1 GiB 的 `ext4.img` 时，`dd` 在写入约 715 MiB 后返回 `Cannot allocate memory`。初始化失败返回 shell 后，后台页面回收线程发生 panic。

关键输出摘录：

```text
mkdir failed: No space left on device
lmbench: invalid measurement output

# 后续重新初始化时：
dd: error writing '/opt/tests/benchmark/lmbench/ext4.img': Cannot allocate memory
716+0 records in
715+0 records out
749830144 bytes (750 MB, 715 MiB) copied

Kernel Panic Occurred. raw_pid: 6
Location:
    File: src/mm/allocator/kernel_allocator.rs
    Line: 247, Column: 5
Message:
    global_alloc_error, layout: Layout { size: 32768, align: 8 (1 << 3) }
```

下面按调用方向整理关键栈帧，省略地址、泛型符号和展开栈的辅助函数：

```text
kernel_thread_bootstrap_stage2
  → KernelThreadClosure::run
  → page_reclaim_thread
  → PageReclaimer::shrink_list
  → RawVec::grow_one
  → alloc::raw_vec::handle_error
  → alloc::alloc::handle_alloc_error
  → __rust_alloc_error_handler
  → global_alloc_err_handler
  → panic
```

这次失败发生在后台回收线程中，申请大小为 **32768 字节**。它并非 `lat_fs` 正在执行时直接抛出的 panic，而是出现在上述负载与重新初始化之后。前面的文件系统 `ENOSPC` 与后面的内存分配失败是否具有共同根因，尚未确认。

### 用户缺页处理中的直接内存页回收分配 panic

另一次运行 LMbench 时，`mem_copy_bw` 已完成，runner 随后开始执行 `mem_read_bw`，在用户缺页处理触发的直接回收路径中发生 panic：

```text
[lmbench-runner] RUN  mem_read_bw (samples=1, timeout=120s)
Kernel Panic Occurred. raw_pid: 95
Location:
    File: src/mm/allocator/kernel_allocator.rs
    Line: 247, Column: 5
Message:
    global_alloc_error, layout: Layout { size: 256, align: 8 (1 << 3) }
```

按调用方向整理的关键栈帧为：

```text
缺页异常入口
  → do_page_fault
  → X86_64MMArch::do_user_addr_fault
  → PageReclaimer::shrink_list
  → RawVec 扩容
  → alloc::raw_vec::handle_error
  → alloc::alloc::handle_alloc_error
  → __rust_alloc_error_handler
  → global_alloc_err_handler
  → panic
```

这里是在触发缺页的用户进程上下文中执行直接回收，不是另外唤起一个回收线程。缺页处理发现 `VM_FAULT_OOM` 后先尝试直接回收；本次在回收过程中申请 **256 字节**失败，没有走到这次缺页处理后续的 OOM killer 处置。

### 两条回收路径共同的失败位置

`kernel/src/mm/page.rs` 中的 `PageReclaimer::shrink_list()` 分为两个阶段：先收集待回收页面，再调用 `evict_pages()` 尝试回收。第一阶段的 `drain_lru()` 使用会动态扩容的 `Vec`：

```rust
fn drain_lru(&mut self, count: PageFrameCount) -> Vec<Arc<Page>> {
    let mut victims = Vec::new();
    for _ in 0..count.data() {
        match self.lru.pop_lru() {
            Some((_paddr, page)) => victims.push(page),
            None => break,
        }
    }
    victims
}
```

`victims.push(page)` 扩容时分配的是存放 `Arc<Page>` 引用的数组，不是在创建新的物理页面。前述后台回收和直接回收两次 panic 的栈都指向这里的 `RawVec` 扩容失败；`drain_lru()` 可能被内联，因此打印的栈直接表现为 `shrink_list → RawVec`。

后台回收线程在空闲物理页低于 4096 页时尝试回收 4096 页；缺页处理的直接回收尝试收集 64 页。这个回收触发阈值并不等于为回收路径预留了一块独占可用内存，因此不能保证上述 `Vec` 扩容一定成功。

预期行为是：回收路径应能在内存紧张、辅助分配失败时继续执行有界的回收或向上报告失败，让内核有机会执行后续分配失败／OOM 处置，而不是因为收集待回收页面引用的数组扩容失败就直接全局 panic。使系统进入这一状态的内存占用来源仍需另行定位。

## 复现方式

**暂未找到稳定复现方式。** 当前现象与同一 guest 的前序负载、文件系统状态以及内存状态有关的可能性尚待验证。连续多次运行通过，不能排除后续单次运行失败；先前也观察到 `mem_read_bw` 连续运行 20 次未触发，而其他运行中出现过上述 panic。

可以先在 DragonOS guest 内通过原生 runner 尝试以下负载。初始化、校准、超时、结果提取和清理由 runner 统一处理，不单独直接调用 `lat_fs` 或 `bw_mem`。

反复运行 ext4 10 KiB 文件创建／删除用例：

```sh
cd /opt/tests/benchmark/lmbench
sh run.sh --only ext4_create_delete_files_10k_ops --samples 5 --timeout 120 --warmup 0
# guest 未 panic 时，可以在同一 guest 中再次执行该命令，观察结果变化。
```

针对新增的文件创建 panic，已有的一次运行顺序是先单 sample 超时，再在同一 guest 内运行五个 samples：

```sh
sh run.sh --only ext4_create_delete_files_10k_ops --samples 1 --timeout 120 --warmup 0
# 上次 runner 返回后，在同一 guest 内继续：
sh run.sh --only ext4_create_delete_files_10k_ops --samples 5 --timeout 120 --warmup 0
```

该顺序曾出现“先前四次超时、第五次文件创建时 panic”，但尚未验证能够稳定复现。应保留前序超时及其清理过程，不能把它描述成每次新启动后第五个 sample 必然触发。

尝试单独运行内存读取用例：

```sh
sh run.sh --only mem_read_bw --samples 1 --timeout 120 --warmup 0
```

也可以使用 `--samples 20` 观察同一次初始化后连续采样的情况。这个尝试与首次观察到的“前序 `mem_copy_bw` 完成后运行 `mem_read_bw`”不同，不应当作已经验证的稳定复现条件。

排查时重点记录以下信息：

- sample 的退出码和原始错误，以及初始化、执行、清理三个阶段中哪一阶段失败。
- 运行前后的 `/proc/meminfo`，特别是 `MemFree`、`Cached`、`Shmem` 和 `Slab`。
- `/ext4` 的实际挂载信息，以及 `df -h /ext4`、`df -i /ext4`，区分块空间和 inode 耗尽。
- runner 打印的 `ENOUGH` 值。早先 ENOSPC 的 ext4 两轮失败均出现校准失败并回退到 `50000`；新增文件创建 panic 和此前 `mem_read_bw` panic 所在运行的值均为 `1000000`。
- 首个 panic 的完整调用栈、失败分配大小，以及此前在同一 guest 中做过的测试和清理操作。

## 运行环境

- DragonOS，x86_64，QEMU/KVM。
- VM 配置为 2 vCPU、2 GiB RAM；启动日志已确认 guest 识别到接近 2 GiB 的可用物理内存。
- LMbench 3.0-a9，通过 `run.sh` 运行，sample 超时 120 秒，warmup 为 0。
- `ext4_create_delete_files_10k_ops` 的测量命令为 `lat_fs -s 10k -P 1`，10k 表示每个文件为 10 KiB，不是创建／删除次数。
- `mem_read_bw` 的测量命令为 `bw_mem -P 1 -N 50 512m frd`，buffer 为 512 MiB。
