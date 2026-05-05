use crate::arch::interrupt::TrapFrame;
use crate::syscall::table::FormattedSyscallParam;
use crate::{
    arch::syscall::nr::SYS_SHMDT,
    mm::{mmu_gather::MmuGather, ucontext::AddressSpace, VirtAddr},
    syscall::table::Syscall,
};
use alloc::vec::Vec;
use syscall_table_macros::declare_syscall;
use system_error::SystemError;
pub struct SysShmdtHandle;

impl SysShmdtHandle {
    #[inline(always)]
    fn vaddr(args: &[usize]) -> VirtAddr {
        VirtAddr::new(args[0])
    }
}

impl Syscall for SysShmdtHandle {
    fn num_args(&self) -> usize {
        1
    }

    fn entry_format(&self, args: &[usize]) -> Vec<FormattedSyscallParam> {
        vec![FormattedSyscallParam::new(
            "vaddr",
            format!("{}", Self::vaddr(args).data()),
        )]
    }
    /// # SYS_SHMDT系统调用函数，用于取消对共享内存的连接
    ///
    /// ## 参数
    ///
    /// - `vaddr`:  需要取消映射的虚拟内存区域起始地址
    ///
    /// ## 返回值
    ///
    /// 成功：0
    /// 失败：错误码
    fn handle(&self, args: &[usize], _frame: &mut TrapFrame) -> Result<usize, SystemError> {
        let vaddr = Self::vaddr(args);
        let current_address_space = AddressSpace::current()?;
        let mut address_write_guard = current_address_space.write();

        // 获取vma
        let vma = address_write_guard
            .mappings
            .contains(vaddr)
            .ok_or(SystemError::EINVAL)?;

        // 判断vaddr是否为起始地址
        let region = {
            let guard = vma.lock();
            if guard.region().start() != vaddr || guard.shm_id().is_none() {
                return Err(SystemError::EINVAL);
            }
            *guard.region()
        };

        let vma = address_write_guard
            .mappings
            .remove_vma(&region)
            .ok_or(SystemError::EINVAL)?;

        // 解除映射并释放 VMA；Drop 路径会执行 close_once()，从而完成 detach 记账。
        {
            let _pt_edit = current_address_space.page_table_edit();
            let mut tlb = MmuGather::gather(&current_address_space);
            vma.unmap(&mut address_write_guard.user_mapper.utable, &mut tlb);
            tlb.finish();
        }

        return Ok(0);
    }
}

declare_syscall!(SYS_SHMDT, SysShmdtHandle);
