use crate::syscall::table::Syscall;
use crate::arch::syscall::nr::SYS_2333;
use log::info;
use alloc::vec::Vec;
use crate::syscall::table::FormattedSyscallParam;

pub struct Sys2333Handle;

impl Syscall for Sys2333Handle
{
    fn num_args(&self) -> usize {
        0
    }

    fn handle(&self, args: &[usize], frame: &mut crate::arch::interrupt::TrapFrame) -> Result<usize, system_error::SystemError> {
        info!("syscall 2333 called");
        Ok(6666)
    }

    fn entry_format(&self, args: &[usize]) -> Vec<FormattedSyscallParam> {
        vec![]
    }
}

syscall_table_macros::declare_syscall!(SYS_2333, Sys2333Handle);