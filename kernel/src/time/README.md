# DragonOS 时间子系统（结构化拆解）

本文档按“三步走”策略拆解时间子系统：静态视角 → 动态视角 → 分配视角。
本次先完成静态视角的类图（完整版与简化版）。

## 第一步：静态视角（Module Structure）

### 类图（完整）

```plantuml
@startuml
skinparam classAttributeIconSize 0

interface Clocksource {
  +read(): CycleNum
  +enable(): Result
  +disable(): Result
  +clocksource_data(): ClocksourceData
}
class ClocksourceData {
  +name: String
  +rating: i32
  +flags: ClocksourceFlags
  +mult: u32
  +shift: u32
}
class ClocksouceWatchdog {
  -watchdog: Option<Clocksource>
  -is_running: bool
}
class WatchdogTimerFunc
class CycleNum

class ClocksourceJiffies
class Timekeeper {
  -inner: RwLock<TimekeeperData>
  -new()
  +timekeeper_setup_internals(clock)
  +timekeeping_get_ns()
  +timekeeping_bigadjust(error, interval, offset)
  +timekeeping_adjust(offset)
  +logarithmic_accumulation(offset, shift)
}
class TimekeeperData {
  -clock: Option<Clocksource>
  -shift: i32
  -cycle_interval: CycleNum
  -xtime_interval: u64
  -xtime_nsec: u64
  -ntp_error: i64
  -ntp_error_shift: i32
  -mult: u32
  -xtime: PosixTimeSpec
  -wall_to_monotonic: PosixTimeSpec
  -raw_time: PosixTimeSpec
  -total_sleep_time: PosixTimeSpec
  -real_time_offset: ktime_t
  -new()
}

interface TimerFunction {
  +run(): Result
}
class Timer
class InnerTimer {
  +expire_jiffies: u64
  +timer_func: Option<Box<dyn TimerFunction>>
  -self_ref: Weak<Timer>
  +triggered: bool
}
class Jiffies
class DoTimerSoftirq

class ClocksourceAPI <<utility>> {
  +{static} clocksource_select()
  +{static} clocksource_boot_finish()
}
class TimekeepingAPI <<utility>> {
  +{static} timekeeping_init()
  +{static} getnstimeofday()
  +{static} update_wall_time()
}
class TimerAPI <<utility>> {
  +{static} update_timer_jiffies()
  +{static} clock()
}

class ClocksourceRegistry <<static>> {
  +CLOCKSOURCE_LIST
  +WATCHDOG_LIST
  +CUR_CLOCKSOURCE
  +FINISHED_BOOTING
}

ClocksourceJiffies ..|> Clocksource
Clocksource *-- ClocksourceData
ClocksouceWatchdog --> Clocksource : select/watch
WatchdogTimerFunc ..|> TimerFunction
ClocksouceWatchdog --> Timer : watchdog timer

Timekeeper *-- TimekeeperData
Timekeeper --> Clocksource : current clock
TimekeepingAPI ..> Timekeeper : module-level
ClocksourceAPI ..> ClocksourceRegistry : module-level

Timer *-- InnerTimer
Timer --> TimerFunction
Timer --> Jiffies
DoTimerSoftirq --> Timer
TimerAPI ..> Timer : module-level
TimerAPI ..> TimekeepingAPI : update wall time

note right of TimerFunction
  Timer 到期后调用 run()
end note

note right of DoTimerSoftirq
  TIMER softirq 处理器：不可重入地
  从 TIMER_LIST 取到期定时器并执行
end note

note right of InnerTimer
  无公开方法，主要由 Timer 访问
end note

note right of TimekeepingAPI
  模块级辅助函数：对外时间接口
end note

ClocksourceRegistry --> Clocksource

@enduml
```

### 类图（简化）

```plantuml
@startuml
skinparam classAttributeIconSize 0

interface Clocksource {
  +read(): CycleNum
  +clocksource_data(): ClocksourceData
}
class Timekeeper
class TimekeepingAPI <<utility>>
interface TimerFunction {
  +run(): Result
}
class Timer
class TimerAPI <<utility>>
class TickCommon
class ClocksourceWatchdog
class DoTimerSoftirq
class Jiffies

TimekeepingAPI ..> Timekeeper : module-level
Timekeeper --> Clocksource : time base
Timer --> TimerFunction : callback
TimerAPI ..> Timer : module-level
TimerAPI ..> TimekeepingAPI : update wall time
TickCommon --> Timer : periodic tick
DoTimerSoftirq --> Timer : run expired timers
ClocksourceWatchdog --> Timer : schedule check
ClocksourceWatchdog --> Clocksource : validate/reselect
Timer --> Jiffies : tick count

@enduml
```

### 包图（完整）

```plantuml
@startuml
skinparam packageStyle rectangle

package "time/mod.rs" as mod
package "clocksource.rs" as cs
package "jiffies.rs" as jf
package "timekeeping.rs" as tk
package "timekeep.rs" as tkp
package "timer.rs" as tr
package "tick_common.rs" as tc
package "sleep.rs" as sl
package "timeconv.rs" as tcv
package "syscall/" as sc
package "driver/rtc" as rtc

mod ..> cs : 声明模块
mod ..> jf : 声明模块
mod ..> tk : 声明模块
mod ..> tkp : 声明模块
mod ..> tr : 声明模块
mod ..> tc : 声明模块
mod ..> sl : 声明模块
mod ..> tcv : 声明模块
mod ..> sc : 声明模块

cs ..> tr : watchdog timer
jf ..> cs : clocksource impl
tk ..> cs : time base
tk ..> jf : default clock
tk ..> tkp : ktime
tk ..> sc : PosixTimeval
tr ..> jf : NSEC_PER_JIFFY
tr ..> tk : update wall time
tc ..> tr : periodic tick
sl ..> tr : nanosleep timer
sl ..> tk : getnstimeofday
tcv ..> sc : PosixTimeT
sc ..> tk : posix clock/time
sc ..> sl : nanosleep
sc ..> tr : timer syscalls
tkp ..> rtc : read time

@enduml
```

## 第二步：动态视角（C&C）

### 时序图 1：启动与 watchdog 线程初始化（模块级）

```plantuml
@startuml
actor Boot
participant "start_kernel" as SK
participant "TimekeepingAPI\n<<utility>>" as TK
participant "TimerAPI\n<<utility>>" as TMR
participant "KthreadAPI\n<<utility>>" as KTI
participant "ClocksourceAPI\n<<utility>>" as CSA
participant "initial_kernel_thread" as IKT
participant "do_initcalls" as DIC
participant "init_watchdog_kthread" as IWK
participant "KernelThreadMechanism" as KTM
participant "watchdog_kthread" as WK

Boot -> SK : start_kernel()
activate SK
SK -> TK : timekeeping_init()
SK -> TMR : timer_init()
SK -> KTI : kthread_init()
SK -> CSA : clocksource_boot_finish()
SK -> IKT : schedule initial thread
activate IKT
IKT -> DIC : do_initcalls()
activate DIC
DIC -> IWK : init_watchdog_kthread()
activate IWK
IWK -> KTM : create_and_run()
activate KTM
KTM -> WK : create + wakeup
deactivate KTM
deactivate IWK
deactivate DIC
deactivate IKT
deactivate SK
@enduml
```

### 时序图 2：tick 驱动定时器与软中断

```plantuml
@startuml
participant "HardwareTimerIRQ" as IRQ
participant "TickCommon" as TC
participant "TimerAPI\n<<utility>>" as TMR
participant "TimekeepingAPI\n<<utility>>" as TK
participant "Softirq(TIMER)" as SIRQ
participant "DoTimerSoftirq" as DTS
participant "Timer" as T
participant "TimerFunction" as TF

IRQ -> TC : tick_handle_periodic()
TC -> TMR : update_timer_jiffies(1)
TMR -> TK : update_wall_time()
TC -> TMR : run_local_timer()
TMR -> TMR : try_raise_timer_softirq()
TMR -> SIRQ : raise_softirq(TIMER)
SIRQ -> DTS : run()
alt 有到期定时器
  DTS -> T : run()
  T -> TF : run()
else 无到期定时器
  DTS -> DTS : return
end
@enduml
```

### 时序图 3：watchdog 检测与不稳定时钟源处理

```plantuml
@startuml
participant "ClocksourceWatchdog" as CWD
participant "TimerAPI\n<<utility>>" as TMR
participant "Timer" as T
participant "WatchdogTimerFunc" as WTF
participant "clocksource_watchdog" as CSW
participant "Clocksource" as CS
participant "ClocksourceAPI\n<<utility>>" as CSA
participant "watchdog_kthread" as WK

CWD -> TMR : schedule watchdog timer
TMR -> T : Timer::new(WatchdogTimerFunc)
T -> WTF : run()
WTF -> CSW : clocksource_watchdog()
CSW -> CS : read + compare
alt 偏差过大
  CSW -> CS : set_unstable()
  CSW -> WK : run_watchdog_kthread()
  WK -> CSA : clocksource_select()
else 偏差正常
  CSW -> CWD : keep watchdog running
end
@enduml
```

## 第三步：分配视角（Allocation）

待补充。
