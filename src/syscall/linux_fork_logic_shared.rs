use core::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn try_clone_linux_fork_path(
    path: &str,
) -> Result<alloc::string::String, alloc::collections::TryReserveError> {
    let mut child = alloc::string::String::new();
    child.try_reserve_exact(path.len())?;
    child.push_str(path);
    Ok(child)
}

static LINUX_FORK_FAILURE_POINT: AtomicUsize = AtomicUsize::new(LinuxForkFailurePoint::COUNT);
static LINUX_FORK_FAILURE_OCCURRENCE: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn configure_fork_failure(point: LinuxForkFailurePoint, occurrence: usize) {
    LINUX_FORK_FAILURE_OCCURRENCE.store(occurrence, Ordering::SeqCst);
    LINUX_FORK_FAILURE_POINT.store(point as usize, Ordering::SeqCst);
}

pub(crate) fn clear_fork_failure() {
    LINUX_FORK_FAILURE_POINT.store(LinuxForkFailurePoint::COUNT, Ordering::SeqCst);
    LINUX_FORK_FAILURE_OCCURRENCE.store(0, Ordering::SeqCst);
}

pub(crate) fn fork_failpoint(point: LinuxForkFailurePoint) -> bool {
    if LINUX_FORK_FAILURE_POINT.load(Ordering::SeqCst) != point as usize {
        return false;
    }
    let remaining = LINUX_FORK_FAILURE_OCCURRENCE.load(Ordering::SeqCst);
    if remaining != 0 {
        LINUX_FORK_FAILURE_OCCURRENCE.store(remaining - 1, Ordering::SeqCst);
        return false;
    }
    clear_fork_failure();
    true
}

pub(crate) trait LinuxForkTransactionBackend {
    type Error;
    type Output;

    fn injected_failure(&self) -> Self::Error;
    fn acquire_scheduler_thread(&mut self) -> Result<(), Self::Error>;
    fn acquire_task(&mut self) -> Result<(), Self::Error>;
    fn acquire_process(&mut self) -> Result<(), Self::Error>;
    fn acquire_resources(&mut self) -> Result<(), Self::Error>;
    fn acquire_memory(&mut self) -> Result<(), Self::Error>;
    fn configure_child(&mut self) -> Result<(), Self::Error>;
    fn install_resources(&mut self) -> Result<(), Self::Error>;
    fn begin_publication(&mut self) -> Result<(), Self::Error>;
    fn publish_process(&mut self) -> Result<(), Self::Error>;
    fn publish_task(&mut self) -> Result<(), Self::Error>;
    fn publish_scheduler_thread(&mut self) -> Result<(), Self::Error>;
    fn complete_publication(&mut self) -> Result<(), Self::Error>;
    fn finish(&mut self) -> Result<Self::Output, Self::Error>;
    fn rollback(&mut self, acquisition: LinuxForkAcquisition);
}

struct LinuxForkTransaction<B: LinuxForkTransactionBackend> {
    backend: B,
    ledger: LinuxForkAcquisitionLedger,
    committed: bool,
}

impl<B: LinuxForkTransactionBackend> LinuxForkTransaction<B> {
    fn new(backend: B) -> Self {
        Self {
            backend,
            ledger: LinuxForkAcquisitionLedger::new(),
            committed: false,
        }
    }

    fn acquire(
        &mut self,
        acquisition: LinuxForkAcquisition,
        operation: impl FnOnce(&mut B) -> Result<(), B::Error>,
    ) -> Result<(), B::Error> {
        if !self.ledger.acquire(acquisition) {
            return Err(self.backend.injected_failure());
        }
        operation(&mut self.backend)
    }

    fn fail_if(
        &self,
        point: LinuxForkFailurePoint,
        should_fail: &mut impl FnMut(LinuxForkFailurePoint) -> bool,
    ) -> Result<(), B::Error> {
        if should_fail(point) {
            Err(self.backend.injected_failure())
        } else {
            Ok(())
        }
    }
}

impl<B: LinuxForkTransactionBackend> Drop for LinuxForkTransaction<B> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut rollback = [None; 6];
        let rollback_len = self.ledger.rollback_into(&mut rollback);
        for acquisition in rollback[..rollback_len].iter().flatten().copied() {
            self.backend.rollback(acquisition);
        }
        debug_assert!(self.ledger.is_empty());
    }
}

pub(crate) fn run_linux_fork_transaction<B: LinuxForkTransactionBackend>(
    backend: B,
    mut should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<B::Output, B::Error> {
    let mut transaction = LinuxForkTransaction::new(backend);

    transaction.acquire(LinuxForkAcquisition::SchedulerThread, |backend| {
        backend.acquire_scheduler_thread()
    })?;
    transaction.fail_if(LinuxForkFailurePoint::SchedulerThread, &mut should_fail)?;
    transaction.acquire(LinuxForkAcquisition::Task, |backend| backend.acquire_task())?;
    transaction.fail_if(LinuxForkFailurePoint::Task, &mut should_fail)?;
    transaction.acquire(LinuxForkAcquisition::Process, |backend| {
        backend.acquire_process()
    })?;
    transaction.fail_if(LinuxForkFailurePoint::Process, &mut should_fail)?;
    transaction.acquire(LinuxForkAcquisition::Resources, |backend| {
        backend.acquire_resources()
    })?;
    transaction.acquire(LinuxForkAcquisition::Memory, |backend| backend.acquire_memory())?;
    transaction.fail_if(LinuxForkFailurePoint::Memory, &mut should_fail)?;
    transaction.acquire(LinuxForkAcquisition::Configured, |backend| {
        backend.configure_child()
    })?;
    transaction.fail_if(LinuxForkFailurePoint::Configured, &mut should_fail)?;

    transaction.backend.install_resources()?;
    transaction.backend.begin_publication()?;
    transaction.backend.publish_process()?;
    transaction.fail_if(
        LinuxForkFailurePoint::ProcessPublication,
        &mut should_fail,
    )?;
    transaction.backend.publish_task()?;
    transaction.fail_if(LinuxForkFailurePoint::TaskPublication, &mut should_fail)?;
    transaction.backend.publish_scheduler_thread()?;
    transaction.fail_if(
        LinuxForkFailurePoint::SchedulerPublication,
        &mut should_fail,
    )?;
    transaction.backend.complete_publication()?;

    let output = transaction.backend.finish()?;
    transaction.committed = true;
    Ok(output)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxForkPreparedContext<F> {
    pub frame: F,
    pub return_pc: u64,
    pub pstate: u64,
    pub user_sp: u64,
    pub tls: u64,
    pub root_paddr: u64,
}

pub(crate) fn prepare_linux_fork_context<F: Copy>(
    mut frame: F,
    return_pc: u64,
    pstate: u64,
    user_sp: u64,
    tls: u64,
    root_paddr: u64,
    set_child_result: impl FnOnce(&mut F),
) -> LinuxForkPreparedContext<F> {
    set_child_result(&mut frame);
    LinuxForkPreparedContext {
        frame,
        return_pc,
        pstate,
        user_sp,
        tls,
        root_paddr,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinuxCredentialsCore {
    pub real_uid: usize,
    pub effective_uid: usize,
    pub saved_uid: usize,
    pub filesystem_uid: usize,
    pub real_gid: usize,
    pub effective_gid: usize,
    pub saved_gid: usize,
    pub filesystem_gid: usize,
}

impl LinuxCredentialsCore {
    pub(crate) const fn fork_child(self) -> Self {
        self
    }

    pub(crate) fn set_resuid(&mut self, real: usize, effective: usize, saved: usize) {
        self.real_uid = real;
        self.effective_uid = effective;
        self.saved_uid = saved;
        self.filesystem_uid = effective;
    }

    pub(crate) fn set_resgid(&mut self, real: usize, effective: usize, saved: usize) {
        self.real_gid = real;
        self.effective_gid = effective;
        self.saved_gid = saved;
        self.filesystem_gid = effective;
    }

    pub(crate) fn set_filesystem_uid(&mut self, uid: usize) -> usize {
        let previous = self.filesystem_uid;
        self.filesystem_uid = uid;
        previous
    }

    pub(crate) fn set_filesystem_gid(&mut self, gid: usize) -> usize {
        let previous = self.filesystem_gid;
        self.filesystem_gid = gid;
        previous
    }
}
