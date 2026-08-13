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

#[cfg(not(target_os = "none"))]
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

pub(crate) trait LinuxForkOwnershipOps {
    type Error;
    type Output;
    type SchedulerThread;
    type Parent;
    type Task;
    type Process;
    type Resources;
    type Memory;
    type Configured;
    type Publication;

    fn injected_failure(&self) -> Self::Error;
    fn acquire_scheduler_thread(&mut self) -> Result<Self::SchedulerThread, Self::Error>;
    fn acquire_task(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(Self::Parent, Self::Task), Self::Error>;
    fn acquire_process(
        &mut self,
        parent: &Self::Parent,
        scheduler_thread: &Self::SchedulerThread,
        task: &Self::Task,
    ) -> Result<Self::Process, Self::Error>;
    fn acquire_resources(
        &mut self,
        parent: &Self::Parent,
    ) -> Result<Self::Resources, Self::Error>;
    fn acquire_memory(
        &mut self,
        parent: &Self::Parent,
        process: &Self::Process,
        resources: &mut Self::Resources,
    ) -> Result<Self::Memory, Self::Error>;
    fn configure_child(
        &mut self,
        process: &Self::Process,
        scheduler_thread: &Self::SchedulerThread,
        memory: &Self::Memory,
    ) -> Result<Self::Configured, Self::Error>;
    fn install_resources(
        &mut self,
        process: &Self::Process,
        resources: &mut Option<Self::Resources>,
    ) -> Result<(), Self::Error>;
    fn begin_publication(&mut self) -> Result<Self::Publication, Self::Error>;
    fn publish_process(
        &mut self,
        process: &Self::Process,
        configured: &Self::Configured,
    ) -> Result<(), Self::Error>;
    fn publish_task(&mut self, task: &Self::Task) -> Result<(), Self::Error>;
    fn publish_scheduler_thread(
        &mut self,
        scheduler_thread: &Self::SchedulerThread,
    ) -> Result<(), Self::Error>;
    fn complete_publication(&mut self, process: &Self::Process) -> Result<(), Self::Error>;
    fn finish(
        &mut self,
        process: &Self::Process,
        configured: &Self::Configured,
    ) -> Result<Self::Output, Self::Error>;
    fn restore_publication(&mut self, publication: Self::Publication);
    fn rollback_configured(&mut self, configured: Self::Configured);
    fn rollback_memory(&mut self, memory: Self::Memory);
    fn rollback_reserved_resources(&mut self, resources: Self::Resources);
    fn rollback_installed_resources(&mut self, process: &Self::Process);
    fn rollback_process(&mut self, process: Self::Process);
    fn rollback_task(&mut self, task: Self::Task);
    fn rollback_scheduler_thread(&mut self, scheduler_thread: Self::SchedulerThread);
}

pub(crate) struct LinuxForkOwnershipCore<O: LinuxForkOwnershipOps> {
    ops: O,
    scheduler_thread: Option<O::SchedulerThread>,
    parent: Option<O::Parent>,
    task: Option<O::Task>,
    process: Option<O::Process>,
    resources: Option<O::Resources>,
    resources_installed: bool,
    memory: Option<O::Memory>,
    configured: Option<O::Configured>,
    publication: Option<O::Publication>,
}

impl<O: LinuxForkOwnershipOps> LinuxForkOwnershipCore<O> {
    pub(crate) fn new(ops: O) -> Self {
        Self {
            ops,
            scheduler_thread: None,
            parent: None,
            task: None,
            process: None,
            resources: None,
            resources_installed: false,
            memory: None,
            configured: None,
            publication: None,
        }
    }
}

impl<O: LinuxForkOwnershipOps> LinuxForkTransactionBackend for LinuxForkOwnershipCore<O> {
    type Error = O::Error;
    type Output = O::Output;

    fn injected_failure(&self) -> Self::Error {
        self.ops.injected_failure()
    }

    fn acquire_scheduler_thread(&mut self) -> Result<(), Self::Error> {
        self.scheduler_thread = Some(self.ops.acquire_scheduler_thread()?);
        Ok(())
    }

    fn acquire_task(&mut self) -> Result<(), Self::Error> {
        let scheduler_thread = self
            .scheduler_thread
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let (parent, task) = self.ops.acquire_task(scheduler_thread)?;
        self.parent = Some(parent);
        self.task = Some(task);
        Ok(())
    }

    fn acquire_process(&mut self) -> Result<(), Self::Error> {
        let parent = self
            .parent
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let scheduler_thread = self
            .scheduler_thread
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let task = self
            .task
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.process = Some(self.ops.acquire_process(parent, scheduler_thread, task)?);
        Ok(())
    }

    fn acquire_resources(&mut self) -> Result<(), Self::Error> {
        let parent = self
            .parent
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.resources = Some(self.ops.acquire_resources(parent)?);
        Ok(())
    }

    fn acquire_memory(&mut self) -> Result<(), Self::Error> {
        let parent = self
            .parent
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let resources = self
            .resources
            .as_mut()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.memory = Some(self.ops.acquire_memory(parent, process, resources)?);
        Ok(())
    }

    fn configure_child(&mut self) -> Result<(), Self::Error> {
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let scheduler_thread = self
            .scheduler_thread
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.configured = Some(
            self.ops
                .configure_child(process, scheduler_thread, memory)?,
        );
        Ok(())
    }

    fn install_resources(&mut self) -> Result<(), Self::Error> {
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.ops.install_resources(process, &mut self.resources)?;
        self.resources_installed = true;
        Ok(())
    }

    fn begin_publication(&mut self) -> Result<(), Self::Error> {
        if self.publication.is_some() {
            return Err(self.ops.injected_failure());
        }
        self.publication = Some(self.ops.begin_publication()?);
        Ok(())
    }

    fn publish_process(&mut self) -> Result<(), Self::Error> {
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let configured = self
            .configured
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.ops.publish_process(process, configured)
    }

    fn publish_task(&mut self) -> Result<(), Self::Error> {
        let task = self
            .task
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.ops.publish_task(task)
    }

    fn publish_scheduler_thread(&mut self) -> Result<(), Self::Error> {
        let scheduler_thread = self
            .scheduler_thread
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.ops.publish_scheduler_thread(scheduler_thread)
    }

    fn complete_publication(&mut self) -> Result<(), Self::Error> {
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        self.ops.complete_publication(process)
    }

    fn finish(&mut self) -> Result<Self::Output, Self::Error> {
        let process = self
            .process
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let configured = self
            .configured
            .as_ref()
            .ok_or_else(|| self.ops.injected_failure())?;
        let output = self.ops.finish(process, configured)?;
        if let Some(publication) = self.publication.take() {
            self.ops.restore_publication(publication);
        }
        Ok(output)
    }

    fn rollback(&mut self, acquisition: LinuxForkAcquisition) {
        match acquisition {
            LinuxForkAcquisition::Configured => {
                if let Some(configured) = self.configured.take() {
                    self.ops.rollback_configured(configured);
                }
            }
            LinuxForkAcquisition::Memory => {
                if let Some(memory) = self.memory.take() {
                    self.ops.rollback_memory(memory);
                }
            }
            LinuxForkAcquisition::Resources => {
                if self.resources_installed {
                    if let Some(process) = self.process.as_ref() {
                        self.ops.rollback_installed_resources(process);
                    }
                    self.resources_installed = false;
                }
                if let Some(resources) = self.resources.take() {
                    self.ops.rollback_reserved_resources(resources);
                }
            }
            LinuxForkAcquisition::Process => {
                if let Some(process) = self.process.take() {
                    self.ops.rollback_process(process);
                }
            }
            LinuxForkAcquisition::Task => {
                if let Some(task) = self.task.take() {
                    self.ops.rollback_task(task);
                }
            }
            LinuxForkAcquisition::SchedulerThread => {
                if let Some(scheduler_thread) = self.scheduler_thread.take() {
                    self.ops.rollback_scheduler_thread(scheduler_thread);
                }
                if let Some(publication) = self.publication.take() {
                    self.ops.restore_publication(publication);
                }
            }
        }
    }
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

pub(crate) trait LinuxForkPageOps {
    type Page: Copy;
    type Error;

    fn failure_error(&self) -> Self::Error;
    fn is_private(&self, page: Self::Page) -> bool;
    fn allocate_private(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error>;
    fn copy_private(
        &mut self,
        parent: Self::Page,
        child: Self::Page,
    ) -> Result<(), Self::Error>;
    fn acquire_shared(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error>;
    fn release_page(&mut self, page: Self::Page);
    fn map_page(
        &mut self,
        address: usize,
        page: Self::Page,
        prot: usize,
    ) -> Result<(), Self::Error>;
    fn unmap_page(&mut self, address: usize);
}

fn linux_fork_page_failure(
    specific: LinuxForkFailurePoint,
    aggregate: LinuxForkFailurePoint,
    should_fail: &mut impl FnMut(LinuxForkFailurePoint) -> bool,
) -> bool {
    should_fail(specific) || should_fail(aggregate)
}

fn release_linux_fork_pages<O: LinuxForkPageOps>(ops: &mut O, pages: &[O::Page]) {
    for page in pages.iter().copied().rev() {
        ops.release_page(page);
    }
}

pub(crate) fn clone_linux_fork_pages<O: LinuxForkPageOps>(
    ops: &mut O,
    parent_pages: &[O::Page],
    mut should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<alloc::vec::Vec<O::Page>, O::Error> {
    let mut child_pages = alloc::vec::Vec::new();
    child_pages
        .try_reserve_exact(parent_pages.len())
        .map_err(|_| ops.failure_error())?;

    for parent in parent_pages.iter().copied() {
        let child = if ops.is_private(parent) {
            let child = match ops.allocate_private(parent) {
                Ok(child) => child,
                Err(error) => {
                    release_linux_fork_pages(ops, &child_pages);
                    return Err(error);
                }
            };
            if linux_fork_page_failure(
                LinuxForkFailurePoint::PrivatePageAllocation,
                LinuxForkFailurePoint::PrivatePage,
                &mut should_fail,
            ) {
                ops.release_page(child);
                release_linux_fork_pages(ops, &child_pages);
                return Err(ops.failure_error());
            }
            if let Err(error) = ops.copy_private(parent, child) {
                ops.release_page(child);
                release_linux_fork_pages(ops, &child_pages);
                return Err(error);
            }
            if linux_fork_page_failure(
                LinuxForkFailurePoint::PrivatePageCopy,
                LinuxForkFailurePoint::PrivatePage,
                &mut should_fail,
            ) {
                ops.release_page(child);
                release_linux_fork_pages(ops, &child_pages);
                return Err(ops.failure_error());
            }
            child
        } else {
            let child = match ops.acquire_shared(parent) {
                Ok(child) => child,
                Err(error) => {
                    release_linux_fork_pages(ops, &child_pages);
                    return Err(error);
                }
            };
            if should_fail(LinuxForkFailurePoint::SharedReference) {
                ops.release_page(child);
                release_linux_fork_pages(ops, &child_pages);
                return Err(ops.failure_error());
            }
            child
        };
        child_pages.push(child);
    }
    Ok(child_pages)
}

pub(crate) fn map_linux_fork_pages<O: LinuxForkPageOps>(
    ops: &mut O,
    address: usize,
    page_size: usize,
    pages: &[O::Page],
    prot: usize,
    should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<(), O::Error> {
    map_linux_fork_pages_with_protection(ops, address, page_size, pages, |_| prot, should_fail)
}

pub(crate) fn map_linux_fork_pages_with_protection<O: LinuxForkPageOps>(
    ops: &mut O,
    address: usize,
    page_size: usize,
    pages: &[O::Page],
    mut protection: impl FnMut(usize) -> usize,
    mut should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<(), O::Error> {
    let mut mapped = 0usize;
    for page in pages.iter().copied() {
        let Some(page_address) = mapped
            .checked_mul(page_size)
            .and_then(|offset| address.checked_add(offset))
        else {
            for rollback in (0..mapped).rev() {
                ops.unmap_page(address + rollback * page_size);
            }
            return Err(ops.failure_error());
        };
        if let Err(error) = ops.map_page(page_address, page, protection(mapped)) {
            for rollback in (0..mapped).rev() {
                ops.unmap_page(address + rollback * page_size);
            }
            return Err(error);
        }
        mapped += 1;
        let failed = if ops.is_private(page) {
            linux_fork_page_failure(
                LinuxForkFailurePoint::PrivatePageMap,
                LinuxForkFailurePoint::PrivatePage,
                &mut should_fail,
            )
        } else {
            linux_fork_page_failure(
                LinuxForkFailurePoint::SharedPageMap,
                LinuxForkFailurePoint::SharedReference,
                &mut should_fail,
            )
        };
        if failed {
            for rollback in (0..mapped).rev() {
                ops.unmap_page(address + rollback * page_size);
            }
            return Err(ops.failure_error());
        }
    }
    Ok(())
}

#[cfg(not(target_os = "none"))]
pub(crate) fn clone_and_map_linux_fork_pages<O: LinuxForkPageOps>(
    ops: &mut O,
    address: usize,
    page_size: usize,
    parent_pages: &[O::Page],
    prot: usize,
    mut should_fail: impl FnMut(LinuxForkFailurePoint) -> bool,
) -> Result<alloc::vec::Vec<O::Page>, O::Error> {
    let pages = clone_linux_fork_pages(ops, parent_pages, &mut should_fail)?;
    if let Err(error) = map_linux_fork_pages(
        ops,
        address,
        page_size,
        &pages,
        prot,
        &mut should_fail,
    ) {
        release_linux_fork_pages(ops, &pages);
        return Err(error);
    }
    Ok(pages)
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
