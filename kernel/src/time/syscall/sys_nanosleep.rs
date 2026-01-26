use crate::arch::interrupt::TrapFrame;
use crate::arch::syscall::nr::SYS_NANOSLEEP;
use crate::ipc::signal::{RestartBlock, RestartBlockData};
use crate::process::ProcessManager;
use crate::syscall::table::{FormattedSyscallParam, Syscall};
use crate::syscall::user_access::{UserBufferReader, UserBufferWriter};
use crate::time::sleep::nanosleep;
use crate::time::timekeeping::getnstimeofday;
use crate::time::PosixTimeSpec;
use alloc::vec::Vec;
use system_error::SystemError;

use super::PosixClockID;

pub struct SysNanosleep;

impl SysNanosleep {
    fn sleep_time(args: &[usize]) -> *const PosixTimeSpec {
        args[0] as *const PosixTimeSpec
    }

    fn rm_time(args: &[usize]) -> *mut PosixTimeSpec {
        args[1] as *mut PosixTimeSpec
    }

    #[inline]
    fn is_valid_timespec(ts: &PosixTimeSpec) -> bool {
        ts.tv_sec >= 0 && ts.tv_nsec >= 0 && ts.tv_nsec < 1_000_000_000
    }

    #[inline]
    fn ktime_now() -> PosixTimeSpec {
        // 与 clock_nanosleep 的 Monotonic/Boottime 处理保持一致（当前用 getnstimeofday 近似）
        getnstimeofday()
    }

    #[inline]
    fn calc_remaining(deadline: &PosixTimeSpec, now: &PosixTimeSpec) -> PosixTimeSpec {
        let mut sec = deadline.tv_sec - now.tv_sec;
        let mut nsec = deadline.tv_nsec - now.tv_nsec;
        if nsec < 0 {
            sec -= 1;
            nsec += 1_000_000_000;
        }
        if sec < 0 {
            return PosixTimeSpec {
                tv_sec: 0,
                tv_nsec: 0,
            };
        }
        PosixTimeSpec {
            tv_sec: sec,
            tv_nsec: nsec,
        }
    }

    #[inline]
    fn add_timespec(a: &PosixTimeSpec, b: &PosixTimeSpec) -> PosixTimeSpec {
        let mut sec = a.tv_sec + b.tv_sec;
        let mut nsec = a.tv_nsec + b.tv_nsec;
        if nsec >= 1_000_000_000 {
            sec += 1;
            nsec -= 1_000_000_000;
        }
        PosixTimeSpec {
            tv_sec: sec,
            tv_nsec: nsec,
        }
    }
}

impl Syscall for SysNanosleep {
    fn num_args(&self) -> usize {
        2
    }

    fn handle(&self, args: &[usize], _frame: &mut TrapFrame) -> Result<usize, SystemError> {
        let sleep_time_reader = UserBufferReader::new(
            Self::sleep_time(args),
            core::mem::size_of::<PosixTimeSpec>(),
            true,
        )?;
        let rm_time_ptr = Self::rm_time(args);
        let mut rm_time_writer = if !rm_time_ptr.is_null() {
            Some(UserBufferWriter::new(
                rm_time_ptr,
                core::mem::size_of::<PosixTimeSpec>(),
                true,
            )?)
        } else {
            None
        };

        let sleep_time = sleep_time_reader.read_one_from_user::<PosixTimeSpec>(0)?;
        if !Self::is_valid_timespec(sleep_time) {
            return Err(SystemError::EINVAL);
        }

        let rq = PosixTimeSpec {
            tv_sec: sleep_time.tv_sec,
            tv_nsec: sleep_time.tv_nsec,
        };

        // nanosleep 语义：相对睡眠，基于绝对 deadline 进行重启
        let now = Self::ktime_now();
        let deadline = Self::add_timespec(&now, &rq);

        let wait_res = {
            let remain = Self::calc_remaining(&deadline, &now);
            if remain.tv_sec == 0 && remain.tv_nsec == 0 {
                Ok(())
            } else {
                nanosleep(remain).map(|_| ())
            }
        };

        match wait_res {
            Ok(()) => Ok(0),
            Err(_e) => {
                // 信号打断：写回剩余时间，并设置 restart block
                if let Some(ref mut rm_time) = rm_time_writer {
                    let now = Self::ktime_now();
                    let remain = Self::calc_remaining(&deadline, &now);
                    rm_time.copy_one_to_user(&remain, 0)?;
                }
                let data = RestartBlockData::Nanosleep {
                    deadline,
                    clockid: PosixClockID::Monotonic,
                };
                let rb = RestartBlock::new(&crate::ipc::signal::RestartFnNanosleep, data);
                ProcessManager::current_pcb().set_restart_fn(Some(rb))
            }
        }
    }

    fn entry_format(&self, args: &[usize]) -> Vec<FormattedSyscallParam> {
        vec![
            FormattedSyscallParam::new(
                "sleep_time",
                format!("{:#x}", Self::sleep_time(args) as usize),
            ),
            FormattedSyscallParam::new("rm_time", format!("{:#x}", Self::rm_time(args) as usize)),
        ]
    }
}

syscall_table_macros::declare_syscall!(SYS_NANOSLEEP, SysNanosleep);
