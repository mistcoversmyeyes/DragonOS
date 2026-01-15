use crate::arch::interrupt::TrapFrame;
use crate::mm::page::PageFlushAll;
use crate::syscall::table::FormattedSyscallParam;
use crate::{
    arch::syscall::nr::SYS_SHMDT,
    arch::MMArch,
    mm::{ucontext::AddressSpace, VirtAddr},
    process::ProcessManager,
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

        let (shm_id, shm_attach) = {
            let vma_guard = vma.lock();
            // 判断vaddr是否为起始地址
            if vma_guard.region().start() != vaddr {
                return Err(SystemError::EINVAL);
            }
            // 仅允许 shm 映射执行 shmdt
            let shm_id = vma_guard.shm_id().ok_or(SystemError::EINVAL)?;
            let shm_attach = vma_guard.shm_attach().ok_or(SystemError::EINVAL)?;
            (shm_id, shm_attach)
        };

        let vmas: Vec<_> = address_write_guard
            .mappings
            .iter_vmas()
            .filter_map(|vma| {
                let vma_guard = vma.lock();
                if vma_guard.shm_id() == Some(shm_id) && vma_guard.shm_attach() == Some(shm_attach)
                {
                    Some(vma.clone())
                } else {
                    None
                }
            })
            .collect();

        if vmas.is_empty() {
            return Err(SystemError::EINVAL);
        }

        // 取消映射
        let mut flusher: PageFlushAll<MMArch> = PageFlushAll::new();
        for vma in vmas {
            let region = { *vma.lock().region() };
            vma.unmap(&mut address_write_guard.user_mapper.utable, &mut flusher);
            address_write_guard.mappings.remove_vma(&region);
        }

        let ipcns = ProcessManager::current_ipcns();
        let mut shm_manager_guard = ipcns.shm.lock();
        shm_manager_guard.detach_shm(shm_id)?;

        return Ok(0);
    }
}

declare_syscall!(SYS_SHMDT, SysShmdtHandle);
