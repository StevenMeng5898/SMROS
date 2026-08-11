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
    fn try_clone_for_fork(&self) -> Result<Self, SysError> {
        self.try_slice(0)
    }

    fn try_slice(&self, delta: usize) -> Result<Self, SysError> {
        match self {
            Self::Anonymous => Ok(Self::Anonymous),
            Self::File { fd, offset, path } => {
                let mut sliced_path = String::new();
                sliced_path
                    .try_reserve_exact(path.len())
                    .map_err(|_| SysError::ENOMEM)?;
                sliced_path.push_str(path);
                Ok(Self::File {
                    fd: *fd,
                    offset: offset.saturating_add(delta as u64),
                    path: sliced_path,
                })
            }
            Self::SharedMemory { id } => Ok(Self::SharedMemory { id: *id }),
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

struct LinuxMappingMetadataPlan {
    mappings: Vec<LinuxProcessMapping>,
    removed: Vec<LinuxPageBacking>,
    shared_attachments: Vec<LinuxSharedAttachmentRecord>,
    detached: Vec<(u32, usize)>,
}

impl LinuxMappingMetadataPlan {
    fn try_clone_mapping(mapping: &LinuxProcessMapping) -> Result<LinuxProcessMapping, SysError> {
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(mapping.pages.len())
            .map_err(|_| SysError::ENOMEM)?;
        pages.extend_from_slice(&mapping.pages);
        Ok(LinuxProcessMapping {
            addr: mapping.addr,
            len: mapping.len,
            prot: mapping.prot,
            flags: mapping.flags,
            pages,
            source: mapping.source.try_clone_for_fork()?,
        })
    }

    fn try_clone_mapping_metadata(memory: &LinuxProcessMemory) -> Result<Self, SysError> {
        let mut mappings = Vec::new();
        mappings
            .try_reserve_exact(memory.mappings.len())
            .map_err(|_| SysError::ENOMEM)?;
        for mapping in &memory.mappings {
            mappings.push(Self::try_clone_mapping(mapping)?);
        }

        let mut shared_attachments = Vec::new();
        shared_attachments
            .try_reserve_exact(memory.shared_attachments.len())
            .map_err(|_| SysError::ENOMEM)?;
        shared_attachments.extend_from_slice(&memory.shared_attachments);
        Ok(Self {
            mappings,
            removed: Vec::new(),
            shared_attachments,
            detached: Vec::new(),
        })
    }

    fn try_mapping_piece(
        mapping: &LinuxProcessMapping,
        start: usize,
        end: usize,
        prot: usize,
    ) -> Result<LinuxProcessMapping, SysError> {
        let start_page = (start - mapping.addr) / PAGE_SIZE;
        let end_page = (end - mapping.addr) / PAGE_SIZE;
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(end_page - start_page)
            .map_err(|_| SysError::ENOMEM)?;
        pages.extend_from_slice(&mapping.pages[start_page..end_page]);
        Ok(LinuxProcessMapping {
            addr: start,
            len: end - start,
            prot,
            flags: mapping.flags,
            pages,
            source: mapping.source.try_slice(start - mapping.addr)?,
        })
    }

    fn try_transform_range(
        &mut self,
        address: usize,
        len: usize,
        replacement_prot: Option<usize>,
    ) -> Result<(), SysError> {
        let end = address.checked_add(len).ok_or(SysError::EINVAL)?;
        let mut mapping_count = 0usize;
        let mut removed_count = 0usize;
        for mapping in &self.mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                mapping_count = mapping_count.checked_add(1).ok_or(SysError::ENOMEM)?;
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            mapping_count = mapping_count
                .checked_add(usize::from(overlap_start > mapping.addr))
                .and_then(|count| count.checked_add(usize::from(replacement_prot.is_some())))
                .and_then(|count| count.checked_add(usize::from(overlap_end < mapping_end)))
                .ok_or(SysError::ENOMEM)?;
            if replacement_prot.is_none() {
                removed_count = removed_count
                    .checked_add((overlap_end - overlap_start) / PAGE_SIZE)
                    .ok_or(SysError::ENOMEM)?;
            }
        }

        let mut transformed = Vec::new();
        transformed
            .try_reserve_exact(mapping_count)
            .map_err(|_| SysError::ENOMEM)?;
        self.removed
            .try_reserve_exact(removed_count)
            .map_err(|_| SysError::ENOMEM)?;

        let mappings = core::mem::take(&mut self.mappings);
        for mapping in mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                transformed.push(mapping);
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            let start_page = (overlap_start - mapping.addr) / PAGE_SIZE;
            let end_page = (overlap_end - mapping.addr) / PAGE_SIZE;
            if replacement_prot.is_none() {
                self.removed
                    .extend_from_slice(&mapping.pages[start_page..end_page]);
            }
            if overlap_start > mapping.addr {
                transformed.push(Self::try_mapping_piece(
                    &mapping,
                    mapping.addr,
                    overlap_start,
                    mapping.prot,
                )?);
            }
            if let Some(prot) = replacement_prot {
                transformed.push(Self::try_mapping_piece(
                    &mapping,
                    overlap_start,
                    overlap_end,
                    prot,
                )?);
            }
            if overlap_end < mapping_end {
                transformed.push(Self::try_mapping_piece(
                    &mapping,
                    overlap_end,
                    mapping_end,
                    mapping.prot,
                )?);
            }
        }
        self.mappings = transformed;
        Ok(())
    }

    fn try_reserve_mapping_slot(&mut self) -> Result<(), SysError> {
        self.mappings
            .try_reserve_exact(1)
            .map_err(|_| SysError::ENOMEM)
    }

    fn insert_mapping(&mut self, mapping: LinuxProcessMapping) {
        let index = self
            .mappings
            .iter()
            .position(|candidate| candidate.addr > mapping.addr)
            .unwrap_or(self.mappings.len());
        self.mappings.insert(index, mapping);
    }

    fn take_mapping(&mut self, address: usize, len: usize) -> Option<LinuxProcessMapping> {
        let index = self
            .mappings
            .iter()
            .position(|mapping| mapping.addr == address && mapping.len == len)?;
        Some(self.mappings.remove(index))
    }

    fn try_reserve_attachment_slot(&mut self) -> Result<(), SysError> {
        self.shared_attachments
            .try_reserve_exact(1)
            .map_err(|_| SysError::ENOMEM)
    }

    fn try_prepare_attachment_reconciliation(&mut self) -> Result<(), SysError> {
        self.detached
            .try_reserve_exact(self.shared_attachments.len())
            .map_err(|_| SysError::ENOMEM)
    }

    fn reconcile_shared_attachments(&mut self) {
        let mappings = &self.mappings;
        let detached = &mut self.detached;
        self.shared_attachments.retain(|attachment| {
            let has_mapping = mappings.iter().any(|mapping| {
                matches!(
                    mapping.source,
                    LinuxMappingSource::SharedMemory { id } if id == attachment.object_id
                ) && ranges_overlap(attachment.addr, attachment.len, mapping.addr, mapping.len)
            });
            if !has_mapping {
                detached.push(
                    linux_shared_attachment_detached_reference(*attachment, &[])
                        .expect("missing shared mapping detaches the attachment"),
                );
            }
            has_mapping
        });
    }
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

    fn table_page_count(&self) -> usize {
        1
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessMemoryResourceCounts {
    pub private_pages: usize,
    pub shared_pages: usize,
    pub page_table_pages: usize,
    pub(crate) process_pids: [usize; linux_process::LINUX_PROCESS_LIMIT],
    pub(crate) process_count: usize,
}

impl LinuxProcessMemoryResourceCounts {
    const fn new() -> Self {
        Self {
            private_pages: 0,
            shared_pages: 0,
            page_table_pages: 0,
            process_pids: [0; linux_process::LINUX_PROCESS_LIMIT],
            process_count: 0,
        }
    }
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
        let Some(index) = runtime.memories.iter().position(|memory| memory.pid == pid) else {
            return false;
        };
        runtime.memories.remove(index);
        true
    })
}

pub(crate) fn deactivate_current_address_space() -> Result<(), SysError> {
    #[cfg(target_arch = "aarch64")]
    if !crate::kernel_lowlevel::mmu::activate_bootstrap_on_current_cpu() {
        return Err(SysError::EIO);
    }
    Ok(())
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
        let address_space = Aarch64AddressSpace::new_for_fork(fork_table_allocation_failure)
            .map_err(map_address_error)?;
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
            let source = mapping.source.try_clone_for_fork()?;
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
                let mut page_ops = LinuxProcessForkPageOps { memory: &mut child };
                super::linux_process::clone_linux_fork_pages(
                    &mut page_ops,
                    &mapping.pages,
                    super::linux_process::fork_failpoint,
                )?
            };
            let map_result = super::linux_process::map_linux_fork_pages(
                &mut LinuxProcessForkPageOps { memory: &mut child },
                mapping.addr,
                PAGE_SIZE,
                &pages,
                mapping.prot,
                super::linux_process::fork_failpoint,
            );
            if let Err(error) = map_result {
                LinuxProcessMemory::free_backings(&pages);
                return Err(error);
            }
            child.mappings.push(LinuxProcessMapping {
                addr: mapping.addr,
                len: mapping.len,
                prot: mapping.prot,
                flags: mapping.flags,
                pages,
                source,
            });
        }

        if shared_attachments
            .iter()
            .any(|attachment| !attachment.pages.is_empty() || attachment.owns_attachment_reference)
        {
            return Err(SysError::EINVAL);
        }

        let brk_pages = super::linux_process::clone_linux_fork_pages(
            &mut LinuxProcessForkPageOps { memory: &mut child },
            &parent.brk.pages,
            super::linux_process::fork_failpoint,
        )?;
        let brk_start = child.brk.start;
        let map_result = super::linux_process::map_linux_fork_pages(
            &mut LinuxProcessForkPageOps { memory: &mut child },
            brk_start,
            PAGE_SIZE,
            &brk_pages,
            LINUX_PROT_READ | LINUX_PROT_WRITE,
            super::linux_process::fork_failpoint,
        );
        if let Err(error) = map_result {
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

#[cfg(target_arch = "aarch64")]
fn fork_table_allocation_failure(allocation: usize) -> bool {
    let point = if allocation == 0 {
        LinuxForkFailurePoint::ChildRoot
    } else {
        LinuxForkFailurePoint::TablePage
    };
    super::linux_process::fork_failpoint(point)
}

struct LinuxProcessForkPageOps<'a> {
    memory: &'a mut LinuxProcessMemory,
}

impl super::linux_process::LinuxForkPageOps for LinuxProcessForkPageOps<'_> {
    type Page = LinuxPageBacking;
    type Error = SysError;

    fn failure_error(&self) -> Self::Error {
        SysError::ENOMEM
    }

    fn is_private(&self, page: Self::Page) -> bool {
        matches!(page, LinuxPageBacking::Private { .. })
    }

    fn allocate_private(&mut self, _parent: Self::Page) -> Result<Self::Page, Self::Error> {
        PageFrameAllocator::alloc()
            .map(|pfn| LinuxPageBacking::Private { pfn })
            .ok_or(SysError::ENOMEM)
    }

    fn copy_private(&mut self, parent: Self::Page, child: Self::Page) -> Result<(), Self::Error> {
        let LinuxPageBacking::Private { pfn: parent_pfn } = parent else {
            return Err(SysError::EINVAL);
        };
        let LinuxPageBacking::Private { pfn: child_pfn } = child else {
            return Err(SysError::EINVAL);
        };
        let parent_physical =
            PageFrameAllocator::pfn_address(parent_pfn).ok_or(SysError::ENOMEM)?;
        let child_physical = PageFrameAllocator::pfn_address(child_pfn).ok_or(SysError::ENOMEM)?;
        unsafe {
            core::ptr::copy_nonoverlapping(
                parent_physical as *const u8,
                child_physical as *mut u8,
                PAGE_SIZE,
            );
        }
        Ok(())
    }

    fn acquire_shared(&mut self, parent: Self::Page) -> Result<Self::Page, Self::Error> {
        let LinuxPageBacking::Shared {
            object_id,
            page_index,
            ..
        } = parent
        else {
            return Err(SysError::EINVAL);
        };
        acquire_shared_page(object_id, page_index)
            .then_some(parent)
            .ok_or(SysError::ENOMEM)
    }

    fn release_page(&mut self, page: Self::Page) {
        LinuxProcessMemory::free_backings(core::slice::from_ref(&page));
    }

    fn map_page(
        &mut self,
        address: usize,
        page: Self::Page,
        prot: usize,
    ) -> Result<(), Self::Error> {
        self.memory.map_page(address, page.pfn(), prot)
    }

    fn unmap_page(&mut self, address: usize) {
        let _ = self.memory.unmap_page(address);
    }
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
    let (address, detached) =
        with_current(|memory| memory.map(requested, len, prot, flags, source, replace, contents))?;
    release_detached_attachment_references(&detached);
    Ok(address)
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
    let (address, detached) = with_current(|memory| {
        memory.remap(old_address, old_len, new_len, may_move, fixed, dont_unmap)
    })?;
    release_detached_attachment_references(&detached);
    Ok(address)
}

fn release_detached_attachment_references(detached: &[(u32, usize)]) {
    for (object_id, _address) in detached {
        let _ = super::release_shared_memory_attachment_reference(*object_id);
    }
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
        let mut plan = match LinuxMappingMetadataPlan::try_clone_mapping_metadata(memory) {
            Ok(plan) => plan,
            Err(_) => return Ok(false),
        };
        if plan.try_reserve_attachment_slot().is_err() {
            return Ok(false);
        }
        let page_count = memory.mappings[mapping_index].pages.len();
        let mut originals = Vec::new();
        if originals.try_reserve_exact(page_count).is_err() {
            return Ok(false);
        }
        originals.extend_from_slice(&memory.mappings[mapping_index].pages);
        let mut shared = Vec::new();
        if shared.try_reserve_exact(page_count).is_err() {
            return Ok(false);
        }
        let mut acquired = Vec::new();
        if acquired.try_reserve_exact(page_count).is_err() {
            return Ok(false);
        }
        let pages = match memory.try_mapped_pages_overlapping(address, len) {
            Ok(pages) if !pages.is_empty() => pages,
            _ => return Ok(false),
        };
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

        {
            let planned_mapping = &mut plan.mappings[mapping_index];
            planned_mapping.source = LinuxMappingSource::SharedMemory { id: object_id };
            planned_mapping.pages = shared;
        }
        plan.shared_attachments.push(LinuxSharedAttachmentRecord {
            object_id,
            addr: address,
            len,
        });
        if memory.unmap_pages_transactionally(&pages).is_err() {
            SelfContainedSharedRollback::release(&acquired);
            return Ok(false);
        }
        if memory
            .map_unmapped_pages(address, &plan.mappings[mapping_index].pages, pages[0].prot)
            .is_err()
        {
            memory.restore_mapped_pages(&pages);
            SelfContainedSharedRollback::release(&acquired);
            return Ok(false);
        }
        let _ = memory.commit_mapping_metadata(plan);

        for (page_index, backing) in originals.iter().copied().enumerate() {
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
        let attachment_count = memory
            .mappings
            .iter()
            .filter(|mapping| matches!(mapping.source, LinuxMappingSource::SharedMemory { .. }))
            .count();
        let mut attachments = Vec::new();
        attachments
            .try_reserve_exact(attachment_count)
            .map_err(|_| SysError::ENOMEM)?;
        for mapping in &memory.mappings {
            let LinuxMappingSource::SharedMemory { id } = mapping.source else {
                continue;
            };
            let Some(attachment) = memory.shared_attachments.iter().find(|attachment| {
                attachment.object_id == id
                    && linux_shared_attachment_has_mapping(
                        **attachment,
                        &[LinuxSharedMappingRange {
                            object_id: id,
                            addr: mapping.addr,
                            len: mapping.len,
                        }],
                    )
            }) else {
                continue;
            };
            let mut pages = Vec::new();
            pages
                .try_reserve_exact(mapping.pages.len())
                .map_err(|_| SysError::ENOMEM)?;
            pages.extend_from_slice(&mapping.pages);
            attachments.push(LinuxSharedAttachmentClone {
                object_id: id,
                attachment_addr: attachment.addr,
                attachment_len: attachment.len,
                addr: mapping.addr,
                len: mapping.len,
                pages,
                owns_attachment_reference: false,
            });
        }
        Ok(attachments)
    })?;

    let mut acquired = Vec::new();
    acquired
        .try_reserve_exact(attachments.len())
        .map_err(|_| SysError::ENOMEM)?;
    for mut attachment in attachments {
        let mut local_acquired = Vec::new();
        if local_acquired
            .try_reserve_exact(attachment.pages.len())
            .is_err()
        {
            release_shared_attachments(&acquired);
            return Err(SysError::ENOMEM);
        }
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

pub(crate) fn resource_counts() -> LinuxProcessMemoryResourceCounts {
    with_runtime(|runtime| {
        let mut counts = LinuxProcessMemoryResourceCounts::new();
        for memory in &runtime.memories {
            counts.process_pids[counts.process_count] = memory.pid;
            counts.process_count += 1;
            counts.page_table_pages += memory.address_space.table_page_count();
            for page in memory
                .mappings
                .iter()
                .flat_map(|mapping| mapping.pages.iter())
                .chain(memory.brk.pages.iter())
            {
                match page {
                    LinuxPageBacking::Private { .. } => counts.private_pages += 1,
                    LinuxPageBacking::Shared { .. } => counts.shared_pages += 1,
                }
            }
        }
        counts
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
        if !self.range_accessible(address, bytes.len(), true) {
            return Err(SysError::EFAULT);
        }
        self.copy_to_mapped_pages(address, bytes)
    }

    fn copy_to_mapped_pages(&self, address: usize, bytes: &[u8]) -> Result<(), SysError> {
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .copy_to_user(address, bytes)
            .map_err(map_copy_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space.copy_to_user(address, bytes)
    }

    fn copy_from_user(&self, address: usize, out: &mut [u8]) -> Result<(), SysError> {
        if !self.range_accessible(address, out.len(), false) {
            return Err(SysError::EFAULT);
        }
        #[cfg(target_arch = "aarch64")]
        return self
            .address_space
            .copy_from_user(address, out)
            .map_err(map_copy_address_error);
        #[cfg(not(target_arch = "aarch64"))]
        self.address_space.copy_from_user(address, out)
    }

    fn range_accessible(&self, address: usize, len: usize, write: bool) -> bool {
        let Some(brk_len) = self.brk.pages.len().checked_mul(PAGE_SIZE) else {
            return false;
        };
        let access_ranges = self
            .mappings
            .iter()
            .map(|mapping| LinuxMappingAccessRange {
                addr: mapping.addr,
                len: mapping.len,
                prot: mapping.prot,
            })
            .chain(core::iter::once(LinuxMappingAccessRange {
                addr: self.brk.start,
                len: brk_len,
                prot: LINUX_PROT_READ | LINUX_PROT_WRITE,
            }));
        if !linux_mapping_access_range_covered(access_ranges, address, len, write) {
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

    fn copy_mapping_backings(
        pages: &[LinuxPageBacking],
        out: &mut [u8],
    ) -> Result<(), SysError> {
        let capacity = pages
            .len()
            .checked_mul(PAGE_SIZE)
            .ok_or(SysError::ENOMEM)?;
        if out.len() > capacity {
            return Err(SysError::EFAULT);
        }
        let mut copied = 0usize;
        for page in pages {
            if copied == out.len() {
                break;
            }
            let physical =
                PageFrameAllocator::pfn_address(page.pfn()).ok_or(SysError::ENOMEM)?;
            let chunk = core::cmp::min(PAGE_SIZE, out.len() - copied);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    physical as *const u8,
                    out[copied..copied + chunk].as_mut_ptr(),
                    chunk,
                );
            }
            copied += chunk;
        }
        Ok(())
    }

    fn range_is_mapped(&self, address: usize, len: usize) -> bool {
        let Some(end) = address.checked_add(len) else {
            return false;
        };
        if len == 0 {
            return false;
        }
        let mut cursor = address;
        for mapping in &self.mappings {
            let Some(mapping_end) = mapping.addr.checked_add(mapping.len) else {
                continue;
            };
            if mapping.addr > cursor {
                break;
            }
            if mapping.addr <= cursor && mapping_end > cursor {
                cursor = core::cmp::min(mapping_end, end);
                if cursor == end {
                    return true;
                }
            }
        }
        false
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
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(page_count)
            .map_err(|_| SysError::ENOMEM)?;
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

    fn try_mapped_pages_overlapping(
        &self,
        address: usize,
        len: usize,
    ) -> Result<Vec<LinuxMappedPage>, SysError> {
        let end = address.checked_add(len).ok_or(SysError::EINVAL)?;
        let mut page_count = 0usize;
        for mapping in &self.mappings {
            if !ranges_overlap(address, len, mapping.addr, mapping.len) {
                continue;
            }
            let mapping_end = mapping.addr + mapping.len;
            let overlap_start = core::cmp::max(address, mapping.addr);
            let overlap_end = core::cmp::min(end, mapping_end);
            page_count = page_count
                .checked_add((overlap_end - overlap_start) / PAGE_SIZE)
                .ok_or(SysError::ENOMEM)?;
        }
        let mut pages = Vec::new();
        pages
            .try_reserve_exact(page_count)
            .map_err(|_| SysError::ENOMEM)?;
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
        Ok(pages)
    }

    fn try_mapped_pages_from_backings(
        address: usize,
        pages: &[LinuxPageBacking],
        prot: usize,
    ) -> Result<Vec<LinuxMappedPage>, SysError> {
        let mut mapped = Vec::new();
        mapped
            .try_reserve_exact(pages.len())
            .map_err(|_| SysError::ENOMEM)?;
        for (index, backing) in pages.iter().copied().enumerate() {
            mapped.push(LinuxMappedPage {
                address: address
                    .checked_add(index.checked_mul(PAGE_SIZE).ok_or(SysError::ENOMEM)?)
                    .ok_or(SysError::ENOMEM)?,
                backing,
                prot,
            });
        }
        Ok(mapped)
    }

    fn commit_mapping_metadata(
        &mut self,
        plan: LinuxMappingMetadataPlan,
    ) -> (Vec<LinuxPageBacking>, Vec<(u32, usize)>) {
        let LinuxMappingMetadataPlan {
            mappings,
            removed,
            shared_attachments,
            detached,
        } = plan;
        self.mappings = mappings;
        self.shared_attachments = shared_attachments;
        (removed, detached)
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
    ) -> Result<(usize, Vec<(u32, usize)>), SysError> {
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
        let next_addr = if (LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&address) {
            Some(address.checked_add(len).ok_or(SysError::ENOMEM)?)
        } else {
            None
        };
        if replace {
            let pages = if flags & LINUX_MAP_SHARED != 0 {
                self.allocate_shared_mmap_pages(len, contents, &source)?
            } else {
                self.allocate_unmapped_pages(len, contents)?
            };
            let detached = match self
                .replace_mapping_transactionally(address, len, prot, flags, source, pages)
            {
                Ok(detached) => detached,
                Err((error, pages)) => {
                    Self::free_backings(&pages);
                    return Err(error);
                }
            };
            if let Some(next_addr) = next_addr {
                self.next_addr = next_addr;
            }
            return Ok((address, detached));
        }

        let mut plan = LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?;
        plan.try_reserve_mapping_slot()?;
        let pages = if flags & LINUX_MAP_SHARED != 0 {
            self.allocate_shared_mmap_pages(len, contents, &source)?
        } else {
            self.allocate_unmapped_pages(len, contents)?
        };
        plan.insert_mapping(LinuxProcessMapping {
            addr: address,
            len,
            prot,
            flags,
            pages,
            source,
        });
        let mapping_pages = &plan
            .mappings
            .iter()
            .find(|mapping| mapping.addr == address && mapping.len == len)
            .expect("reserved mapping was inserted")
            .pages;
        if let Err(error) = self.map_unmapped_pages(address, mapping_pages, prot) {
            let replacement = plan
                .take_mapping(address, len)
                .expect("failed mapping remains staged");
            Self::free_backings(&replacement.pages);
            return Err(error);
        }
        let (_, detached) = self.commit_mapping_metadata(plan);
        if let Some(next_addr) = next_addr {
            self.next_addr = next_addr;
        }
        Ok((address, detached))
    }

    fn replace_mapping_transactionally(
        &mut self,
        address: usize,
        len: usize,
        prot: usize,
        flags: usize,
        source: LinuxMappingSource,
        pages: Vec<LinuxPageBacking>,
    ) -> Result<Vec<(u32, usize)>, (SysError, Vec<LinuxPageBacking>)> {
        let old_pages = match self.try_mapped_pages_overlapping(address, len) {
            Ok(pages) => pages,
            Err(error) => return Err((error, pages)),
        };
        let mut plan = match LinuxMappingMetadataPlan::try_clone_mapping_metadata(self) {
            Ok(plan) => plan,
            Err(error) => return Err((error, pages)),
        };
        if let Err(error) = plan.try_transform_range(address, len, None) {
            return Err((error, pages));
        }
        if let Err(error) = plan.try_reserve_mapping_slot() {
            return Err((error, pages));
        }
        if let Err(error) = plan.try_prepare_attachment_reconciliation() {
            return Err((error, pages));
        }
        plan.insert_mapping(LinuxProcessMapping {
            addr: address,
            len,
            prot,
            flags,
            pages,
            source,
        });
        plan.reconcile_shared_attachments();
        let mapping_pages = &plan
            .mappings
            .iter()
            .find(|mapping| mapping.addr == address && mapping.len == len)
            .expect("replacement mapping was inserted")
            .pages;

        if let Err(error) = self.unmap_pages_transactionally(&old_pages) {
            let replacement = plan
                .take_mapping(address, len)
                .expect("failed replacement remains staged");
            return Err((error, replacement.pages));
        }
        if let Err(error) = self.map_unmapped_pages(address, mapping_pages, prot) {
            self.restore_mapped_pages(&old_pages);
            let replacement = plan
                .take_mapping(address, len)
                .expect("failed replacement remains staged");
            return Err((error, replacement.pages));
        }

        let (removed, detached) = self.commit_mapping_metadata(plan);
        Self::free_backings(&removed);
        Ok(detached)
    }

    fn protect(&mut self, address: usize, len: usize, prot: usize) -> Result<(), SysError> {
        if !self.range_is_mapped(address, len) {
            return Err(SysError::EINVAL);
        }
        let mut plan = LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?;
        plan.try_transform_range(address, len, Some(prot))?;
        let pages = self.try_mapped_pages_overlapping(address, len)?;
        self.protect_pages_transactionally(&pages, prot)?;
        let _ = self.commit_mapping_metadata(plan);
        Ok(())
    }

    fn unmap(&mut self, address: usize, len: usize) -> Result<Vec<(u32, usize)>, SysError> {
        address.checked_add(len).ok_or(SysError::EINVAL)?;
        let pages = self.try_mapped_pages_overlapping(address, len)?;
        if pages.is_empty() {
            return Err(SysError::EINVAL);
        }
        let mut plan = LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?;
        plan.try_transform_range(address, len, None)?;
        plan.try_prepare_attachment_reconciliation()?;
        plan.reconcile_shared_attachments();
        self.unmap_pages_transactionally(&pages)?;
        let (removed, detached) = self.commit_mapping_metadata(plan);
        Self::free_backings(&removed);
        Ok(detached)
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
            let planned_page_count = self
                .brk
                .pages
                .len()
                .checked_add(new_pages - old_pages)
                .ok_or(SysError::ENOMEM)?;
            let mut planned_pages = Vec::new();
            planned_pages
                .try_reserve_exact(planned_page_count)
                .map_err(|_| SysError::ENOMEM)?;
            planned_pages.extend_from_slice(&self.brk.pages);
            let pages = self.allocate_unmapped_pages(len, &[])?;
            let mapped = match Self::try_mapped_pages_from_backings(
                start,
                &pages,
                LINUX_PROT_READ | LINUX_PROT_WRITE,
            ) {
                Ok(mapped) => mapped,
                Err(error) => {
                    Self::free_backings(&pages);
                    return Err(error);
                }
            };
            planned_pages.extend_from_slice(&pages);
            if let Err(error) =
                self.map_unmapped_pages(start, &pages, LINUX_PROT_READ | LINUX_PROT_WRITE)
            {
                Self::free_backings(&pages);
                return Err(error);
            }
            if let Err(error) = self.zero_user_range(old_brk, new_brk - old_brk) {
                let _ = self.unmap_pages_transactionally(&mapped);
                Self::free_backings(&pages);
                return Err(error);
            }
            self.brk.pages = planned_pages;
        } else if new_pages < old_pages {
            let mut planned_pages = Vec::new();
            planned_pages
                .try_reserve_exact(new_pages)
                .map_err(|_| SysError::ENOMEM)?;
            planned_pages.extend_from_slice(&self.brk.pages[..new_pages]);
            let mut backings = Vec::new();
            backings
                .try_reserve_exact(old_pages - new_pages)
                .map_err(|_| SysError::ENOMEM)?;
            backings.extend_from_slice(&self.brk.pages[new_pages..old_pages]);
            let removed = Self::try_mapped_pages_from_backings(
                self.brk.start + new_pages * PAGE_SIZE,
                &backings,
                LINUX_PROT_READ | LINUX_PROT_WRITE,
            )?;
            self.unmap_pages_transactionally(&removed)?;
            self.brk.pages = planned_pages;
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
            self.copy_to_mapped_pages(address + offset, &zeros[..chunk])?;
            offset += chunk;
        }
        Ok(())
    }

    fn rollback_remap_destination(
        &mut self,
        mapped: &[LinuxMappedPage],
        replaced: &[LinuxMappedPage],
    ) {
        let _ = self.unmap_pages_transactionally(mapped);
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
    ) -> Result<(usize, Vec<(u32, usize)>), SysError> {
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
            return Ok((old_address, Vec::new()));
        }
        if dont_unmap && old_len != new_len {
            return Err(SysError::EINVAL);
        }
        if fixed.is_none() && !dont_unmap && new_len < old_len {
            let detached = self.unmap(old_address + new_len, old_len - new_len)?;
            return Ok((old_address, detached));
        }
        let grow_start = old_address + old_len;
        let extra_len = new_len.saturating_sub(old_len);
        if fixed.is_none()
            && !dont_unmap
            && new_len > old_len
            && self.range_available(grow_start, extra_len)
        {
            let prot = self.mappings[index].prot;
            let planned_page_count = self.mappings[index]
                .pages
                .len()
                .checked_add(extra_len / PAGE_SIZE)
                .ok_or(SysError::ENOMEM)?;
            let mut planned_pages = Vec::new();
            planned_pages
                .try_reserve_exact(planned_page_count)
                .map_err(|_| SysError::ENOMEM)?;
            planned_pages.extend_from_slice(&self.mappings[index].pages);
            let pages = self.allocate_unmapped_pages(extra_len, &[])?;
            planned_pages.extend_from_slice(&pages);
            if let Err(error) = self.map_unmapped_pages(grow_start, &pages, prot) {
                Self::free_backings(&pages);
                return Err(error);
            }
            self.mappings[index].len = new_len;
            self.mappings[index].pages = planned_pages;
            return Ok((old_address, Vec::new()));
        }
        if !may_move {
            return Err(SysError::ENOMEM);
        }
        let prot = self.mappings[index].prot;
        let flags = self.mappings[index].flags;
        let source = self.mappings[index].source.try_clone_for_fork()?;
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
        let next_addr = if (LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&new_address) {
            Some(new_address.checked_add(new_len).ok_or(SysError::ENOMEM)?)
        } else {
            None
        };
        let replaced = self.try_mapped_pages_overlapping(new_address, new_len)?;
        let source_pages = if dont_unmap {
            Vec::new()
        } else {
            self.try_mapped_pages_overlapping(old_address, old_len)?
        };
        let mut plan = LinuxMappingMetadataPlan::try_clone_mapping_metadata(self)?;
        plan.try_transform_range(new_address, new_len, None)?;
        if !dont_unmap {
            plan.try_transform_range(old_address, old_len, None)?;
        }
        plan.try_reserve_mapping_slot()?;
        plan.try_prepare_attachment_reconciliation()?;

        let copy_len = core::cmp::min(old_len, new_len);
        let mut contents = Vec::new();
        contents
            .try_reserve_exact(copy_len)
            .map_err(|_| SysError::ENOMEM)?;
        contents.resize(copy_len, 0);
        Self::copy_mapping_backings(&self.mappings[index].pages, &mut contents)?;
        let pages = self.allocate_unmapped_pages(new_len, &contents)?;
        let mapped = match Self::try_mapped_pages_from_backings(new_address, &pages, prot) {
            Ok(mapped) => mapped,
            Err(error) => {
                Self::free_backings(&pages);
                return Err(error);
            }
        };
        plan.insert_mapping(LinuxProcessMapping {
            addr: new_address,
            len: new_len,
            prot,
            flags,
            pages,
            source,
        });
        plan.reconcile_shared_attachments();
        let mapping_pages = &plan
            .mappings
            .iter()
            .find(|mapping| mapping.addr == new_address && mapping.len == new_len)
            .expect("remap destination was inserted")
            .pages;
        if let Err(error) = self.unmap_pages_transactionally(&replaced) {
            let replacement = plan
                .take_mapping(new_address, new_len)
                .expect("failed remap destination remains staged");
            Self::free_backings(&replacement.pages);
            return Err(error);
        }
        if let Err(error) = self.map_unmapped_pages(new_address, mapping_pages, prot) {
            self.restore_mapped_pages(&replaced);
            let replacement = plan
                .take_mapping(new_address, new_len)
                .expect("failed remap destination remains staged");
            Self::free_backings(&replacement.pages);
            return Err(error);
        }
        if !dont_unmap {
            if let Err(error) = self.unmap_pages_transactionally(&source_pages) {
                self.rollback_remap_destination(&mapped, &replaced);
                let replacement = plan
                    .take_mapping(new_address, new_len)
                    .expect("failed remap destination remains staged");
                Self::free_backings(&replacement.pages);
                return Err(error);
            }
        }

        let (removed, detached) = self.commit_mapping_metadata(plan);
        Self::free_backings(&removed);
        if let Some(next_addr) = next_addr {
            self.next_addr = next_addr;
        }
        Ok((new_address, detached))
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

#[cfg(target_arch = "aarch64")]
fn map_copy_address_error(error: crate::kernel_lowlevel::mmu::AddressSpaceError) -> SysError {
    use crate::kernel_lowlevel::mmu::AddressSpaceError;
    let error = match error {
        AddressSpaceError::OutOfMemory => LinuxAddressSpaceErrorKind::OutOfMemory,
        AddressSpaceError::InvalidAddress => LinuxAddressSpaceErrorKind::InvalidAddress,
        AddressSpaceError::InvalidPermissions => LinuxAddressSpaceErrorKind::InvalidPermissions,
        AddressSpaceError::AlreadyMapped => LinuxAddressSpaceErrorKind::AlreadyMapped,
        AddressSpaceError::NotMapped => LinuxAddressSpaceErrorKind::NotMapped,
        AddressSpaceError::PermissionDenied => LinuxAddressSpaceErrorKind::PermissionDenied,
    };
    match linux_copy_address_error_class(error) {
        LinuxCopyAddressErrorClass::Fault => SysError::EFAULT,
        LinuxCopyAddressErrorClass::OutOfMemory => SysError::ENOMEM,
    }
}
