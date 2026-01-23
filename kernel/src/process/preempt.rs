use core::{intrinsics::likely, panic::Location};

use crate::process::{ProcessManager, __PROCESS_MANAGEMENT_INIT_DONE};

pub struct PreemptGuard;

impl PreemptGuard {
    pub fn new() -> Self {
        ProcessManager::preempt_disable();
        Self
    }
}

impl Drop for PreemptGuard {
    fn drop(&mut self) {
        ProcessManager::preempt_enable();
    }
}

impl ProcessManager {
    /// 增加当前进程的锁持有计数
    #[inline(always)]
    #[track_caller]
    pub fn preempt_disable() {
        if likely(unsafe { __PROCESS_MANAGEMENT_INIT_DONE }) {
            let pcb = ProcessManager::current_pcb();
            if pcb.preempt_count() == 1 {
                let loc = Location::caller() as *const Location;
                super::LAST_PREEMPT_DISABLE_SITE.store(loc as usize, core::sync::atomic::Ordering::Relaxed);
                super::LAST_PREEMPT_DISABLE_PID.store(
                    pcb.raw_pid().data(),
                    core::sync::atomic::Ordering::Relaxed,
                );
            }
            pcb.preempt_disable();
        }
    }

    /// 减少当前进程的锁持有计数
    #[inline(always)]
    pub fn preempt_enable() {
        if likely(unsafe { __PROCESS_MANAGEMENT_INIT_DONE }) {
            ProcessManager::current_pcb().preempt_enable();
        }
    }
}
