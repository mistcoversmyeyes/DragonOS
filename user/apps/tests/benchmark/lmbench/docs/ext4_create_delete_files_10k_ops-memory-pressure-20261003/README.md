# ext4_create_delete_files_10k_ops 内存压力与回收实验记录

## 现象

在 DragonOS 中连续运行 `ext4_create_delete_files_10k_ops`：前两轮 runner 均成功返回并完成清理，但空闲物理内存和空闲 inode 数持续下降，文件缓存、脏页和内核堆占用持续增加。第三轮在文件创建路径发生 `global_alloc_error`，随后内核 panic。原计划运行 20 轮，实际只完成两轮，第三轮中断。

这次压测没有出现 `No space left on device`。它与此前讨论过的 ENOSPC 现象需要分别记录，不能仅凭这次 panic 确认二者具有相同根因。

本目录共三个文件：

| 文件 | 内容 |
| --- | --- |
| [README.md](README.md) | 现象、实验方式、结果及当前推测；包含后续单轮静置／sync 对照结果 |
| [ext4_create_delete_files_10k_ops-serial.txt](ext4_create_delete_files_10k_ops-serial.txt) | 连续压测从启动到 panic 的完整串口可读副本 |
| [ext4_create_delete_files_10k_ops-resource-trends.csv](ext4_create_delete_files_10k_ops-resource-trends.csv) | 连续压测的基线及第一、第二轮结束后的资源数据 |

两个数据文件分别由原实验的 `serial-readable.txt`、`resource-trends.csv` 重命名复制，内容未重新采样或改写。串口可读副本已去除 ANSI 控制序列、CR 和 NUL；原始字节串口仍保存在原实验目录中。本文的静置／sync 对照来自另一台全新启动、使用相同内核与基础镜像的 guest，不是第三轮 panic 后继续执行的操作。

## 实验

### 环境与负载

| 项目 | 配置 |
| --- | --- |
| 源码提交 | `3ffc8327522dbd650505015202dce35fee545ef8` |
| 平台 | DragonOS x86_64，QEMU/KVM，2 vCPU，2 GiB RAM |
| Guest 内存统计 | `MemTotal=2069836 KiB` |
| 磁盘 | 独立 2 GiB 基础镜像，QEMU `snapshot=on` |
| 文件系统布局 | `/ext4` 是根 ext4 上的普通目录；`/tmp` 是 tmpfs；本次没有额外挂载 1 GiB loop ext4 fixture |
| 用例 | `ext4_create_delete_files_10k_ops`，每个文件大小为 10 KiB，10k 不是文件数量 |
| Runner 参数 | 每次 `--samples 1 --timeout 120 --warmup 0` |
| 校准 | 由原生 runner 自动校准；本文两组实验均得到 `ENOUGH=1000000` |

所有负载均通过原生 runner 执行，初始化、校准、测量和清理由 runner 负责，没有直接调用 `lat_fs`：

```sh
cd /opt/tests/benchmark/lmbench
sh run.sh --only ext4_create_delete_files_10k_ops \
  --samples 1 --timeout 120 --warmup 0
```

**实验 A：连续压测。** 在同一 guest 中重复上述命令，计划最多 20 轮；遇到首个异常停止。第一轮之前采集基线，每轮 runner 返回后采集阶段数据。此次与独立 ramfs VM 并行启动，因此性能数值仅作为测量成功的记录，不用来比较文件系统性能。

**实验 B：单轮、静置、显式同步。** 使用相同产物新启动 guest，执行一次上述命令；返回后立即采样，并在静置 15、30、60 秒时采样。随后只执行一次 `sync`，记录退出码和耗时；返回后立即及再过 30 秒采样。`sync` 的宿主保护时限为 180 秒；它是测试结束后的诊断干预，没有写入 runner，也没有改变单个 sample 的 120 秒超时。静置期间不增加清理操作，不执行 `drop_caches`。

### 观测方式与统计口径

阶段采样通过串口在 guest 内依次执行：

```sh
printf '\n__OBS_after_2__\n'
cat /proc/meminfo
df -k / /ext4 /tmp
df -i / /ext4 /tmp
printf '\n__HOST_DONE_obs_after_2__\n'
```

`OBS` 只是日志分段标记；`baseline` 在首次 runner 之前，`after_N` 在第 N 次完整 runner 返回之后。实验 A 另有每次读取后等待 5 秒的后台 `/proc/meminfo` 采样，标记为 `__MEM_TICK__`。用于 CSV 的 `baseline`、`after_1`、`after_2` 已核对，没有周期采样输出交错。第三轮 panic 前的数据来自最后一次周期采样，不代表分配失败瞬间的精确状态。

这些读取不是原子快照。实验 A 的 `after_N` 没有额外等待后台回收完成；runner 报告清理完成，也不等于所有 inode 和物理页已经释放。`/ext4` 与 `/` 属于同一个文件系统，所以 `df` 的两个 ext4 行不能相加；CSV 记录的是根 ext4 的统计。

数值单位为 KiB（原始 CSV 的 `_kB` 列和串口的 `kB` 按 1024 字节解释），MiB 按 KiB / 1024 换算。各字段口径如下：

- `MemFree`：物理页分配器报告的空闲页大小。
- `Cached`：此版本统计的文件后备页大小，已扣除 shmem 页。
- `Dirty`：文件脏页状态统计，不能作为独立内存池再次与 `Cached` 相加。
- `Slab`：Zone 持有的全部 slab 页（空页、部分使用页、满页）加上 `LARGE_ALLOCATION_BYTES`，后者是直接通过 Buddy 分配、尚未释放或转交 Zone 的大块内核堆物理页。这个字段不等于存活对象的有效字节数，也不是 Buddy 全部用途的分配总量。
- `IFree`：文件系统报告的空闲 inode 数。

### 证据来源与一致性

| 实验 | 原记录标识 | UTC 开始时间 |
| --- | --- | --- |
| A：连续压测 | `attempt-20261002T165926Z` | 2026-10-02 16:59:26 |
| B：静置／sync | `attempt-idle-sync-20261003T150952Z` | 2026-10-03 15:09:52 |

两组实验使用相同内核和基础镜像：

```text
kernel.elf SHA256:
c5f7d17a44079a8f9e3640fbf810657d6fbf5533d95a90e475c05872c1a2dff9
disk-image-x86_64.img SHA256:
2ddae2541b45ecf7698a449630ef4781770c4fae325b7ffb70214cd068831675
```

两组实验结束后 QEMU 均已退出，基础镜像校验和不变。实验 A 宿主机 OOM 计数没有增加。归档串口可读副本 SHA256 为 `f5643a79df11735ce070a534c2a7cc1b4e0fd1435f8a26f04f9e2cc4e34fc0d7`；原始字节串口 SHA256 为 `1838211adfafbb175e8d102b705f2e9a6cc78367e8045c0d42603f151257472e`。实验 B 的阶段数据直接收录于下文，未混入实验 A 的 CSV。

## 实验结果

### A：前两轮成功，第三轮内核分配失败

| 轮次 | 结果 | 性能输出 | Runner 调用耗时 |
| --- | --- | ---: | ---: |
| 1 | `rc=0`，有效结果 | 1441 ops/sec | 56.774 秒 |
| 2 | `rc=0`，有效结果 | 3117 ops/sec | 46.352 秒 |
| 3 | 文件创建时 panic，未正常返回 | 无完整结果 | 中断 |

资源变化如下，除 inode 数外均为 KiB：

| 阶段 | MemFree | Cached | Dirty | Slab | IFree | 磁盘可用空间 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 2045868 | 2928 | 28 | 14624 | 124378 | 1450132 |
| after_1 | 1372544 | 286028 | 280672 | 404400 | 100991 | 1450128 |
| after_2 | 637012 | 605856 | 600460 | 819996 | 74342 | 1450128 |

第二轮相对基线的物理内存收支基本闭合：

```text
MemFree 减少 = 2045868 - 637012 = 1408856 KiB（1375.84 MiB）
Cached  增加 =  605856 -   2928 =  602928 KiB（ 588.80 MiB）
Slab    增加 =  819996 -  14624 =  805372 KiB（ 786.50 MiB）

Cached + Slab 增量 = 1408300 KiB，占 MemFree 减少量的 99.96%
差额 = 556 KiB
```

脏页与 inode 还有一个精确对应关系：

| 阶段 | IFree 相对基线减少 | Dirty 相对基线增加 | 对应关系 |
| --- | ---: | ---: | --- |
| after_1 | 23387 | 280644 KiB | 23387 × 12 KiB |
| after_2 | 50036 | 600432 KiB | 50036 × 12 KiB |

10 KiB 文件占用 3 个 4 KiB 页，这个关系强烈支持 inode 和对应文件数据一起滞留，但统计对应本身还不是逐文件引用追踪证据。

第三轮首个 panic 前最后一次采样为 `MemFree=60976 KiB`、`Cached=853536 KiB`、`Dirty=832032 KiB`、`Slab=1015408 KiB`。随后出现：

```text
Kernel Panic Occurred. raw_pid: 174
File: src/mm/allocator/kernel_allocator.rs
Line: 247, Column: 5
global_alloc_error, layout: Layout { size: 4096, align: 1 (1 << 0) }
```

首个调用栈按调用方向整理为：

```text
creat 系统调用
  → open_utils::do_open
  → do_sys_openat2
  → MountFSInode::create_file_with_post_commit
  → LockedExt4Inode::create
  → Ext4::create_with_owner_and_attr
  → Ext4::create_inode_with_metadata
  → handle_alloc_error
  → global_alloc_err_handler
  → panic
```

本次首个失败位于普通文件创建路径，不是 `PageReclaimer::shrink_list` 的 Vec 扩容。串口中的后续分配 panic 不能与首个失败混为同一调用栈。

### B：静置回收缓慢，sync 后大部分资源释放

单轮 runner 返回 `rc=0`，输出 1622 ops/sec，调用耗时 48.903 秒。之后的 `sync` 返回 `rc=0`，耗时 **133.736 秒**，未触及 180 秒保护时限；全过程没有 panic。

除 inode 数外均为 KiB：

| 阶段 | IFree | Dirty | Cached | Slab | MemFree |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline | 124378 | 28 | 2928 | 14636 | 2045860 |
| runner 返回 | 104265 | 241384 | 246704 | 353508 | 1462900 |
| 静置 15 秒 | 104289 | 241096 | 246416 | 353236 | 1463460 |
| 静置 30 秒 | 104313 | 240808 | 246128 | 353028 | 1463956 |
| 静置 60 秒 | 104361 | 240232 | 245552 | 352584 | 1464976 |
| sync 返回 | 124377 | 0 | 5396 | 115644 | 1941840 |
| sync 后 30 秒 | 124377 | 0 | 5396 | 115632 | 1941852 |

静置 0、15、30、60 秒四个时点均满足：

```text
Dirty - baseline.Dirty = (baseline.IFree - IFree) × 12 KiB
```

60 秒自然等待只归还了 96 个 inode；随后 `sync` 期间归还了 20016 个 inode，Dirty 清零，MemFree 回升 476864 KiB。最终仍比基线少 104008 KiB 的 MemFree，其中 Slab 增量为 100996 KiB（约 98.63 MiB）。剩余一个 inode 的归属及这些内核堆页的组成尚未追踪。

## 推测结论

**现有证据支持资源回收滞后：正常运行和短时静置期间的回收推进跟不上负载，显式同步能推动大部分积压资源释放。** 本次尚不支持把 inode 与脏文件页的主要增长定性为永久泄漏；也不能据此排除剩余内核堆占用中的局部泄漏。

一个与现象一致、尚待运行时验证的原因链是：ext4 的脏 inode 队列持有 `Arc<LockedExt4Inode>` 和 `AsyncWork` 保留计数；最后一次 unlink 提交延迟回收请求，但保留计数尚未归零；脏元数据处理并释放保留后，最终回收才得以截断 PageCache 并归还 inode。

在上述源码提交中，可沿以下位置核对这条候选路径：

- `kernel/src/filesystem/ext4/filesystem.rs`：`mark_inode_dirty_if()` 增加保留并入队；`flush_dirty_inodes()` 处理队列；`sync_fs()` 刷新元数据并等待已提交的延迟回收。
- `kernel/src/filesystem/ext4/inode.rs`：`try_schedule_deferred_eviction()` 检查保留计数；`run_deferred_eviction()` 执行 `page_cache.truncate(0)` 和 inode 回收。
- `kernel/src/filesystem/vfs/inode_lifecycle.rs`：`try_begin_freeing()` 在仍有语义引用时拒绝最终回收。

需要保留以下结论边界：

1. **未定位具体瓶颈。** 还不能断定某个后台线程执行慢；触发频率、处理预算、写回等待和队列积压都需要进一步区分。`sync` 同时推动多个阶段，不能单独证明 `AsyncWork` 就是现场阻塞点。
2. **同步可完成不等于同步足够快。** 单次 `sync` 耗时约 134 秒，这个成本本身也值得定位。
3. **Slab 增长不等于活对象泄漏。** 尚未拆分 Zone 驻留页、大块内核堆分配、空闲槽位和碎片；同步后约 99 MiB 的增量仍需解释。
4. **分配失败与 panic 是另一层问题。** 已确认文件创建中的不可失败分配触发全局 panic；未证明 Buddy 分配算法错误，也未在本次现场验证完整 OOM 处置机制。
5. **没有全量验收结论。** 本目录覆盖 ext4 10 KiB 用例及一次静置／sync 对照，不代表全部 ext4、ramfs 或 LMbench 用例已通过；仍在进行的长时间自然静置实验未纳入本报告。

后续应围绕“后台写回／回收为什么推进缓慢、sync 推动了哪一段进展”继续取证。手动 `sync` 是本次因果排查的干预手段，不应被直接加入 runner 作为掩盖回收问题的处理方式。
