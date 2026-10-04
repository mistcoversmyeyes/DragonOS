# ext4_create_delete_files_10k_ops 连续压测

[返回 Issue](../../index.md) · [材料目录](../../README.md)

## 目的与环境

观察同一 guest 连续运行原生 `ext4_create_delete_files_10k_ops` 时，runner 返回后的 inode、文件后备页和内核堆物理占用是否累积，以及出现异常时的完整现场。

| 项目 | 配置 |
| --- | --- |
| 原记录 | `attempt-20261002T165926Z` |
| 开始时间 | 2026-10-03 00:59:26，北京时间 |
| 源码提交 | `3ffc8327522dbd650505015202dce35fee545ef8` |
| 分支 | `experiment/lmbench-ext4-pressure-20261003` |
| 平台 | DragonOS x86_64，QEMU/KVM，2 vCPU，2 GiB RAM |
| Guest 内存统计 | `MemTotal=2069836 KiB` |
| 磁盘 | 独立 2 GiB 基础镜像，`snapshot=on` |
| 文件系统布局 | `/ext4` 是根 ext4 上的普通目录；`/tmp` 是 tmpfs；没有额外挂载 1 GiB loop fixture |
| 负载 | 每个文件写入 10 KiB 后关闭、删除；`10k` 是文件大小，不是操作次数 |
| 校准 | 原生 runner 自动校准，三轮均为 `ENOUGH=1000000` |

内核和基础镜像 SHA256 记录于 [manifest.json](manifest.json)。本次与独立 ramfs VM 并行启动，性能输出仅作为测量成功记录，不用于比较文件系统性能。

## 方法

在 guest 中重复执行以下命令，计划最多 20 轮，遇到首个异常停止：

```sh
cd /opt/tests/benchmark/lmbench
sh run.sh --only ext4_create_delete_files_10k_ops \
  --samples 1 --timeout 120 --warmup 0
```

初始化、校准、测量和清理由 runner 负责；没有直接调用 `lat_fs`，没有额外执行 `sync` 或 `drop_caches`。每轮包含一次完整 runner 调用，不是一次初始化后的 20 个 sample。

首次 runner 之前读取基线，每轮返回后立即采集阶段数据：

```sh
cat /proc/meminfo
df -k / /ext4 /tmp
df -i / /ext4 /tmp
```

串口用 `__OBS_baseline__`、`__OBS_after_N__` 等标记分段。另有每次读取后等待 5 秒的后台 `/proc/meminfo` 采样，标记为 `__MEM_TICK__`。阶段读取不是原子快照，也没有等待后台回收稳定；CSV 中的三个阶段已核对没有周期采样输出交错。第三轮 panic 前的数据来自最后一次周期采样，不代表分配失败瞬间的精确状态。

## 结果

| 轮次 | 结果 | 性能输出 | Runner 调用耗时 |
| --- | --- | ---: | ---: |
| 1 | `rc=0`，有效结果 | 1441 ops/sec | 56.774 秒 |
| 2 | `rc=0`，有效结果 | 3117 ops/sec | 46.352 秒 |
| 3 | 文件创建时 panic，未正常返回 | 无完整结果 | 中断 |

原计划 20 轮，实际完成两轮，第三轮中断。此次没有出现 `No space left on device`，宿主机 OOM 计数没有增加。

除 inode 数外，以下单位均为 KiB：

| 阶段 | MemFree | Cached | Dirty | Slab | IFree | 磁盘可用空间 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| baseline | 2045868 | 2928 | 28 | 14624 | 124378 | 1450132 |
| after_1 | 1372544 | 286028 | 280672 | 404400 | 100991 | 1450128 |
| after_2 | 637012 | 605856 | 600460 | 819996 | 74342 | 1450128 |

第二轮相对基线的物理内存收支基本闭合：

```text
MemFree 减少 = 1408856 KiB（1375.84 MiB）
Cached  增加 =  602928 KiB（ 588.80 MiB）
Slab    增加 =  805372 KiB（ 786.50 MiB）
Cached + Slab 增量 = 1408300 KiB，占 MemFree 减少量的 99.96%
差额 = 556 KiB
```

两个阶段的脏页增量均与空闲 inode 减少量精确对应：

| 阶段 | IFree 相对基线减少 | Dirty 相对基线增加 | 对应关系 |
| --- | ---: | ---: | --- |
| after_1 | 23387 | 280644 KiB | 23387 × 12 KiB |
| after_2 | 50036 | 600432 KiB | 50036 × 12 KiB |

10 KiB 文件涉及三个 4 KiB 页；该对应关系支持 inode 和文件数据一起滞留，但不是逐文件引用追踪证据。

第三轮首个 panic 前最后一次采样为 `MemFree=60976 KiB`、`Cached=853536 KiB`、`Dirty=832032 KiB`、`Slab=1015408 KiB`。随后首个 panic 为：

```text
Kernel Panic Occurred. raw_pid: 174
File: src/mm/allocator/kernel_allocator.rs
Line: 247, Column: 5
global_alloc_error, layout: Layout { size: 4096, align: 1 (1 << 0) }
```

首个调用栈按调用方向整理：

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

此栈确认普通文件创建路径的 4096 字节申请失败，不足以进一步定位到该函数内的某一个具体分配表达式。后续 panic 与首个失败分别记录，不能混为同一个调用栈。

## 统计口径与结论边界

- `Cached` 是此版本的文件后备页统计，扣除了 shmem；`Dirty` 是脏页状态统计，不能作为独立内存池再次与 `Cached` 相加。
- `Slab` 是 Zone 持有的全部 slab 页加上 `LARGE_ALLOCATION_BYTES`。它包含空闲槽位、部分使用页和大块内核堆分配，不等于存活对象有效大小，也不是 Buddy 全部用途的分配总量。
- `/` 和 `/ext4` 在同一个文件系统，`df` 中两行 ext4 数据不能相加；CSV 记录根 ext4 统计。
- 本次证明了连续负载后的资源累积及文件创建分配 panic，未证明 Buddy 算法错误、全部残留资源永久泄漏或 ENOSPC 的共同根因。
- 静置和同步能否释放积压，由另一台全新 guest 的 [sync 对照](../sync-reclaim-experiment/report.md)及[长时自然回收实验](../background-pagecache-reclaim-experiment/report.md)单独验证。

## 附件与完整性

- [资源 CSV](resource-trends.csv)、[完整原始串口](serial.log)、[可读串口](serial-readable.txt)、[首个 panic](first-panic.txt)。
- [环境与产物身份](manifest.json)、[轮次结果](summary.json)、[退出及完整性核验](validation.json)。

两份串口和 CSV 继承原实验记录；原仓库归档的可读串口与 CSV 均与本目录对应附件逐字节一致。原始字节串口 SHA256 为 `1838211adfafbb175e8d102b705f2e9a6cc78367e8045c0d42603f151257472e`。本次 QEMU 已终止，基础镜像前后 SHA256 相同。旧报告中的静置／sync 内容已拆入独立报告，不与本次压测混成同一现场。
