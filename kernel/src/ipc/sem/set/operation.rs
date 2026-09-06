//! Allocation-free atomic attempt: simulation and commit cannot be separated by callers.
use super::*;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ipc::sem) enum SemWaitType {
    /// `sem_op < 0`: wait for semval to increase (GETNCNT)
    Increase,
    /// `sem_op == 0`: wait for semval to reach zero (GETZCNT)
    Zero,
}

/// The precise operation currently blocking an operation group.
#[derive(Debug, Clone, Copy)]
pub(in crate::ipc::sem) struct SemBlockedOp {
    pub(in crate::ipc::sem) semnum: usize,
    pub(in crate::ipc::sem) wait_type: SemWaitType,
    pub(in crate::ipc::sem) nowait: bool,
}
#[derive(Debug)]
struct SemopScratchEntry {
    semnum: usize,
    initial_val: i32,
    virtual_val: i32,
    initial_adj: i16,
    virtual_adj: i16,
}

#[derive(Debug)]
pub(in crate::ipc::sem) struct SemopScratch {
    entries: Vec<SemopScratchEntry>,
}

impl SemopScratch {
    pub(in crate::ipc::sem) fn try_new(capacity: usize) -> Result<Self, SystemError> {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|_| SystemError::ENOMEM)?;
        Ok(Self { entries })
    }

    fn clear(&mut self) {
        self.entries.clear();
    }

    fn entry_for(
        &mut self,
        set: &KernelSemSet,
        semnum: usize,
        undo: Option<&SemUndoRecord>,
    ) -> Result<&mut SemopScratchEntry, SystemError> {
        if let Some(index) = self.entries.iter().position(|entry| entry.semnum == semnum) {
            return Ok(&mut self.entries[index]);
        }

        if self.entries.len() == self.entries.capacity() {
            return Err(SystemError::ENOMEM);
        }

        let initial_val = set.sems[semnum].val;
        let initial_adj = undo
            .map(|record| record.adjustment(semnum))
            .unwrap_or_default();
        self.entries.push(SemopScratchEntry {
            semnum,
            initial_val,
            virtual_val: initial_val,
            initial_adj,
            virtual_adj: initial_adj,
        });
        Ok(self
            .entries
            .last_mut()
            .expect("SEM_UNDO scratch entry was just inserted"))
    }
}

/// Fixed-capacity virtual semaphore state produced by `SemopScratch`.
#[derive(Debug)]
struct SemopSimulation {
    entry_count: usize,
}

impl SemopSimulation {
    #[cfg(test)]
    fn empty_for_test() -> Self {
        Self { entry_count: 0 }
    }
}

/// Result of an attempted `semop` execution
#[derive(Debug)]
enum SemopOutcome {
    Ready(SemopSimulation),
    Blocked(SemBlockedOp),
}

impl SemopOutcome {
    #[cfg(test)]
    fn ready_for_test(self) -> SemopSimulation {
        match self {
            Self::Ready(simulation) => simulation,
            Self::Blocked(_) => panic!("expected ready semop outcome"),
        }
    }
}

impl KernelSemSet {
    /// Simulate sops in order without changing shared semaphore values or undo records.
    fn simulate_semop(
        set: &KernelSemSet,
        sops: &[PosixSemBuf],
        undo: Option<&mut SemUndoRecord>,
        scratch: &mut SemopScratch,
    ) -> Result<SemopOutcome, SystemError> {
        scratch.clear();

        for op in sops {
            let idx = op.sem_num as usize;
            if idx >= set.sems.len() {
                return Err(SystemError::EFBIG);
            }

            let has_undo = (op.sem_flg as u32) & SemFlags::SEM_UNDO.bits() != 0;
            let entry = scratch.entry_for(set, idx, undo.as_deref())?;
            let current = entry.virtual_val;
            if op.sem_op == 0 {
                if current != 0 {
                    return Ok(SemopOutcome::Blocked(SemBlockedOp {
                        semnum: idx,
                        wait_type: SemWaitType::Zero,
                        nowait: (op.sem_flg as u32) & SemFlags::IPC_NOWAIT.bits() != 0,
                    }));
                }
                continue;
            }

            let result = current as i64 + op.sem_op as i64;
            if result > SEMVMX as i64 {
                return Err(SystemError::ERANGE);
            }
            if result < 0 {
                return Ok(SemopOutcome::Blocked(SemBlockedOp {
                    semnum: idx,
                    wait_type: SemWaitType::Increase,
                    nowait: (op.sem_flg as u32) & SemFlags::IPC_NOWAIT.bits() != 0,
                }));
            }

            if has_undo {
                let next_adj = entry.virtual_adj as i32 - op.sem_op as i32;
                if !(i16::MIN as i32..=i16::MAX as i32).contains(&next_adj) {
                    return Err(SystemError::ERANGE);
                }
                entry.virtual_adj = next_adj as i16;
            }
            entry.virtual_val = result as i32;
        }

        Ok(SemopOutcome::Ready(SemopSimulation {
            entry_count: scratch.entries.len(),
        }))
    }

    /// Commit a successful simulation while the manager lock is held.
    fn commit_semop(
        set: &mut KernelSemSet,
        simulation: SemopSimulation,
        scratch: &SemopScratch,
        pid: Option<Arc<Pid>>,
        mut undo: Option<&mut SemUndoRecord>,
    ) -> bool {
        let mut values_changed = false;
        for entry in scratch.entries.iter().take(simulation.entry_count) {
            let sem = &mut set.sems[entry.semnum];
            values_changed |= entry.initial_val != entry.virtual_val;
            sem.val = entry.virtual_val;
            sem.pid = pid.clone();
            if entry.virtual_adj != entry.initial_adj {
                if let Some(record) = undo.as_deref_mut() {
                    record.set_adjustment(entry.semnum, entry.virtual_adj);
                }
            }
        }
        set.sem_otime = PosixTimeSpec::now().tv_sec;
        values_changed
    }
}

#[derive(Debug)]
pub(in crate::ipc::sem) enum SemAttempt {
    Completed { values_changed: bool },
    Blocked(SemBlockedOp),
}
impl KernelSemSet {
    /// The caller holds the manager lock and (for SEM_UNDO) the current group record.
    /// Does not allocate, queue, wake or authorize. Failed/blocked attempts never commit.
    pub(in crate::ipc::sem) fn try_apply(
        &mut self,
        sops: &[PosixSemBuf],
        pid: Option<Arc<Pid>>,
        mut undo: Option<&mut SemUndoRecord>,
        scratch: &mut SemopScratch,
    ) -> Result<SemAttempt, SystemError> {
        match Self::simulate_semop(self, sops, undo.as_deref_mut(), scratch)? {
            SemopOutcome::Ready(simulation) => Ok(SemAttempt::Completed {
                values_changed: Self::commit_semop(self, simulation, scratch, pid, undo),
            }),
            SemopOutcome::Blocked(blocker) => Ok(SemAttempt::Blocked(blocker)),
        }
    }
}

#[cfg(test)]
mod tests;
