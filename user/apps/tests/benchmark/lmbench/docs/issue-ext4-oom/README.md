# ext4 小文件写回／回收与内存不足 Issue 材料

[index.md](index.md) 是待审阅的 Issue 主体，只包含问题描述、验收条件和报告链接。实验方法、完整结果和探索过程保存在各实验报告中；[historical-panics.md](historical-panics.md) 保留迁移前的草稿及早期现场。

## 目录结构

```text
issue-ext4-oom/
├── README.md                         # 目录树与阅读说明
├── index.md                          # Issue：问题描述、验收条件、报告链接
├── historical-panics.md              # 原草稿中的历史现场与调用栈
└── evidence/
    ├── stress-run-experiment/
    │   ├── report.md                 # 连续压测：两轮成功，第三轮 panic
    │   ├── resource-trends.csv        # 基线及每轮结束后的资源数据
    │   ├── serial.log                # 完整原始串口
    │   ├── serial-readable.txt       # 去除控制字符的串口副本
    │   ├── first-panic.txt            # 首个 panic 摘录
    │   ├── manifest.json             # 实验版本、环境、参数与产物哈希
    │   ├── summary.json              # 轮次、退出状态、耗时
    │   └── validation.json           # 镜像未改、资源退出等验证记录
    ├── sync-reclaim-experiment/
    │   ├── report.md                 # 单轮负载 → 静置 → sync 对照
    │   ├── resource-trends.csv        # 各阶段的内存与 inode 数据
    │   ├── sync.log                  # sync 命令与完成标记
    │   ├── serial.log
    │   ├── serial-readable.txt
    │   ├── manifest.json
    │   ├── summary.json              # 含 sync 返回码及宿主计时
    │   └── validation.json
    └── background-pagecache-reclaim-experiment/
        ├── report.md                 # 长时自然回收，注明用户提前停止
        ├── inode-curve.csv           # 全部 563 个实测点及相邻采样速率
        ├── inode-curve.png           # 浏览用曲线
        ├── inode-curve.svg           # 矢量曲线
        ├── analysis.json             # 平均速率、拟合及采样完整性统计
        ├── serial.log
        ├── serial-readable.txt
        ├── manifest.json
        ├── summary.json
        └── validation.json
```

## 阅读入口

| 材料 | 阅读目的 |
| --- | --- |
| [Issue 主体](index.md) | 审阅问题范围和完成标准 |
| [连续压测报告](evidence/stress-run-experiment/report.md) | 查看资源积压、物理内存收支及首个 panic |
| [sync 对照报告](evidence/sync-reclaim-experiment/report.md) | 比较短时自然回收与显式同步后的资源变化 |
| [后台自然回收报告](evidence/background-pagecache-reclaim-experiment/report.md) | 查看 140.5 分钟的实测曲线和回收速率 |
| [历史现场](historical-panics.md) | 核对早期 ENOSPC 和不同分配失败调用栈 |

三组实验分别从全新 snapshot guest 开始，使用相同源码提交、内核和基础镜像，不是同一台 guest 上连续经历的三个阶段。各目录的 `manifest.json` 记录原 attempt 标识，标识中的时间为 UTC；报告中的墙钟开始时间使用北京时间，时间曲线使用宿主 monotonic 计时。

CSV、原始串口和图像按原实验文件逐字节归档；`serial-readable.txt` 是当时已经生成的可读副本，去除了 ANSI、CR 和 NUL。JSON 元数据移除了宿主文件系统路径，并补充由实际启动参数和预检记录核对的环境字段；测量数值、返回码和产物身份保持不变。原实验目录及历史记录仍保留，阅读本目录不需要访问那些宿主路径。

`background-pagecache-reclaim-experiment` 在用户要求下提前停止。它记录完整的已观测时段，不表示 inode 已全部回收；图中没有把线性外推当成实测数据。`sync` 对照中的显式同步是诊断干预，没有加入 runner。
