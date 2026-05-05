use crate::alloc::vec::Vec;
use crate::arch::interrupt::TrapFrame;
use crate::syscall::table::FormattedSyscallParam;
use crate::{
    arch::syscall::nr::SYS_SHMAT,
    arch::MMArch,
    ipc::shm::{ShmFlags, ShmId},
    libs::align::page_align_up,
    mm::{
        allocator::page_frame::{PageFrameCount, PhysPageFrame, VirtPageFrame},
        mmu_gather::MmuGather,
        page::{page_manager_lock, EntryFlags, Flusher, PageFlushAll},
        syscall::ProtFlags,
        ucontext::{AddressSpace, LockedVMA, VMA},
        VirtAddr, VirtRegion, VmFlags,
    },
    process::ProcessManager,
    syscall::{table::Syscall, user_access::UserBufferReader},
};
use alloc::sync::Arc;
use syscall_table_macros::declare_syscall;
use system_error::SystemError;
pub struct SysShmatHandle;

fn map_sysv_shm_pages(
    vma: &Arc<LockedVMA>,
    mapper: &mut crate::arch::mm::PageMapper,
    start_phys: PhysPageFrame,
    destination: VirtPageFrame,
    count: PageFrameCount,
    flags: EntryFlags<MMArch>,
) {
    let mut page_manager_guard = page_manager_lock();
    let mut phys = start_phys;
    let mut virt = destination;
    let mut flusher: PageFlushAll<MMArch> = PageFlushAll::new();

    for _ in 0..count.data() {
        let r = unsafe { mapper.map_phys(virt.virt_address(), phys.phys_address(), flags) }
            .expect("Failed to map SysV SHM page");
        flusher.consume(r);

        page_manager_guard
            .get_unwrap(&phys.phys_address())
            .write()
            .insert_vma(vma.clone());

        phys = phys.next();
        virt = virt.next();
    }
}

/// # SYS_SHMAT系统调用函数，用于连接共享内存段
///
/// ## 参数
///
/// - `id`: 共享内存id
/// - `vaddr`: 连接共享内存的进程虚拟内存区域起始地址
/// - `shmflg`: 共享内存标志
///
/// ## 返回值
///
/// 成功：映射到共享内存的虚拟内存区域起始地址
/// 失败：错误码
pub(super) fn do_kernel_shmat(
    id: ShmId,
    vaddr: VirtAddr,
    shmflg: ShmFlags,
) -> Result<usize, SystemError> {
    let ipcns = ProcessManager::current_ipcns();
    let current_address_space = AddressSpace::current()?;
    let mut address_write_guard = current_address_space.write();
    let vm_flags = VmFlags::from(shmflg);
    let page_flags: EntryFlags<MMArch> =
        EntryFlags::from_prot_flags(ProtFlags::from(vm_flags), true);

    let (size, phys) = {
        let mut shm_manager_guard = ipcns.shm.lock();
        let kernel_shm = shm_manager_guard.get_mut(&id).ok_or(SystemError::EINVAL)?;
        (
            page_align_up(kernel_shm.size()),
            PhysPageFrame::new(kernel_shm.start_paddr()),
        )
    };
    let count = PageFrameCount::from_bytes(size).unwrap();
    let r = match vaddr.data() {
        // 找到空闲区域并映射到共享内存
        0 => {
            // 找到空闲区域
            let region = address_write_guard
                .mappings
                .find_free(vaddr, size)
                .ok_or(SystemError::EINVAL)?;
            let destination = VirtPageFrame::new(region.start());
            let mut vma = VMA::new_sysv_shm(region, vm_flags, page_flags, id);
            vma.set_mapped(true);
            let vma = LockedVMA::new(vma);

            map_sysv_shm_pages(
                &vma,
                &mut address_write_guard.user_mapper.utable,
                phys,
                destination,
                count,
                page_flags,
            );

            // 将VMA加入到当前进程的VMA列表中
            vma.open()?;
            address_write_guard.mappings.insert_vma(vma);

            region.start().data()
        }
        // 指定虚拟地址
        _ => {
            // 获取对应vma
            let vma = address_write_guard
                .mappings
                .contains(vaddr)
                .ok_or(SystemError::EINVAL)?;
            let old_region = {
                let guard = vma.lock();
                if guard.region().start() != vaddr {
                    return Err(SystemError::EINVAL);
                }
                *guard.region()
            };
            let new_region = VirtRegion::new(vaddr, size);
            if address_write_guard
                .mappings
                .conflicts(new_region)
                .any(|conflict| conflict.lock().region() != &old_region)
            {
                return Err(SystemError::EINVAL);
            }

            // 验证用户虚拟内存区域是否有效
            let _ = UserBufferReader::new(vaddr.data() as *const u8, size, true)?;

            // 取消原映射
            let vma = address_write_guard
                .mappings
                .remove_vma(&old_region)
                .ok_or(SystemError::EINVAL)?;
            {
                let _pt_edit = current_address_space.page_table_edit();
                let mut tlb = MmuGather::gather(&current_address_space);
                vma.unmap(&mut address_write_guard.user_mapper.utable, &mut tlb);
                tlb.finish();
            }

            let mut new_vma = VMA::new_sysv_shm(new_region, vm_flags, page_flags, id);
            new_vma.set_mapped(true);
            let new_vma = LockedVMA::new(new_vma);
            map_sysv_shm_pages(
                &new_vma,
                &mut address_write_guard.user_mapper.utable,
                phys,
                VirtPageFrame::new(vaddr),
                count,
                page_flags,
            );
            new_vma.open()?;
            address_write_guard.mappings.insert_vma(new_vma);

            vaddr.data()
        }
    };

    // 更新最后一次连接时间
    let mut shm_manager_guard = ipcns.shm.lock();
    let kernel_shm = shm_manager_guard.get_mut(&id).ok_or(SystemError::EINVAL)?;
    kernel_shm.update_atim();

    Ok(r)
}

impl SysShmatHandle {
    #[inline(always)]
    fn id(args: &[usize]) -> ShmId {
        ShmId::new(args[0]) // 更正 ShmIT 为 ShmId
    }

    #[inline(always)]
    fn vaddr(args: &[usize]) -> VirtAddr {
        VirtAddr::new(args[1])
    }
    #[inline(always)]
    fn shmflg(args: &[usize]) -> ShmFlags {
        ShmFlags::from_bits_truncate(args[2] as u32)
    }
}

impl Syscall for SysShmatHandle {
    fn num_args(&self) -> usize {
        3
    }

    fn entry_format(&self, args: &[usize]) -> Vec<FormattedSyscallParam> {
        vec![
            FormattedSyscallParam::new("id", format!("{}", Self::id(args).data())),
            FormattedSyscallParam::new("vaddr", format!("{}", Self::vaddr(args).data())),
            FormattedSyscallParam::new("shmflg", format!("{}", Self::shmflg(args).bits())),
        ]
    }
    fn handle(&self, args: &[usize], _frame: &mut TrapFrame) -> Result<usize, SystemError> {
        let id = Self::id(args);
        let vaddr = Self::vaddr(args);
        let shmflg = Self::shmflg(args);
        do_kernel_shmat(id, vaddr, shmflg)
    }
}

declare_syscall!(SYS_SHMAT, SysShmatHandle);
