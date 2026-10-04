# bug(mm): lmbench `ext4_create_delete_files_10k_ops` triggers oom and kernel allocation panics

关联总跟踪：[LMbench blocking issues #2286](https://github.com/DragonOS-Community/DragonOS/issues/2286)。本文为待审阅的 Issue 正文草稿。

## 问题描述

### 大量小文件的后台写回与回收吞吐不足

在 DragonOS x86_64、2 vCPU、2 GiB RAM 的 guest 中，通过原生 LMbench runner 连续运行 `ext4_create_delete_files_10k_ops`，每个文件写入 10 KiB 数据后关闭、删除。runner 返回并完成清理后，待写回的文件数据、尚未归还的 inode 和内核堆物理占用仍大量积压；持续运行使空闲物理内存显著下降，最终在文件创建路径出现内核分配失败并 panic。

单轮结束后的自然回收持续推进，但吞吐不足以及时消化积压；显式 `sync` 能使大部分 inode 和脏页释放。需要使后台写回与最终回收能够及时处理该负载，避免持续的小文件创建／删除活动耗尽可用内存。

### 内存不足时内核分配失败会引发全局 panic

运行期内存紧张时，普通文件创建路径的内核堆分配失败进入 `global_alloc_error`，导致全局 panic。历史现场还记录了后台页面回收和用户缺页直接回收过程中，收集待回收页面引用的数组扩容失败后 panic。

内核应能将运行期可处置的内存不足控制为明确的请求失败或进程处置，并保留回收及恢复能力；不应因回收路径自身的辅助分配失败，使整个系统进入 panic／halt。

## 验收条件

### 后台写回与回收

- [ ] 在相同 2 GiB guest 环境，通过原生 runner 连续完成至少 20 轮 `ext4_create_delete_files_10k_ops` 单采样；保持 `--samples 1 --timeout 120 --warmup 0`、10 KiB 文件大小和自动校准行为。
- [ ] 在同一 guest 中完成 `--samples 5 --timeout 120 --warmup 0` 的多采样运行，所有 sample 均有有效结果；不能通过缩小负载、延长 sample 超时或忽略失败实现通过。
- [ ] 上述持续负载不依赖额外手动 `sync`、`drop_caches` 或重启；不因回收积压触发 ENOSPC、内核分配 panic 或系统失响应。
- [ ] 提供负载前、运行中及结束后的 `IFree`、`Dirty`、`Cached`、`Slab`、`MemFree` 时间序列，证明资源占用不会随已完成轮次无界增加；给出停止负载后的自然回收曲线和稳定状态。
- [ ] 相同单轮负载的自然回收吞吐和积压排空时间相对本 Issue 的基线明显改善，不再需要数小时消化单轮积压；不要求分配器统计在结束瞬间与启动基线完全相同。

### 内存不足时的稳定性与恢复

- [ ] 覆盖普通文件创建、用户缺页直接回收和后台回收三条运行期内存不足路径；分配失败有可识别的错误或进程终止结果，不引发全局 panic。
- [ ] 在回收器自身的辅助内存申请失败时，系统仍能报告失败或继续推进回收，不发生 panic、无限重试或永久阻塞。
- [ ] 释放负载资源或完成必要的 OOM 进程处置后，系统能够恢复响应，并再次完成文件创建、读写和删除；保留恢复前后的指标、错误及完整串口。
- [ ] 验证同时包含回收吞吐改善和内存不足失败处置；不能仅凭增大 VM 内存、降低测试负载或避免触发内存压力关闭问题。

## 实验报告

实验方案、数据、曲线、调用栈和探索过程由下列报告承载，本文暂不 inline：

- [连续压测：两轮成功，第三轮文件创建时 panic](evidence/stress-run-experiment/report.md)。
- [单轮静置与显式 sync 对照](evidence/sync-reclaim-experiment/report.md)。
- [140.5 分钟后台自然回收时间曲线](evidence/background-pagecache-reclaim-experiment/report.md)。
- [历史文件创建、后台回收及直接回收 panic 现场](historical-panics.md)。

附件目录和阅读说明见 [README.md](README.md)。
