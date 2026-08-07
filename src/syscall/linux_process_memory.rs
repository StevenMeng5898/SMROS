use alloc::string::String;
use alloc::vec::Vec;

use crate::kernel_lowlevel::memory::{PageFrameAllocator, PAGE_SIZE};
#[cfg(target_arch = "aarch64")]
use crate::kernel_lowlevel::Aarch64AddressSpace;
use crate::kernel_objects::ObjectType;

use super::{linux_process, SysError};

include!("linux_process_memory_logic_shared.rs");
include!("linux_runtime_lock_shared.rs");

#[derive(Clone)]
pub(crate) enum LinuxMappingSource {
    Anonymous,
    File {
        fd: usize,
        offset: u64,
        path: String,
    },
    SharedMemory {
        id: u32,
    },
}

impl LinuxMappingSource {
    pub(crate) fn slice(&self, delta: usize) -> Self {
        match self {
            Self::Anonymous => Self::Anonymous,
            Self::File { fd, offset, path } => Self::File {
                fd: *fd,
                offset: offset.saturating_add(delta as u64),
                path: path.clone(),
            },
            Self::SharedMemory { id } => Self::SharedMemory { id: *id },
        }
    }
}

pub(crate) struct LinuxProcessMapping {
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub flags: usize,
    pub pages: Vec<LinuxPageBacking>,
    pub source: LinuxMappingSource,
}

#[derive(Clone, Copy)]
struct LinuxMappedPage {
    address: usize,
    backing: LinuxPageBacking,
    prot: usize,
}

pub(crate) struct BrkState {
    pub start: usize,
    pub current: usize,
    pub limit: usize,
    pub pages: Vec<LinuxPageBacking>,
}

impl BrkState {
    fn new() -> Self {
        Self {
            start: LINUX_BRK_BASE,
            current: LINUX_BRK_BASE,
            limit: LINUX_BRK_LIMIT,
            pages: Vec::new(),
        }
    }
}

pub(crate) struct LinuxProcessMemory {
    pub pid: usize,
    #[cfg(target_arch = "aarch64")]
    pub address_space: Aarch64AddressSpace,
    #[cfg(not(target_arch = "aarch64"))]
    address_space: FallbackAddressSpace,
    pub mappings: Vec<LinuxProcessMapping>,
    shared_attachments: Vec<LinuxSharedAttachmentRecord>,
    pub initial_stack: Option<(usize, usize)>,
    pub next_addr: usize,
    pub brk: BrkState,
}

#[cfg(not(target_arch = "aarch64"))]
struct FallbackAddressSpace {
    root_paddr: u64,
}

#[cfg(not(target_arch = "aarch64"))]
impl FallbackAddressSpace {
    fn new(pid: usize) -> Result<Self, SysError> {
        let root_paddr = pid.checked_mul(PAGE_SIZE).ok_or(SysError::ENOMEM)? as u64;
        Ok(Self { root_paddr })
    }

    fn root_paddr(&self) -> u64 {
        self.root_paddr
    }

    fn map_user_page(
        &mut self,
        _vaddr: usize,
        _pfn: u64,
        _readable: bool,
        _writable: bool,
        _executable: bool,
    ) -> Result<(), SysError> {
        Ok(())
    }

    fn protect_user_page(
        &mut self,
        _vaddr: usize,
        _readable: bool,
        _writable: bool,
        _executable: bool,
    ) -> Result<(), SysError> {
        Ok(())
    }

    fn unmap_user_page(&mut self, _vaddr: usize) -> Result<u64, SysError> {
        Ok(0)
    }

    fn copy_to_user(&self, vaddr: usize, bytes: &[u8]) -> Result<(), SysError> {
        if !super::syscall_logic::user_buffer_valid(vaddr, bytes.len()) {
            return Err(SysError::EFAULT);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), vaddr as *mut u8, bytes.len());
        }
        Ok(())
    }

    fn copy_from_user(&self, vaddr: usize, out: &mut [u8]) -> Result<(), SysError> {
        if !super::syscall_logic::user_buffer_valid(vaddr, out.len()) {
            return Err(SysError::EFAULT);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(vaddr as *const u8, out.as_mut_ptr(), out.len());
        }
        Ok(())
    }
}

pub(crate) struct LinuxMemoryStats {
    pub mapping_count: usize,
    pub mapped_bytes: usize,
    pub committed_pages: usize,
    pub brk_start: usize,
    pub brk_current: usize,
    pub brk_limit: usize,
    pub brk_committed_pages: usize,
}

#[derive(Clone)]
pub(crate) struct LinuxMemoryMappingSnapshot {
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub flags: usize,
    pub pfns: Vec<u64>,
    pub source: LinuxMappingSource,
}

struct LinuxProcessMemoryRuntime {
    memories: Vec<LinuxProcessMemory>,
}

struct LinuxSharedPageRuntime {
    pages: Vec<LinuxSharedPageRecord>,
    mmap_objects: Vec<LinuxSharedMmapObject>,
    next_mmap_object_id: u32,
}

struct LinuxSharedMmapObject {
    object_id: u32,
    file_path: Option<String>,
}

impl LinuxSharedPageRuntime {
    const fn new() -> Self {
        Self {
            pages: Vec::new(),
            mmap_objects: Vec::new(),
            next_mmap_object_id: u32::MAX,
        }
    }
}

#[derive(Clone)]
pub(crate) struct LinuxSharedAttachmentClone {
    pub object_id: u32,
    pub attachment_addr: usize,
    pub attachment_len: usize,
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub flags: usize,
    pub pages: Vec<LinuxPageBacking>,
    owns_attachment_reference: bool,
}

impl LinuxProcessMemoryRuntime {
    const fn new() -> Self {
        Self {
            memories: Vec::new(),
        }
    }
}

static LINUX_PROCESS_MEMORY_RUNTIME: LinuxRuntimeLock<LinuxProcessMemoryRuntime> =
    LinuxRuntimeLock::new(LinuxProcessMemoryRuntime::new());
static LINUX_SHARED_PAGE_RUNTIME: LinuxRuntimeLock<LinuxSharedPageRuntime> =
    LinuxRuntimeLock::new(LinuxSharedPageRuntime::new());

fn with_runtime<R>(operation: impl FnOnce(&mut LinuxProcessMemoryRuntime) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_PROCESS_MEMORY_RUNTIME.lock();
    let result = operation(&mut runtime);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

fn with_shared_pages<R>(operation: impl FnOnce(&mut LinuxSharedPageRuntime) -> R) -> R {
    let interrupt_state = crate::kernel_lowlevel::cpu::mask_interrupts();
    let mut runtime = LINUX_SHARED_PAGE_RUNTIME.lock();
    let result = operation(&mut runtime);
    drop(runtime);
    crate::kernel_lowlevel::cpu::restore_interrupts(interrupt_state);
    result
}

fn shared_page(object_id: u32, page_index: usize) -> Option<LinuxSharedPageRecord> {
    with_shared_pages(|runtime| {
        runtime
            .pages
            .iter()
            .copied()
            .find(|page| page.object_id == object_id && page.page_index == page_index)
    })
}

fn acquire_or_register_shared_page(
    object_id: u32,
    page_index: usize,
    candidate_pfn: u64,
) -> Option<u64> {
    with_shared_pages(|runtime| {
        if let Some(page) = runtime
            .pages
            .iter_mut()
            .find(|page| page.object_id == object_id && page.page_index == page_index)
        {
            page.references = linux_shared_reference_acquire(page.references)?;
            return Some(page.pfn);
        }
        if runtime.pages.try_reserve(1).is_err() {
            return None;
        }
        runtime.pages.push(LinuxSharedPageRecord {
            object_id,
            page_index,
            pfn: candidate_pfn,
            references: 1,
            named: true,
        });
        Some(candidate_pfn)
    })
}

fn acquire_shared_page(object_id: u32, page_index: usize) -> bool {
    with_shared_pages(|runtime| {
        let Some(page) = runtime
            .pages
            .iter_mut()
            .find(|page| page.object_id == object_id && page.page_index == page_index)
        else {
            return false;
        };
        let Some(references) = linux_shared_reference_acquire(page.references) else {
            return false;
        };
        page.references = references;
        true
    })
}

fn release_shared_page(object_id: u32, page_index: usize) -> Option<u64> {
    with_shared_pages(|runtime| {
        let index = runtime
            .pages
            .iter()
            .position(|page| page.object_id == object_id && page.page_index == page_index)?;
        let references = linux_shared_reference_release(runtime.pages[index].references)?;
        runtime.pages[index].references = references;
        if references != 0 {
            return None;
        }
        let pfn = runtime.pages.swap_remove(index).pfn;
        if !runtime.pages.iter().any(|page| page.object_id == object_id) {
            runtime
                .mmap_objects
                .retain(|object| object.object_id != object_id);
        }
        Some(pfn)
    })
}

fn shared_mmap_object(source: &LinuxMappingSource) -> Result<(u32, usize), SysError> {
    with_shared_pages(|runtime| {
        let (file_path, first_page) = match source {
            LinuxMappingSource::Anonymous => (None, 0),
            LinuxMappingSource::File { offset, path, .. } => {
                let offset = usize::try_from(*offset).map_err(|_| SysError::EINVAL)?;
                (Some(path.as_str()), offset / PAGE_SIZE)
            }
            LinuxMappingSource::SharedMemory { .. } => return Err(SysError::EINVAL),
        };
        if let Some(file_path) = file_path {
            if let Some(object) = runtime
                .mmap_objects
                .iter()
                .find(|object| object.file_path.as_deref() == Some(file_path))
            {
                return Ok((object.object_id, first_page));
            }
        }

        let object_id = runtime.next_mmap_object_id;
        if object_id < 0x8000_0000 {
            return Err(SysError::ENOMEM);
        }
        runtime.next_mmap_object_id = object_id.checked_sub(1).ok_or(SysError::ENOMEM)?;
        runtime
            .mmap_objects
            .try_reserve(1)
            .map_err(|_| SysError::ENOMEM)?;
        let file_path = if let Some(path) = file_path {
            let mut owned = String::new();
            owned
                .try_reserve_exact(path.len())
                .map_err(|_| SysError::ENOMEM)?;
            owned.push_str(path);
            Some(owned)
        } else {
            None
        };
        runtime.mmap_objects.push(LinuxSharedMmapObject {
            object_id,
            file_path,
        });
        Ok((object_id, first_page))
    })
}

fn remove_empty_shared_mmap_object(object_id: u32) {
    with_shared_pages(|runtime| {
        if !runtime.pages.iter().any(|page| page.object_id == object_id) {
            runtime
                .mmap_objects
                .retain(|object| object.object_id != object_id);
        }
    });
}

pub(crate) fn remove_shared_page_name(object_id: u32) {
    with_shared_pages(|runtime| {
        for page in &mut runtime.pages {
            if page.object_id == object_id {
                page.named = false;
            }
        }
    });
}

pub(crate) fn register_root(pid: usize) -> Result<u64, SysError> {
    if linux_process::current_pid()? != pid {
        return Err(SysError::ESRCH);
    }
    with_runtime(|runtime| {
        if runtime.memories.iter().any(|memory| memory.pid == pid) {
            return Err(SysError::EBUSY);
        }
        #[cfg(target_arch = "aarch64")]
        let address_space =
            Aarch64AddressSpace::new_with_kernel_map().map_err(map_address_error)?;
        #[cfg(not(target_arch = "aarch64"))]
        let address_space = FallbackAddressSpace::new(pid)?;
        let root_paddr = address_space.root_paddr();
        if root_paddr == 0 {
            return Err(SysError::ENOMEM);
        }
        runtime.memories.push(LinuxProcessMemory {
            pid,
            address_space,
            mappings: Vec::new(),
            shared_attachments: Vec::new(),
            initial_stack: None,
            next_addr: LINUX_MMAP_BASE,
            brk: BrkState::new(),
        });
        Ok(root_paddr)
    })
}

pub(crate) fn with_current<R>(
    operation: impl FnOnce(&mut LinuxProcessMemory) -> Result<R, SysError>,
) -> Result<R, SysError> {
    let pid = linux_process::current_pid()?;
    with_pid(pid, operation)
}

fn with_pid<R>(
    pid: usize,
    operation: impl FnOnce(&mut LinuxProcessMemory) -> Result<R, SysError>,
) -> Result<R, SysError> {
    with_runtime(|runtime| {
        let memory = runtime
            .memories
            .iter_mut()
            .find(|memory| memory.pid == pid)
            .ok_or(SysError::ESRCH)?;
        operation(memory)
    })
}

pub(crate) fn copy_from_process(
    pid: usize,
    address: usize,
    out: &mut [u8],
) -> Result<(), SysError> {
    if out.is_empty() {
        return Ok(());
    }
    with_pid(pid, |memory| memory.copy_from_user(address, out))
}

pub(crate) fn copy_to_process(pid: usize, address: usize, bytes: &[u8]) -> Result<(), SysError> {
    if bytes.is_empty() {
        return Ok(());
    }
    with_pid(pid, |memory| memory.copy_to_user(address, bytes))
}

pub(crate) fn reset_launch() {
    with_runtime(|runtime| {
        runtime.memories.clear();
    });
    with_shared_pages(|runtime| {
        for page in runtime.pages.drain(..) {
            PageFrameAllocator::free(page.pfn);
        }
        runtime.mmap_objects.clear();
        runtime.next_mmap_object_id = u32::MAX;
    });
}

pub(crate) fn unregister(pid: usize) -> bool {
    with_runtime(|runtime| {
        let pids = runtime
            .memories
            .iter()
            .map(|memory| memory.pid)
            .collect::<Vec<_>>();
        let Some(index) = linux_process_memory_remove_index(&pids, pid) else {
            return false;
        };
        runtime.memories.remove(index);
        true
    })
}

pub(crate) fn clone_for_fork(
    parent_pid: usize,
    child_pid: usize,
    mut shared_attachments: Vec<LinuxSharedAttachmentClone>,
) -> Result<u64, SysError> {
    let result = with_runtime(|runtime| {
        if runtime
            .memories
            .iter()
            .any(|memory| memory.pid == child_pid)
        {
            return Err(SysError::EBUSY);
        }
        runtime
            .memories
            .try_reserve(1)
            .map_err(|_| SysError::ENOMEM)?;
        let parent = runtime
            .memories
            .iter()
            .find(|memory| memory.pid == parent_pid)
            .ok_or(SysError::ESRCH)?;

        #[cfg(target_arch = "aarch64")]
        let address_space =
            Aarch64AddressSpace::new_with_kernel_map().map_err(map_address_error)?;
        #[cfg(not(target_arch = "aarch64"))]
        let address_space = FallbackAddressSpace::new(child_pid)?;
        let root_paddr = address_space.root_paddr();
        if root_paddr == 0 || root_paddr == parent.address_space.root_paddr() {
            return Err(SysError::ENOMEM);
        }

        let mut child = LinuxProcessMemory {
            pid: child_pid,
            address_space,
            mappings: Vec::new(),
            shared_attachments: Vec::new(),
            initial_stack: parent.initial_stack,
            next_addr: parent.next_addr,
            brk: BrkState {
                start: parent.brk.start,
                current: parent.brk.current,
                limit: parent.brk.limit,
                pages: Vec::new(),
            },
        };
        child
            .mappings
            .try_reserve_exact(parent.mappings.len())
            .map_err(|_| SysError::ENOMEM)?;
        child
            .shared_attachments
            .try_reserve_exact(shared_attachments.len())
            .map_err(|_| SysError::ENOMEM)?;

        for mapping in &parent.mappings {
            let attachment_index = shared_attachments.iter().position(|attachment| {
                attachment.addr == mapping.addr
                    && attachment.len == mapping.len
                    && matches!(
                        mapping.source,
                        LinuxMappingSource::SharedMemory { id }
                            if id == attachment.object_id
                    )
            });
            let pages = if let Some(index) = attachment_index {
                let pages = core::mem::take(&mut shared_attachments[index].pages);
                if shared_attachments[index].owns_attachment_reference {
                    child.shared_attachments.push(LinuxSharedAttachmentRecord {
                        object_id: shared_attachments[index].object_id,
                        addr: shared_attachments[index].attachment_addr,
                        len: shared_attachments[index].attachment_len,
                    });
                    shared_attachments[index].owns_attachment_reference = false;
                }
                pages
            } else {
                clone_page_backings_for_fork(&mapping.pages)?
            };
            if let Err(error) = child.map_unmapped_pages(mapping.addr, &pages, mapping.prot) {
                LinuxProcessMemory::free_backings(&pages);
                return Err(error);
            }
            child.mappings.push(LinuxProcessMapping {
                addr: mapping.addr,
                len: mapping.len,
                prot: mapping.prot,
                flags: mapping.flags,
                pages,
                source: mapping.source.clone(),
            });
        }

        if shared_attachments
            .iter()
            .any(|attachment| !attachment.pages.is_empty() || attachment.owns_attachment_reference)
        {
            return Err(SysError::EINVAL);
        }

        let brk_pages = clone_page_backings_for_fork(&parent.brk.pages)?;
        if let Err(error) = child.map_unmapped_pages(
            child.brk.start,
            &brk_pages,
            LINUX_PROT_READ | LINUX_PROT_WRITE,
        ) {
            LinuxProcessMemory::free_backings(&brk_pages);
            return Err(error);
        }
        child.brk.pages = brk_pages;
        crate::kernel_lowlevel::cpu::sync_instruction_cache();
        runtime.memories.push(child);
        Ok(root_paddr)
    });
    if result.is_err() {
        release_shared_attachments(&shared_attachments);
    }
    result
}

fn clone_page_backings_for_fork(
    parent_pages: &[LinuxPageBacking],
) -> Result<Vec<LinuxPageBacking>, SysError> {
    let mut child_pages = Vec::new();
    child_pages
        .try_reserve_exact(parent_pages.len())
        .map_err(|_| SysError::ENOMEM)?;
    for parent in parent_pages.iter().copied() {
        let child = match parent {
            LinuxPageBacking::Private { pfn: parent_pfn } => {
                let Some(child_pfn) = PageFrameAllocator::alloc() else {
                    LinuxProcessMemory::free_backings(&child_pages);
                    return Err(SysError::ENOMEM);
                };
                let Some(parent_physical) = PageFrameAllocator::pfn_address(parent_pfn) else {
                    PageFrameAllocator::free(child_pfn);
                    LinuxProcessMemory::free_backings(&child_pages);
                    return Err(SysError::ENOMEM);
                };
                let Some(child_physical) = PageFrameAllocator::pfn_address(child_pfn) else {
                    PageFrameAllocator::free(child_pfn);
                    LinuxProcessMemory::free_backings(&child_pages);
                    return Err(SysError::ENOMEM);
                };
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        parent_physical as *const u8,
                        child_physical as *mut u8,
                        PAGE_SIZE,
                    );
                }
                LinuxPageBacking::Private { pfn: child_pfn }
            }
            LinuxPageBacking::Shared {
                object_id,
                page_index,
                pfn,
            } => {
                if !acquire_shared_page(object_id, page_index) {
                    LinuxProcessMemory::free_backings(&child_pages);
                    return Err(SysError::ENOMEM);
                }
                LinuxPageBacking::Shared {
                    object_id,
                    page_index,
                    pfn,
                }
            }
        };
        child_pages.push(child);
        let failure_point = match parent {
            LinuxPageBacking::Private { .. } => LinuxForkFailurePoint::PrivatePage,
            LinuxPageBacking::Shared { .. } => LinuxForkFailurePoint::SharedReference,
        };
        if super::linux_process::fork_failpoint(failure_point) {
            LinuxProcessMemory::free_backings(&child_pages);
            return Err(SysError::ENOMEM);
        }
    }
    Ok(child_pages)
}

pub(crate) fn copy_from_current(address: usize, out: &mut [u8]) -> Result<(), SysError> {
    if out.is_empty() {
        return Ok(());
    }
    with_current(|memory| memory.copy_from_user(address, out))
}

pub(crate) fn copy_to_current(address: usize, bytes: &[u8]) -> Result<(), SysError> {
    if bytes.is_empty() {
        return Ok(());
    }
    with_current(|memory| memory.copy_to_user(address, bytes))
}

pub(crate) fn zero_current(address: usize, len: usize) -> Result<(), SysError> {
    let mut offset = 0usize;
    let zeros = [0u8; 256];
    while offset < len {
        let chunk = core::cmp::min(zeros.len(), len - offset);
        copy_to_current(
            address.checked_add(offset).ok_or(SysError::EFAULT)?,
            &zeros[..chunk],
        )?;
        offset += chunk;
    }
    Ok(())
}

pub(crate) fn current_root_paddr() -> Result<u64, SysError> {
    with_current(|memory| Ok(memory.address_space.root_paddr()))
}

pub(crate) fn register_initial_stack(address: usize, len: usize) -> Result<(), SysError> {
    with_current(|memory| {
        if memory.initial_stack.is_some() || !linux_user_page_range_valid(address, len) {
            return Err(SysError::EINVAL);
        }
        if !memory.range_is_mapped(address, len) {
            return Err(SysError::EFAULT);
        }
        memory.initial_stack = Some((address, len));
        Ok(())
    })
}

pub(crate) fn map_current_with_contents(
    requested: Option<usize>,
    len: usize,
    prot: usize,
    flags: usize,
    source: LinuxMappingSource,
    replace: bool,
    contents: &[u8],
) -> Result<usize, SysError> {
    with_current(|memory| memory.map(requested, len, prot, flags, source, replace, contents))
}

pub(crate) fn protect_current(address: usize, len: usize, prot: usize) -> Result<(), SysError> {
    with_current(|memory| memory.protect(address, len, prot))
}

pub(crate) fn unmap_current(address: usize, len: usize) -> Result<Vec<(u32, usize)>, SysError> {
    with_current(|memory| memory.unmap(address, len))
}

pub(crate) fn shared_attachment_current(id: Option<u32>, address: usize) -> Option<(u32, usize)> {
    with_current(|memory| {
        Ok(memory.shared_attachments.iter().find_map(|attachment| {
            (attachment.addr == address && id.map_or(true, |id| id == attachment.object_id))
                .then_some((attachment.object_id, attachment.len))
        }))
    })
    .ok()
    .flatten()
}

pub(crate) fn brk_current(new_brk: usize) -> Result<usize, SysError> {
    with_current(|memory| memory.update_brk(new_brk))
}

pub(crate) fn remap_current(
    old_address: usize,
    old_len: usize,
    new_len: usize,
    may_move: bool,
    fixed: Option<usize>,
    dont_unmap: bool,
) -> Result<usize, SysError> {
    with_current(|memory| memory.remap(old_address, old_len, new_len, may_move, fixed, dont_unmap))
}

pub(crate) fn mark_shared(address: usize, len: usize, object_id: u32) -> bool {
    with_current(|memory| {
        let Some(mapping_index) = memory
            .mappings
            .iter()
            .position(|mapping| mapping.addr == address && mapping.len == len)
        else {
            return Ok(false);
        };
        let originals = memory.mappings[mapping_index].pages.clone();
        let mut shared = Vec::with_capacity(originals.len());
        let mut acquired = Vec::new();
        for (page_index, original) in originals.iter().copied().enumerate() {
            let Some(pfn) = acquire_or_register_shared_page(object_id, page_index, original.pfn())
            else {
                SelfContainedSharedRollback::release(&acquired);
                return Ok(false);
            };
            acquired.push((object_id, page_index));
            shared.push(LinuxPageBacking::Shared {
                object_id,
                page_index,
                pfn,
            });
        }

        let pages = memory.mapped_pages_overlapping(address, len);
        if memory.unmap_pages_transactionally(&pages).is_err() {
            SelfContainedSharedRollback::release(&acquired);
            return Ok(false);
        }
        if memory
            .map_unmapped_pages(address, &shared, pages[0].prot)
            .is_err()
        {
            memory.restore_mapped_pages(&pages);
            SelfContainedSharedRollback::release(&acquired);
            return Ok(false);
        }
        let mapping = &mut memory.mappings[mapping_index];
        mapping.source = LinuxMappingSource::SharedMemory { id: object_id };
        let replaced = core::mem::replace(&mut mapping.pages, shared);
        memory.shared_attachments.push(LinuxSharedAttachmentRecord {
            object_id,
            addr: address,
            len,
        });

        for (page_index, backing) in replaced.iter().copied().enumerate() {
            let canonical_pfn = shared_page(object_id, page_index).unwrap().pfn;
            match backing {
                LinuxPageBacking::Private { pfn } => {
                    if pfn != canonical_pfn {
                        PageFrameAllocator::free(pfn);
                    }
                }
                LinuxPageBacking::Shared {
                    object_id,
                    page_index,
                    ..
                } => {
                    if let Some(pfn) = release_shared_page(object_id, page_index) {
                        if pfn != canonical_pfn {
                            PageFrameAllocator::free(pfn);
                        }
                    }
                }
            }
        }
        Ok(true)
    })
    .unwrap_or(false)
}

struct SelfContainedSharedRollback;

impl SelfContainedSharedRollback {
    fn release(acquired: &[(u32, usize)]) {
        for (object_id, page_index) in acquired.iter().copied().rev() {
            let _ = release_shared_page(object_id, page_index);
        }
    }
}

pub(crate) fn reserve_shared_attachments(
    pid: usize,
) -> Result<Vec<LinuxSharedAttachmentClone>, SysError> {
    let attachments = with_pid(pid, |memory| {
        Ok(memory
            .mappings
            .iter()
            .filter_map(|mapping| {
                let LinuxMappingSource::SharedMemory { id } = mapping.source else {
                    return None;
                };
                let attachment = memory.shared_attachments.iter().find(|attachment| {
                    attachment.object_id == id
                        && linux_shared_attachment_has_mapping(
                            **attachment,
                            &[LinuxSharedMappingRange {
                                object_id: id,
                                addr: mapping.addr,
                                len: mapping.len,
                            }],
                        )
                })?;
                Some(LinuxSharedAttachmentClone {
                    object_id: id,
                    attachment_addr: attachment.addr,
                    attachment_len: attachment.len,
                    addr: mapping.addr,
                    len: mapping.len,
                    prot: mapping.prot,
                    flags: mapping.flags,
                    pages: mapping.pages.clone(),
                    owns_attachment_reference: false,
                })
            })
            .collect::<Vec<_>>())
    })?;

    let mut acquired = Vec::new();
    for mut attachment in attachments {
        let owns_attachment_reference =
            !acquired
                .iter()
                .any(|acquired: &LinuxSharedAttachmentClone| {
                    acquired.object_id == attachment.object_id
                        && acquired.attachment_addr == attachment.attachment_addr
                });
        if owns_attachment_reference
            && !super::acquire_shared_memory_attachment_reference(attachment.object_id)
        {
            release_shared_attachments(&acquired);
            return Err(SysError::ENOMEM);
        }
        if owns_attachment_reference
            && super::linux_process::fork_failpoint(LinuxForkFailurePoint::SharedReference)
        {
            release_shared_attachments(&acquired);
            let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
            return Err(SysError::ENOMEM);
        }
        let mut local_acquired = Vec::new();
        for backing in &attachment.pages {
            let LinuxPageBacking::Shared {
                object_id,
                page_index,
                ..
            } = *backing
            else {
                for (object_id, page_index) in local_acquired.into_iter().rev() {
                    let _ = release_shared_page(object_id, page_index);
                }
                release_shared_attachments(&acquired);
                if owns_attachment_reference {
                    let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
                }
                return Err(SysError::EINVAL);
            };
            if !acquire_shared_page(object_id, page_index) {
                for (object_id, page_index) in local_acquired.into_iter().rev() {
                    let _ = release_shared_page(object_id, page_index);
                }
                release_shared_attachments(&acquired);
                if owns_attachment_reference {
                    let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
                }
                return Err(SysError::ENOMEM);
            }
            local_acquired.push((object_id, page_index));
            if super::linux_process::fork_failpoint(LinuxForkFailurePoint::SharedReference) {
                for (object_id, page_index) in local_acquired.into_iter().rev() {
                    let _ = release_shared_page(object_id, page_index);
                }
                release_shared_attachments(&acquired);
                if owns_attachment_reference {
                    let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
                }
                return Err(SysError::ENOMEM);
            }
        }
        attachment.owns_attachment_reference = owns_attachment_reference;
        acquired.push(attachment);
    }
    Ok(acquired)
}

pub(crate) fn release_shared_attachments(attachments: &[LinuxSharedAttachmentClone]) {
    for attachment in attachments.iter().rev() {
        for backing in attachment.pages.iter().rev() {
            if let LinuxPageBacking::Shared {
                object_id,
                page_index,
                ..
            } = *backing
            {
                if let Some(pfn) = release_shared_page(object_id, page_index) {
                    PageFrameAllocator::free(pfn);
                }
            }
        }
        if attachment.owns_attachment_reference {
            let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
        }
    }
}

pub(crate) fn user_range_readable(address: usize, len: usize) -> bool {
    with_current(|memory| Ok(memory.range_accessible(address, len, false))).unwrap_or(false)
}

pub(crate) fn user_range_writable(address: usize, len: usize) -> bool {
    with_current(|memory| Ok(memory.range_accessible(address, len, true))).unwrap_or(false)
}

pub(crate) fn current_stats() -> Option<LinuxMemoryStats> {
    with_current(|memory| Ok(memory.stats())).ok()
}

pub(crate) fn current_snapshots() -> Vec<LinuxMemoryMappingSnapshot> {
    with_current(|memory| {
        Ok(memory
            .mappings
            .iter()
            .map(|mapping| LinuxMemoryMappingSnapshot {
                addr: mapping.addr,
                len: mapping.len,
                prot: mapping.prot,
                flags: mapping.flags,
                pfns: mapping.pages.iter().map(|page| page.pfn()).collect(),
                source: mapping.source.clone(),
            })
            .collect())
    })
    .unwrap_or_default()
}

pub(crate) fn current_shared_attachments() -> Vec<LinuxSharedAttachmentRecord> {
    with_current(|memory| Ok(memory.shared_attachments.clone())).unwrap_or_default()
}

pub(crate) fn total_mapping_count() -> usize {
    with_runtime(|runtime| {
        runtime
            .memories
            .iter()
            .map(|memory| memory.mappings.len())
            .sum()
    })
}

impl LinuxProcessMemory {
    fn page_permissions(prot: usize) -> (bool, bool, bool) {
        (
            prot & (LINUX_PROT_READ | LINUX_PROT_WRITE) != 0,
            prot & LINUX_PROT_WRITE != 0,
            prot & LINUX_PROT_EXEC != 0,
        )
    }

    fn map_page(&mut self, address: usize, pfn: u64, prot: usize) -> Result<(), SysError> {
        let (readable, writable, executable) = Self::page_permissions(prot);
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .map_user_page(address, pfn, readable, writable, executable)
            .map_err(map_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space
            .map_user_page(address, pfn, readable, writable, executable)
    }

    fn protect_page(&mut self, address: usize, prot: usize) -> Result<(), SysError> {
        let (readable, writable, executable) = Self::page_permissions(prot);
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .protect_user_page(address, readable, writable, executable)
            .map_err(map_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space
            .protect_user_page(address, readable, writable, executable)
    }

    fn unmap_page(&mut self, address: usize) -> Result<u64, SysError> {
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .unmap_user_page(address)
            .map_err(map_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space.unmap_user_page(address)
    }

    fn copy_to_user(&self, address: usize, bytes: &[u8]) -> Result<(), SysError> {
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .copy_to_user(address, bytes)
            .map_err(map_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space.copy_to_user(address, bytes)
    }

    fn copy_from_user(&self, address: usize, out: &mut [u8]) -> Result<(), SysError> {
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .copy_from_user(address, out)
            .map_err(map_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space.copy_from_user(address, out)
    }

    fn range_accessible(&self, address: usize, len: usize, write: bool) -> bool {
        if len == 0 || address.checked_add(len).is_none() {
            return false;
        }
        let mut offset = 0usize;
        while offset < len {
            let current = match address.checked_add(offset) {
                Some(current) => current,
                None => return false,
            };
            #[cfg(target_arch = "aarch64")]
            if self.address_space.translate_user(current, write).is_none() {
                return false;
            }
            #[cfg(not(target_arch = "aarch64"))]
            if !super::syscall_logic::user_buffer_valid(current, 1) {
                return false;
            }
            let Some(chunk) = linux_user_copy_chunk(current, len - offset, PAGE_SIZE) else {
                return false;
            };
            offset += chunk;
        }
        true
    }

    fn range_is_mapped(&self, address: usize, len: usize) -> bool {
        linux_mapping_range_covered(&self.mapping_ranges(), address, len)
    }

    fn range_available(&self, address: usize, len: usize) -> bool {
        linux_user_page_range_valid(address, len)
            && address >= LINUX_MMAP_BASE
            && address.checked_add(len).unwrap_or(usize::MAX) <= LINUX_BRK_BASE
            && self.fixed_range_available(address, len)
    }

    fn fixed_range_available(&self, address: usize, len: usize) -> bool {
        linux_user_page_range_valid(address, len)
            && !ranges_overlap(
                address,
                len,
                self.brk.start,
                self.brk.limit - self.brk.start,
            )
            && !self
                .mappings
                .iter()
                .any(|mapping| ranges_overlap(address, len, mapping.addr, mapping.len))
    }

    fn find_free_region(&self, requested: Option<usize>, len: usize) -> Option<usize> {
        if let Some(address) = requested {
            if self.fixed_range_available(address, len) {
                return Some(address);
            }
        }
        let mut candidate = self.next_addr.max(LINUX_MMAP_BASE);
        for mapping in &self.mappings {
            if mapping.addr >= LINUX_BRK_BASE {
                break;
            }
            let candidate_end = candidate.checked_add(len)?;
            if candidate_end <= mapping.addr {
                return Some(candidate);
            }
            let mapping_end = mapping.addr.checked_add(mapping.len)?;
            if mapping_end > candidate {
                candidate = mapping_end;
            }
        }
        let end = candidate.checked_add(len)?;
        if end > LINUX_BRK_BASE {
            return None;
        }
        Some(candidate)
    }

    fn allocate_unmapped_pages(
        &self,
        len: usize,
        contents: &[u8],
    ) -> Result<Vec<LinuxPageBacking>, SysError> {
        if contents.len() > len {
            return Err(SysError::EINVAL);
        }
        let page_count = len / PAGE_SIZE;
        let mut pages = Vec::with_capacity(page_count);
        for _ in 0..page_count {
            let Some(pfn) = PageFrameAllocator::alloc() else {
                Self::free_backings(&pages);
                return Err(SysError::ENOMEM);
            };
            let Some(physical) = PageFrameAllocator::pfn_address(pfn) else {
                PageFrameAllocator::free(pfn);
                Self::free_backings(&pages);
                return Err(SysError::ENOMEM);
            };
            unsafe { core::ptr::write_bytes(physical as *mut u8, 0, PAGE_SIZE) };
            pages.push(LinuxPageBacking::Private { pfn });
        }

        let mut copied = 0usize;
        for page in &pages {
            if copied == contents.len() {
                break;
            }
            let chunk = core::cmp::min(PAGE_SIZE, contents.len() - copied);
            let Some(physical) = PageFrameAllocator::pfn_address(page.pfn()) else {
                Self::free_backings(&pages);
                return Err(SysError::ENOMEM);
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    contents[copied..copied + chunk].as_ptr(),
                    physical as *mut u8,
                    chunk,
                );
            }
            copied += chunk;
        }
        Ok(pages)
    }

    fn allocate_shared_mmap_pages(
        &self,
        len: usize,
        contents: &[u8],
        source: &LinuxMappingSource,
    ) -> Result<Vec<LinuxPageBacking>, SysError> {
        let candidates = self.allocate_unmapped_pages(len, contents)?;
        let (object_id, first_page) = match shared_mmap_object(source) {
            Ok(object) => object,
            Err(error) => {
                Self::free_backings(&candidates);
                return Err(error);
            }
        };
        let mut shared = Vec::new();
        if shared.try_reserve_exact(candidates.len()).is_err() {
            Self::free_backings(&candidates);
            remove_empty_shared_mmap_object(object_id);
            return Err(SysError::ENOMEM);
        }
        for (index, candidate) in candidates.iter().copied().enumerate() {
            let Some(page_index) = linux_shared_page_index(first_page, index) else {
                Self::free_backings(&shared);
                for candidate in &candidates[index..] {
                    PageFrameAllocator::free(candidate.pfn());
                }
                remove_empty_shared_mmap_object(object_id);
                return Err(SysError::EINVAL);
            };
            let Some(pfn) = acquire_or_register_shared_page(object_id, page_index, candidate.pfn())
            else {
                Self::free_backings(&shared);
                for candidate in &candidates[index..] {
                    PageFrameAllocator::free(candidate.pfn());
                }
                remove_empty_shared_mmap_object(object_id);
                return Err(SysError::ENOMEM);
            };
            if pfn != candidate.pfn() {
                PageFrameAllocator::free(candidate.pfn());
            }
            shared.push(LinuxPageBacking::Shared {
                object_id,
                page_index,
                pfn,
            });
        }
        Ok(shared)
    }

    fn free_backings(pages: &[LinuxPageBacking]) {
        for page in pages {
            match *page {
                LinuxPageBacking::Private { pfn } => PageFrameAllocator::free(pfn),
                LinuxPageBacking::Shared {
                    object_id,
                    page_index,
                    ..
                } => {
                    if let Some(pfn) = release_shared_page(object_id, page_index) {
                        PageFrameAllocator::free(pfn);
                    }
                }
            }
        }
    }

    fn map_unmapped_pages(
        &mut self,
        address: usize,
        pages: &[LinuxPageBacking],
        prot: usize,
    ) -> Result<(), SysError> {
        let mut mapped = 0usize;
        for (page_index, page) in pages.iter().enumerate() {
            let page_address = address + page_index * PAGE_SIZE;
            if let Err(error) = self.map_page(page_address, page.pfn(), prot) {
                for rollback in (0..mapped).rev() {
                    let _ = self.unmap_page(address + rollback * PAGE_SIZE);
                }
                return Err(error);
            }
            mapped += 1;
        }
        Ok(())
    }

    fn mapping_ranges(&self) -> Vec<LinuxMappingRange> {
        self.mappings
            .iter()
            .map(|mapping| LinuxMappingRange {
                addr: mapping.addr,
                len: mapping.len,
            })
            .collect()
    }

    fn mapped_pages_overlapping(&self, address: usize, len: usize) -> Vec<LinuxMappedPage> {
        let end = address + len;
        let mut pages = Vec::new();
        for mapping in &self.mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
            let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
            for page_index in start_page..end_page {
                pages.push(LinuxMappedPage {
                    address: mapping.addr + page_index * PAGE_SIZE,
                    backing: mapping.pages[page_index],
                    prot: mapping.prot,
                });
            }
        }
        pages.sort_by_key(|page| page.address);
        pages
    }

    fn restore_mapped_pages(&mut self, pages: &[LinuxMappedPage]) {
        for page in pages {
            let _ = self.map_page(page.address, page.backing.pfn(), page.prot);
        }
    }

    fn unmap_pages_transactionally(&mut self, pages: &[LinuxMappedPage]) -> Result<(), SysError> {
        let mut removed = 0usize;
        for page in pages {
            if let Err(error) = self.unmap_page(page.address) {
                self.restore_mapped_pages(&pages[..removed]);
                return Err(error);
            }
            removed += 1;
        }
        Ok(())
    }

    fn protect_pages_transactionally(
        &mut self,
        pages: &[LinuxMappedPage],
        prot: usize,
    ) -> Result<(), SysError> {
        let mut changed = 0usize;
        for page in pages {
            if let Err(error) = self.protect_page(page.address, prot) {
                let _ = self.map_page(page.address, page.backing.pfn(), page.prot);
                for rollback in pages[..changed].iter().rev() {
                    let _ = self.protect_page(rollback.address, rollback.prot);
                }
                return Err(error);
            }
            changed += 1;
        }
        Ok(())
    }

    fn map(
        &mut self,
        requested: Option<usize>,
        len: usize,
        prot: usize,
        flags: usize,
        source: LinuxMappingSource,
        replace: bool,
        contents: &[u8],
    ) -> Result<usize, SysError> {
        if len == 0 || len % PAGE_SIZE != 0 || contents.len() > len {
            return Err(SysError::EINVAL);
        }
        let address = if replace {
            let address = requested.ok_or(SysError::EINVAL)?;
            if !linux_user_page_range_valid(address, len)
                || ranges_overlap(
                    address,
                    len,
                    self.brk.start,
                    self.brk.limit - self.brk.start,
                )
            {
                return Err(SysError::EINVAL);
            }
            address
        } else {
            self.find_free_region(requested, len)
                .ok_or(SysError::ENOMEM)?
        };
        let pages = if flags & LINUX_MAP_SHARED != 0 {
            self.allocate_shared_mmap_pages(len, contents, &source)?
        } else {
            self.allocate_unmapped_pages(len, contents)?
        };
        if replace {
            if let Err((error, pages)) =
                self.replace_mapping_transactionally(address, len, prot, flags, source, pages)
            {
                Self::free_backings(&pages);
                return Err(error);
            }
        } else {
            if let Err(error) = self.map_unmapped_pages(address, &pages, prot) {
                Self::free_backings(&pages);
                return Err(error);
            }
            self.mappings.push(LinuxProcessMapping {
                addr: address,
                len,
                prot,
                flags,
                pages,
                source,
            });
            self.mappings.sort_by_key(|mapping| mapping.addr);
        }
        if (LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&address) {
            self.next_addr = address.checked_add(len).ok_or(SysError::ENOMEM)?;
        }
        Ok(address)
    }

    fn replace_mapping_transactionally(
        &mut self,
        address: usize,
        len: usize,
        prot: usize,
        flags: usize,
        source: LinuxMappingSource,
        pages: Vec<LinuxPageBacking>,
    ) -> Result<(), (SysError, Vec<LinuxPageBacking>)> {
        let old_pages = self.mapped_pages_overlapping(address, len);
        if let Err(error) = self.unmap_pages_transactionally(&old_pages) {
            return Err((error, pages));
        }
        if let Err(error) = self.map_unmapped_pages(address, &pages, prot) {
            self.restore_mapped_pages(&old_pages);
            return Err((error, pages));
        }

        let replaced = self.replace_mapping_metadata(
            address,
            len,
            LinuxProcessMapping {
                addr: address,
                len,
                prot,
                flags,
                pages,
                source,
            },
        );
        Self::free_backings(&replaced);
        Ok(())
    }

    fn replace_mapping_metadata(
        &mut self,
        address: usize,
        len: usize,
        replacement: LinuxProcessMapping,
    ) -> Vec<LinuxPageBacking> {
        let end = address + len;
        let mut replaced = Vec::new();
        let mappings = core::mem::take(&mut self.mappings);
        for mapping in mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                self.mappings.push(mapping);
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
            let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
            replaced.extend_from_slice(&mapping.pages[start_page..end_page]);
            self.push_mapping_pieces(mapping, overlap_start, overlap_end, None);
        }
        self.mappings.push(replacement);
        self.mappings.sort_by_key(|mapping| mapping.addr);
        replaced
    }

    fn protect(&mut self, address: usize, len: usize, prot: usize) -> Result<(), SysError> {
        let ranges = self.mapping_ranges();
        if !linux_mapping_range_covered(&ranges, address, len) {
            return Err(SysError::EINVAL);
        }
        let pages = self.mapped_pages_overlapping(address, len);
        self.protect_pages_transactionally(&pages, prot)?;
        self.update_mapping_protections(address, len, prot);
        Ok(())
    }

    fn update_mapping_protections(&mut self, address: usize, len: usize, prot: usize) {
        let end = address + len;
        let mappings = core::mem::take(&mut self.mappings);
        for mapping in mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                self.mappings.push(mapping);
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            self.push_mapping_pieces(mapping, overlap_start, overlap_end, Some(prot));
        }
        self.mappings.sort_by_key(|mapping| mapping.addr);
    }

    fn unmap(&mut self, address: usize, len: usize) -> Result<Vec<(u32, usize)>, SysError> {
        address.checked_add(len).ok_or(SysError::EINVAL)?;
        let pages = self.mapped_pages_overlapping(address, len);
        if pages.is_empty() {
            return Err(SysError::EINVAL);
        }
        self.unmap_pages_transactionally(&pages)?;
        let (detached, removed) = self.remove_mapping_metadata(address, len);
        Self::free_backings(&removed);
        Ok(detached)
    }

    fn remove_mapping_metadata(
        &mut self,
        address: usize,
        len: usize,
    ) -> (Vec<(u32, usize)>, Vec<LinuxPageBacking>) {
        let end = address + len;
        let mut detached = Vec::new();
        let mut removed = Vec::new();
        let mappings = core::mem::take(&mut self.mappings);
        for mapping in mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                self.mappings.push(mapping);
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
            let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
            removed.extend_from_slice(&mapping.pages[start_page..end_page]);
            self.push_mapping_pieces(mapping, overlap_start, overlap_end, None);
        }
        self.mappings.sort_by_key(|mapping| mapping.addr);
        let shared_mappings = self
            .mappings
            .iter()
            .filter_map(|mapping| match mapping.source {
                LinuxMappingSource::SharedMemory { id } => Some(LinuxSharedMappingRange {
                    object_id: id,
                    addr: mapping.addr,
                    len: mapping.len,
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        self.shared_attachments.retain(|attachment| {
            if linux_shared_attachment_has_mapping(*attachment, &shared_mappings) {
                true
            } else {
                detached.push((attachment.object_id, attachment.addr));
                false
            }
        });
        (detached, removed)
    }

    fn push_mapping_pieces(
        &mut self,
        mut mapping: LinuxProcessMapping,
        overlap_start: usize,
        overlap_end: usize,
        replacement_prot: Option<usize>,
    ) {
        let mapping_end = mapping.addr + mapping.len;
        let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
        let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
        let original_pages = core::mem::take(&mut mapping.pages);
        if overlap_start > mapping.addr {
            self.mappings.push(LinuxProcessMapping {
                addr: mapping.addr,
                len: overlap_start - mapping.addr,
                prot: mapping.prot,
                flags: mapping.flags,
                pages: original_pages[..start_page].to_vec(),
                source: mapping.source.slice(0),
            });
        }
        if let Some(prot) = replacement_prot {
            self.mappings.push(LinuxProcessMapping {
                addr: overlap_start,
                len: overlap_end - overlap_start,
                prot,
                flags: mapping.flags,
                pages: original_pages[start_page..end_page].to_vec(),
                source: mapping.source.slice(overlap_start - mapping.addr),
            });
        }
        if overlap_end < mapping_end {
            self.mappings.push(LinuxProcessMapping {
                addr: overlap_end,
                len: mapping_end - overlap_end,
                prot: mapping.prot,
                flags: mapping.flags,
                pages: original_pages[end_page..].to_vec(),
                source: mapping.source.slice(overlap_end - mapping.addr),
            });
        }
    }

    fn update_brk(&mut self, new_brk: usize) -> Result<usize, SysError> {
        if new_brk == 0 {
            return Ok(self.brk.current);
        }
        if new_brk < self.brk.start || new_brk > self.brk.limit {
            return Ok(self.brk.current);
        }
        let old_brk = self.brk.current;
        let old_pages = page_count(old_brk - self.brk.start);
        let new_pages = page_count(new_brk - self.brk.start);
        if new_pages > old_pages {
            let start = self.brk.start + old_pages * PAGE_SIZE;
            let len = (new_pages - old_pages) * PAGE_SIZE;
            let pages = self.allocate_unmapped_pages(len, &[])?;
            if let Err(error) =
                self.map_unmapped_pages(start, &pages, LINUX_PROT_READ | LINUX_PROT_WRITE)
            {
                Self::free_backings(&pages);
                return Err(error);
            }
            if let Err(error) = self.zero_user_range(old_brk, new_brk - old_brk) {
                let mapped = pages
                    .iter()
                    .enumerate()
                    .map(|(index, backing)| LinuxMappedPage {
                        address: start + index * PAGE_SIZE,
                        backing: *backing,
                        prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
                    })
                    .collect::<Vec<_>>();
                let _ = self.unmap_pages_transactionally(&mapped);
                Self::free_backings(&pages);
                return Err(error);
            }
            self.brk.pages.extend(pages);
        } else if new_pages < old_pages {
            let removed = self.brk.pages[new_pages..old_pages]
                .iter()
                .enumerate()
                .map(|(offset, backing)| LinuxMappedPage {
                    address: self.brk.start + (new_pages + offset) * PAGE_SIZE,
                    backing: *backing,
                    prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
                })
                .collect::<Vec<_>>();
            self.unmap_pages_transactionally(&removed)?;
            let backings = self.brk.pages.split_off(new_pages);
            Self::free_backings(&backings);
        } else if new_brk > old_brk {
            self.zero_user_range(old_brk, new_brk - old_brk)?;
        }
        self.brk.current = new_brk;
        Ok(self.brk.current)
    }

    fn zero_user_range(&self, address: usize, len: usize) -> Result<(), SysError> {
        let zeros = [0u8; 256];
        let mut offset = 0usize;
        while offset < len {
            let chunk = core::cmp::min(zeros.len(), len - offset);
            self.copy_to_user(address + offset, &zeros[..chunk])?;
            offset += chunk;
        }
        Ok(())
    }

    fn rollback_remap_destination(
        &mut self,
        address: usize,
        pages: &[LinuxPageBacking],
        prot: usize,
        replaced: &[LinuxMappedPage],
    ) {
        let mapped = pages
            .iter()
            .enumerate()
            .map(|(index, backing)| LinuxMappedPage {
                address: address + index * PAGE_SIZE,
                backing: *backing,
                prot,
            })
            .collect::<Vec<_>>();
        let _ = self.unmap_pages_transactionally(&mapped);
        self.restore_mapped_pages(replaced);
    }

    fn remap(
        &mut self,
        old_address: usize,
        old_len: usize,
        new_len: usize,
        may_move: bool,
        fixed: Option<usize>,
        dont_unmap: bool,
    ) -> Result<usize, SysError> {
        let index = self
            .mappings
            .iter()
            .position(|mapping| mapping.addr == old_address && mapping.len == old_len)
            .ok_or(SysError::EINVAL)?;
        let requires_move =
            linux_mremap_requires_move(old_address, old_len, new_len, fixed, dont_unmap);
        if linux_mmap_backing_is_shared(self.mappings[index].flags)
            && !linux_shared_mremap_supported(requires_move)
        {
            return Err(SysError::EINVAL);
        }
        if !requires_move {
            return Ok(old_address);
        }
        if dont_unmap && old_len != new_len {
            return Err(SysError::EINVAL);
        }
        if fixed.is_none() && !dont_unmap && new_len < old_len {
            self.unmap(old_address + new_len, old_len - new_len)?;
            return Ok(old_address);
        }
        let grow_start = old_address + old_len;
        let extra_len = new_len.saturating_sub(old_len);
        if fixed.is_none()
            && !dont_unmap
            && new_len > old_len
            && self.range_available(grow_start, extra_len)
        {
            let prot = self.mappings[index].prot;
            let pages = self.allocate_unmapped_pages(extra_len, &[])?;
            if let Err(error) = self.map_unmapped_pages(grow_start, &pages, prot) {
                Self::free_backings(&pages);
                return Err(error);
            }
            self.mappings[index].len = new_len;
            self.mappings[index].pages.extend(pages);
            return Ok(old_address);
        }
        if !may_move {
            return Err(SysError::ENOMEM);
        }
        let prot = self.mappings[index].prot;
        let flags = self.mappings[index].flags;
        let source = self.mappings[index].source.clone();
        let new_address = if let Some(address) = fixed {
            if !linux_user_page_range_valid(address, new_len)
                || ranges_overlap(address, new_len, old_address, old_len)
                || ranges_overlap(
                    address,
                    new_len,
                    self.brk.start,
                    self.brk.limit - self.brk.start,
                )
            {
                return Err(SysError::EINVAL);
            }
            address
        } else {
            self.find_free_region(None, new_len)
                .ok_or(SysError::ENOMEM)?
        };
        let copy_len = core::cmp::min(old_len, new_len);
        let mut contents = Vec::new();
        contents
            .try_reserve_exact(copy_len)
            .map_err(|_| SysError::ENOMEM)?;
        contents.resize(copy_len, 0);
        self.copy_from_user(old_address, &mut contents)?;
        let pages = self.allocate_unmapped_pages(new_len, &contents)?;
        let replaced = self.mapped_pages_overlapping(new_address, new_len);
        if let Err(error) = self.unmap_pages_transactionally(&replaced) {
            Self::free_backings(&pages);
            return Err(error);
        }
        if let Err(error) = self.map_unmapped_pages(new_address, &pages, prot) {
            self.restore_mapped_pages(&replaced);
            Self::free_backings(&pages);
            return Err(error);
        }
        if !dont_unmap {
            let source_pages = self.mapped_pages_overlapping(old_address, old_len);
            if let Err(error) = self.unmap_pages_transactionally(&source_pages) {
                self.rollback_remap_destination(new_address, &pages, prot, &replaced);
                Self::free_backings(&pages);
                return Err(error);
            }
        }

        let replaced_backings = self.replace_mapping_metadata(
            new_address,
            new_len,
            LinuxProcessMapping {
                addr: new_address,
                len: new_len,
                prot,
                flags,
                pages,
                source,
            },
        );
        Self::free_backings(&replaced_backings);
        if !dont_unmap {
            let (_, source_backings) = self.remove_mapping_metadata(old_address, old_len);
            Self::free_backings(&source_backings);
        }
        if (LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&new_address) {
            self.next_addr = new_address.checked_add(new_len).ok_or(SysError::ENOMEM)?;
        }
        Ok(new_address)
    }

    fn stats(&self) -> LinuxMemoryStats {
        LinuxMemoryStats {
            mapping_count: self.mappings.len(),
            mapped_bytes: self.mappings.iter().map(|mapping| mapping.len).sum(),
            committed_pages: self
                .mappings
                .iter()
                .map(|mapping| mapping.pages.len())
                .sum(),
            brk_start: self.brk.start,
            brk_current: self.brk.current,
            brk_limit: self.brk.limit,
            brk_committed_pages: self.brk.pages.len(),
        }
    }

    fn release_all_pages(&mut self) {
        let mappings = core::mem::take(&mut self.mappings);
        for mapping in mappings {
            Self::free_backings(&mapping.pages);
        }
        let brk_pages = core::mem::take(&mut self.brk.pages);
        Self::free_backings(&brk_pages);
    }
}

impl Drop for LinuxProcessMemory {
    fn drop(&mut self) {
        self.release_all_pages();
        for attachment in self.shared_attachments.drain(..).rev() {
            let _ = super::release_shared_memory_attachment_reference(attachment.object_id);
        }
    }
}

fn page_count(len: usize) -> usize {
    len.saturating_add(PAGE_SIZE - 1) / PAGE_SIZE
}

fn ranges_overlap(first: usize, first_len: usize, second: usize, second_len: usize) -> bool {
    match (first.checked_add(first_len), second.checked_add(second_len)) {
        (Some(first_end), Some(second_end)) => first < second_end && second < first_end,
        _ => false,
    }
}

#[cfg(target_arch = "aarch64")]
fn map_address_error(error: crate::kernel_lowlevel::mmu::AddressSpaceError) -> SysError {
    use crate::kernel_lowlevel::mmu::AddressSpaceError;
    match error {
        AddressSpaceError::OutOfMemory => SysError::ENOMEM,
        AddressSpaceError::PermissionDenied => SysError::EFAULT,
        AddressSpaceError::InvalidAddress
        | AddressSpaceError::InvalidPermissions
        | AddressSpaceError::AlreadyMapped
        | AddressSpaceError::NotMapped => SysError::EINVAL,
    }
}
