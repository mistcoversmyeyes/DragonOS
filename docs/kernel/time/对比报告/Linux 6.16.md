## 第二部分：Linux 6.16 高精度时钟源架构

### 2.1 核心架构组件

#### **1. Clocksource 框架**

**struct clocksource** (Linux 内核定义)：
```c
struct clocksource {
    cycle_t (*read)(struct clocksource *cs);
    int (*enable)(struct clocksource *cs);
    void (*disable)(struct clocksource *cs);
    const char *name;
    struct list_head list;
    int rating;
    cycle_t mask;
    u32 mult;
    u32 shift;
    u64 max_idle_ns;
    unsigned long flags;
    u64 max_adj;
};
```

**关键特性：**
- **动态注册与选择**：多个 clocksource 可同时注册
- **Rating 系统**：数值越高优先级越高
  - TSC: 300-400 (不稳定时降为 0)
  - HPET: 250
  - ACPI_PM: 200
  - Jiffies: 1
- **连续性检测**：通过 `CLOCK_SOURCE_IS_CONTINUOUS` 标志
- **高精度模式支持**：`CLOCK_SOURCE_VALID_FOR_HRES`

#### **2. Clockevent 设备框架**

**struct clock_event_device**：
```c
struct clock_event_device {
    void (*event_handler)(struct clock_event_device *);
    int (*set_next_event)(unsigned long evt, struct clock_event_device *);
    int (*set_next_ktime)(ktime_t expires, struct clock_event_device *);
    const char *name;
    struct device dev;
    unsigned int features;
    unsigned long max_delta_ns;
    unsigned long min_delta_ns;
    u32 mult;
    u32 shift;
    int rating;
    int irq;
    const struct cpumask *cpumask;
    enum clock_event_mode mode;
};
```

**关键功能：**
- **单次触发**：`set_next_event()` 设置下一次中断时间
- **周期模式**：传统的周期性 tick
- **Oneshot 模式**：支持 tickless (NO_HZ)
- **per-CPU 架构**：每个 CPU 有独立的 clockevent 设备

**x86_64 clockevent 设备**：
- **Local APIC Timer**：每个 CPU 一个，rating 最高
- **HPET**：备选方案
- **PIT**：兼容性备选

#### **3. Timekeeping 核心**

**struct timekeeper** (Linux)：
```c
struct timekeeper {
    struct clocksource *clock;
    cycle_t cycle_last;
    u64 xtime_sec;
    u64 xtime_nsec;
    u64 xtime_interval;
    s64 xtime_remainder;
    u32 raw_interval;
    u64 ntp_error;
    u32 ntp_error_shift;
    struct timespec wall_to_monotonic;
    u32 mult;
    u32 shift;
};
```

**关键机制：**
- **NTP 调整**：通过 `ntp_error` 微调频率
- **对数累积**：`logarithmic_accumulation` 优化性能
- **快速路径**：tk_core.seq 序列锁实现无锁读取

#### **4. 高精度定时器**

**struct hrtimer**：
```c
struct hrtimer {
    struct timerqueue_node node;
    ktime_t _softexpires;
    enum hrtimer_restart (*function)(struct hrtimer *);
    struct hrtimer_clock_base *base;
    u8 state;
    u8 is_rel;
    u8 is_hard;
};
```

**关键特性：**
- **纳秒精度**：基于 ktime_t (64位纳秒)
- **红黑树调度**：O(log n) 查找最近定时器
- **多种时钟源**：
  - `CLOCK_MONOTONIC`
  - `CLOCK_REALTIME`
  - `CLOCK_BOOTTIME`
  - `CLOCK_TAI`
- **抢占支持**：`CONFIG_PREEMPT_RT` 集成

### 2.2 x86_64 平台实现细节

#### **TSC Clocksource**

**关键实现** (Linux `arch/x86/kernel/tsc.c`)：

```c
static u64 tsc_read(struct clocksource *cs)
{
    return (u64)rdtsc_ordered();
}

static struct clocksource clocksource_tsc = {
    .name                   = "tsc",
    .rating                 = 300,
    .read                   = tsc_read,
    .mask                   = CLOCKSOURCE_MASK(64),
    .flags                  = CLOCK_SOURCE_IS_CONTINUOUS |
                              CLOCK_SOURCE_MUST_VERIFY,
    .archdata               = &tsc_cs_arch_data,
};
```

**TSC 特性检测**：
- **Invariant TSC**：CPUID.80000007H:EDX[8]
- **TSC deadline**：CPUID.1:ECX[24]
- **TSC adjust**：MSR 0x3B (IA32_TSC_ADJUST)

**频率校准**：
1. **早期校准**：使用 PIT
2. **后期校准**：使用 HPET/ACPI PM
3. **动态调整**：通过 `tsc_khz` 变量

**同步问题**：
- **SMP 同步**：每个 CPU 的 TSC 可能不同步
- **停止时钟**：C-state 会导致 TSC 暂停
- **频率变化**：老 CPU 的 TSC 会随频率变化

#### **HPET Clocksource & Clockevent**

**HPET 寄存器**：
- **General Capabilities**：数量、周期
- **General Configuration**：启用/禁用、legacy 替换
- **Main Counter**：64位递增计数器
- **Timer n Registers**：比较值、配置

**HPET clocksource**：
```c
static u64 hpet_read(struct clocksource *cs)
{
    return (u64)readl(hpet_virt_address + HPET_COUNTER);
}

static struct clocksource clocksource_hpet = {
    .name           = "hpet",
    .rating         = 250,
    .read           = hpet_read,
    .mask           = CLOCKSOURCE_MASK(64),
    .flags          = CLOCK_SOURCE_IS_CONTINUOUS,
};
```

**HPET clockevent**：
- 每个定时器可配置为周期或单次模式
- 支持周期性中断生成
- 优先级低于 Local APIC Timer

#### **Local APIC Timer (Clockevent)**

**实现特性**：
```c
static struct clock_event_device lapic_clockevent = {
    .name           = "lapic",
    .features       = CLOCK_EVT_FEAT_PERIODIC |
                      CLOCK_EVT_FEAT_ONESHOT,
    .set_next_event = lapic_next_event,
    .set_state_shutdown = lapic_timer_shutdown,
    .set_state_periodic = lapic_timer_set_periodic,
    .set_state_oneshot = lapic_timer_set_oneshot,
    .tick_resume    = lapic_timer_resume,
    .irq            = -1,
    .rating         = 100,
};
```

**三种模式**：
1. **One-shot**：一次性中断，支持 tickless
2. **Periodic**：固定频率 tick
3. **TSC-Deadline**：使用 TSC 作为 deadline（最优性能）

**TSC-Deadline 模式**：
```assembly
; 写入 IA32_TSC_DEADLINE MSR
mov msr, 0x000006E0
mov eax, deadline_low
mov edx, deadline_high
wrmsr
```

优势：
- 无需频繁访问 APIC 寄存器
- 更低的中断延迟
- 更精确的时间控制

### 2.3 核心算法

#### **Mult/Shift 缩放算法**

**Cycle 到纳秒**：
```
ns = (cycles * mult) >> shift
```

**计算 mult 和 shift** (Linux `clocks_calc_mult_shift`)：
```c
void clocks_calc_mult_shift(u32 *mult, u32 *shift,
                            u32 from, u32 to, u32 maxsec)
{
    u64 tmp;
    u32 sft, sftacc= 32;

    // 寻找最大的 shift，使得 mult < 2^32
    for (sft = 32; sft > 0; sft--) {
        tmp = (u64)to << sft;
        tmp += from / 2;
        do_div(tmp, from);
        if ((tmp >> sftacc) == 0)
            break;
    }
    *mult = tmp;
    *shift = sft;
}
```

**示例**：
- TSC 频率：2.4 GHz = 2400000000 Hz
- 目标：纳秒 (NSEC_PER_SEC = 1000000000)
- mult ≈ 429497
- shift ≈ 22

#### **Clocksource 选择算法**

**选择优先级**：
1. **用户指定**：`clocksource=bootarg`
2. **最高 rating**
3. **稳定性检查**：watchdog 验证
4. **功能需求**：高精度模式需要 `CLOCK_SOURCE_VALID_FOR_HRES`

**切换流程**：
```c
static void clocksource_select(void)
{
    // 1. 查找最高 rating
    list_for_each_entry(cs, &clocksource_list, list) {
        if (cs->rating > best->rating)
            best = cs;
    }

    // 2. 检查是否需要切换
    if (curr_clocksource != best) {
        // 3. 更新 timekeeper
        timekeeping_change_clocksource(best);
    }
}
```

#### **Watchdog 验证机制**

**工作原理**：
```c
static void clocksource_watchdog(unsigned long data)
{
    // 读取 watchdog clocksource
    wdnow = watchdog->read();

    // 读取被验证的 clocksource
    csnow = cs->read();

    // 转换为纳秒
    wd_nsec = cyc2ns(watchdog, wdnow - wd_last);
    cs_nsec = cyc2ns(cs, csnow - cs_last);

    // 检查偏差
    if (abs(cs_nsec - wd_nsec) > WATCHDOG_THRESHOLD)
        // 标记为 unstable
        cs->flags |= CLOCK_SOURCE_UNSTABLE;
}
```

**Watchdog 时钟源选择**：
- 优先级：HPET > ACPI PM > PIT
- 必须是连续的 (`CLOCK_SOURCE_IS_CONTINUOUS`)
- 不能是被验证的时钟源自己

#### **高精度时间读取（seqcount 优化）**

**快速路径** (Linux `ktime_get_real_fast_ns`)：
```c
u64 ktime_get_real_fast_ns(void)
{
    struct tk_read_base *tkr;
    u64 now;
    unsigned int seq;

    do {
        seq = raw_read_seqcount(&tk_core.seq);
        tkr = tk_core.tkr;

        now = tkr->base +
              ((clocksource_read(tkr->clock) - tkr->cycle_last)
               * tkr->mult) >> tkr->shift;
    } while (read_seqcount_retry(&tk_core.seq, seq));

    return now;
}
```

**优势**：
- 无锁读取（seqcount 实现）
- 仅在 writer 端使用锁
- reader 重试次数很少

---
