use alloc::vec::Vec;

use system_error::SystemError;

use crate::{
    arch::{interrupt::TrapFrame, syscall::nr::SYS_PTRACE},
    process::{ProcessFlags, ProcessManager},
    syscall::table::{FormattedSyscallParam, Syscall},
};

const PTRACE_TRACEME: usize = 0;

pub struct SysPtrace;

impl SysPtrace {
    #[inline(always)]
    fn request(args: &[usize]) -> usize {
        args[0]
    }

    #[inline(always)]
    fn pid(args: &[usize]) -> usize {
        args[1]
    }

    #[inline(always)]
    fn addr(args: &[usize]) -> usize {
        args[2]
    }

    #[inline(always)]
    fn data(args: &[usize]) -> usize {
        args[3]
    }
}

impl Syscall for SysPtrace {
    fn num_args(&self) -> usize {
        4
    }

    fn handle(&self, args: &[usize], _frame: &mut TrapFrame) -> Result<usize, SystemError> {
        match Self::request(args) {
            PTRACE_TRACEME => {
                let current = ProcessManager::current_pcb();
                if current.flags().contains(ProcessFlags::PTRACED) {
                    return Err(SystemError::EPERM);
                }
                current.flags().insert(ProcessFlags::PTRACED);
                Ok(0)
            }
            _ => Err(SystemError::ENOSYS),
        }
    }

    fn entry_format(&self, args: &[usize]) -> Vec<FormattedSyscallParam> {
        vec![
            FormattedSyscallParam::new("request", format!("{:#x}", Self::request(args))),
            FormattedSyscallParam::new("pid", format!("{:#x}", Self::pid(args))),
            FormattedSyscallParam::new("addr", format!("{:#x}", Self::addr(args))),
            FormattedSyscallParam::new("data", format!("{:#x}", Self::data(args))),
        ]
    }
}

syscall_table_macros::declare_syscall!(SYS_PTRACE, SysPtrace);
