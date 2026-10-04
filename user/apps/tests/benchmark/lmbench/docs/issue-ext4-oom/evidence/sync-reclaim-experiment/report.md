# ext4_create_delete_files_10k_ops 静置与 sync 回收对照

[返回 Issue](../../index.md) · [材料目录](../../README.md)

## 目的与环境

比较原生单轮负载结束后的自然回收与单独执行一次 `sync` 后的资源状态，检查连续压测中发现的积压是否仍可释放。

原记录为 `attempt-idle-sync-20261003T150952Z`，开始时间为 2026-10-03 23:09:52（北京时间）。从全新 snapshot guest 开始，源码为 `3ffc8327522dbd650505015202dce35fee545ef8`，使用与[连续压测](../stress-run-experiment/report.md)相同的内核和基础镜像。平台为 x86_64、KVM、2 vCPU、2 GiB RAM；`/ext4` 是根 ext4 的普通目录，`/tmp` 为 tmpfs。

## 方法

通过原生 runner 执行一轮：

```sh
cd /opt/tests/benchmark/lmbench
sh run.sh --only ext4_create_delete_files_10k_ops \
  --samples 1 --timeout 120 --warmup 0
```

保持文件大小、自动校准和 timeout 不变。runner 返回后立即采样，再在静置 15、30、60 秒时采样；期间不执行其他清理。随后单独运行一次 `sync`，记录返回码和宿主 monotonic 耗时，返回后立即及再等待 30 秒采样。

`sync` 的宿主保护时限为 180 秒，它是 sample 结束后的诊断干预，不属于 benchmark 计时，也没有写入 runner。采样依次读取 `/proc/meminfo`、`df -k / /ext4 /tmp` 和 `df -i / /ext4 /tmp`，并使用独立阶段标记；读取不是原子快照。

## 结果

runner 自动校准得到 `ENOUGH=1000000`，返回 `rc=0`，输出 1622 ops/sec，调用耗时 48.903 秒。`sync` 返回 **0**，耗时 **133.736 秒**，未触及保护时限；全过程没有 panic。

除 inode 数外，单位均为 KiB：

| 阶段 | IFree | Dirty | Cached | Slab | MemFree |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline | 124378 | 28 | 2928 | 14636 | 2045860 |
| runner 返回 | 104265 | 241384 | 246704 | 353508 | 1462900 |
| 静置 15 秒 | 104289 | 241096 | 246416 | 353236 | 1463460 |
| 静置 30 秒 | 104313 | 240808 | 246128 | 353028 | 1463956 |
| 静置 60 秒 | 104361 | 240232 | 245552 | 352584 | 1464976 |
| sync 返回 | 124377 | 0 | 5396 | 115644 | 1941840 |
| sync 后 30 秒 | 124377 | 0 | 5396 | 115632 | 1941852 |

静置 0、15、30、60 秒的四个时点均满足：

```text
Dirty - baseline.Dirty = (baseline.IFree - IFree) × 12 KiB
```

60 秒自然等待只归还了 **96 个 inode**；随后 `sync` 期间归还了 **20016 个**，Dirty 清零，MemFree 增加 **476864 KiB**。最终 IFree 比基线少一个；MemFree 比基线少 104008 KiB，其中 Slab 增量为 100996 KiB，约 **98.63 MiB**。

## 观察与边界

大部分积压仍可通过同步路径释放，短时自然回收确有进展但吞吐很低。因此，本次不支持把主要 inode 和脏文件页积压定性为永久泄漏。剩余一个 inode 的归属和约 99 MiB 内核堆增量尚未追踪；Slab 包含分配器驻留页，不能直接将它解释为活对象泄漏。

`sync` 同时推动数据写回、元数据处理和已提交的最终回收；结果不能单独证明具体是哪个持有者或队列造成等待。成功返回也不表示该路径足够快：这次诊断同步本身耗时约 134 秒。

此版本 `sync()` 会遍历各 superblock 的脏 PageCache 并执行元数据同步。后台自然写回的每轮派发上限与周期，则由[长时自然回收报告](../background-pagecache-reclaim-experiment/report.md)结合曲线单独讨论。手动 `sync` 只用于验证积压的可回收性，不作为修复后运行用例的前提。

## 附件与完整性

- [阶段 CSV](resource-trends.csv)、[sync 输出](sync.log)、[完整原始串口](serial.log)、[可读串口](serial-readable.txt)。
- [环境与产物身份](manifest.json)、[运行及 sync 计时](summary.json)、[完整性验证](validation.json)。

七个阶段标记各出现一次，原生 runner 和 `sync` 各执行一次。`serial.log` 为完整 9823 字节 QEMU 串口；宿主接收流少了最后一个彩色 shell 提示符，不影响实验数据。QEMU 已退出，基础镜像和内核前后 SHA256 相同。
