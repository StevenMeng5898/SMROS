pub(crate) const LINUX_PAGE_SIZE: usize = 0x1000;
pub(crate) const LINUX_USER_BASE: usize = 0x1000_0000;
pub(crate) const LINUX_USER_LIMIT: usize = 0x2000_0000;
pub(crate) const LINUX_MAIN_BASE: usize = 0x1000_0000;
pub(crate) const LINUX_INTERPRETER_BASE: usize = 0x1100_0000;
pub(crate) const LINUX_MMAP_BASE: usize = 0x1200_0000;
pub(crate) const LINUX_BRK_BASE: usize = 0x1d00_0000;
pub(crate) const LINUX_BRK_LIMIT: usize = LINUX_BRK_BASE + 0x10_0000;
pub(crate) const LINUX_STACK_TOP: usize = 0x1fff_f000;

pub(crate) const LINUX_PROT_READ: usize = 1;
pub(crate) const LINUX_PROT_WRITE: usize = 2;
pub(crate) const LINUX_PROT_EXEC: usize = 4;
pub(crate) const LINUX_MAP_SHARED: usize = 1;
pub(crate) const LINUX_MAP_PRIVATE: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessAttributesCore {
    pub namespace_flags: usize,
    pub setns_count: usize,
    pub mount_count: usize,
    pub mount_flags: usize,
    pub pivot_rooted: bool,
    pub chrooted: bool,
    pub no_new_privs: bool,
    pub seccomp_mode: usize,
    pub seccomp_filters: usize,
    pub cap_effective: u64,
    pub cap_permitted: u64,
    pub cap_inheritable: u64,
    pub hostname_set: bool,
    pub domainname_set: bool,
}

impl LinuxProcessAttributesCore {
    pub(crate) const fn fork_child(self, namespace_flags: usize) -> Self {
        Self {
            namespace_flags: self.namespace_flags | namespace_flags,
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxForkAcquisition {
    SchedulerThread,
    Task,
    Process,
    Resources,
    Memory,
    Configured,
}

#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxForkFailurePoint {
    SchedulerThread,
    Task,
    Process,
    ChildRoot,
    TablePage,
    DescriptorReference,
    SharedReference,
    PrivatePage,
    Memory,
    Configured,
    ProcessPublication,
    TaskPublication,
    SchedulerPublication,
}

impl LinuxForkFailurePoint {
    pub(crate) const COUNT: usize = Self::SchedulerPublication as usize + 1;
}

pub(crate) struct LinuxForkFailureSchedule {
    point: LinuxForkFailurePoint,
    remaining: usize,
    active: bool,
}

impl LinuxForkFailureSchedule {
    pub(crate) const fn new(point: LinuxForkFailurePoint, occurrence: usize) -> Self {
        Self {
            point,
            remaining: occurrence,
            active: true,
        }
    }

    pub(crate) fn should_fail(&mut self, point: LinuxForkFailurePoint) -> bool {
        if !self.active || point != self.point {
            return false;
        }
        if self.remaining != 0 {
            self.remaining -= 1;
            return false;
        }
        self.active = false;
        true
    }
}

impl LinuxForkAcquisition {
    const ORDER: [Self; 6] = [
        Self::SchedulerThread,
        Self::Task,
        Self::Process,
        Self::Resources,
        Self::Memory,
        Self::Configured,
    ];
}

pub(crate) struct LinuxForkAcquisitionLedger {
    acquired: [Option<LinuxForkAcquisition>; 6],
    len: usize,
}

impl LinuxForkAcquisitionLedger {
    pub(crate) const fn new() -> Self {
        Self {
            acquired: [None; 6],
            len: 0,
        }
    }

    pub(crate) fn acquire(&mut self, stage: LinuxForkAcquisition) -> bool {
        if LinuxForkAcquisition::ORDER.get(self.len).copied() != Some(stage) {
            return false;
        }
        self.acquired[self.len] = Some(stage);
        self.len += 1;
        true
    }

    pub(crate) fn rollback_into(&mut self, out: &mut [Option<LinuxForkAcquisition>]) -> usize {
        let mut written = 0usize;
        while self.len != 0 && written < out.len() {
            self.len -= 1;
            out[written] = self.acquired[self.len].take();
            written += 1;
        }
        written
    }

    pub(crate) fn release(&mut self, stage: LinuxForkAcquisition) -> bool {
        if self.len == 0 || self.acquired[self.len - 1] != Some(stage) {
            return false;
        }
        self.len -= 1;
        self.acquired[self.len] = None;
        true
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub(crate) const fn linux_mmap_backing_is_shared(flags: usize) -> bool {
    flags & LINUX_MAP_SHARED != 0
}

pub(crate) const fn linux_shared_page_index(first_page: usize, index: usize) -> Option<usize> {
    first_page.checked_add(index)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinuxPageBacking {
    Private {
        pfn: u64,
    },
    Shared {
        object_id: u32,
        page_index: usize,
        pfn: u64,
    },
}

pub(crate) fn linux_clone_page_backing(
    backing: LinuxPageBacking,
    private_pfn: u64,
) -> LinuxPageBacking {
    match backing {
        LinuxPageBacking::Private { .. } => LinuxPageBacking::Private { pfn: private_pfn },
        shared @ LinuxPageBacking::Shared { .. } => shared,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSharedAttachmentRecord {
    pub object_id: u32,
    pub addr: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSharedMappingRange {
    pub object_id: u32,
    pub addr: usize,
    pub len: usize,
}

pub(crate) fn linux_shared_attachment_has_mapping(
    attachment: LinuxSharedAttachmentRecord,
    mappings: &[LinuxSharedMappingRange],
) -> bool {
    let Some(attachment_end) = attachment.addr.checked_add(attachment.len) else {
        return false;
    };
    mappings.iter().any(|mapping| {
        if mapping.object_id != attachment.object_id {
            return false;
        }
        mapping
            .addr
            .checked_add(mapping.len)
            .is_some_and(|mapping_end| {
                attachment.addr < mapping_end && mapping.addr < attachment_end
            })
    })
}

pub(crate) const fn linux_shared_mremap_supported(requires_move: bool) -> bool {
    !requires_move
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxDescriptorEntry {
    pub fd: usize,
    pub description_id: u32,
    pub close_on_exec: bool,
}

impl LinuxDescriptorEntry {
    const EMPTY: Self = Self {
        fd: 0,
        description_id: 0,
        close_on_exec: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxOpenDescription {
    pub id: u32,
    pub handle: u32,
    pub object_type: ObjectType,
    pub status_flags: usize,
    pub offset: usize,
    pub references: usize,
}

pub(crate) struct LinuxOpenDescriptionTableCore<const N: usize> {
    descriptions: [Option<LinuxOpenDescription>; N],
    next_id: u32,
}

impl<const N: usize> LinuxOpenDescriptionTableCore<N> {
    pub(crate) const fn new() -> Self {
        Self {
            descriptions: [None; N],
            next_id: 1,
        }
    }

    pub(crate) fn insert(
        &mut self,
        handle: u32,
        object_type: ObjectType,
        status_flags: usize,
        offset: usize,
    ) -> Option<u32> {
        let slot = self.descriptions.iter().position(Option::is_none)?;
        let id = self.next_id;
        if id == 0 {
            return None;
        }
        self.next_id = id.checked_add(1).unwrap_or(0);
        self.descriptions[slot] = Some(LinuxOpenDescription {
            id,
            handle,
            object_type,
            status_flags,
            offset,
            references: 0,
        });
        Some(id)
    }

    pub(crate) fn insert_object(&mut self, handle: u32, object_type: ObjectType) -> Option<u32> {
        self.insert(handle, object_type, 0, 0)
    }

    pub(crate) fn get(&self, id: u32) -> Option<&LinuxOpenDescription> {
        self.descriptions
            .iter()
            .flatten()
            .find(|description| description.id == id)
    }

    pub(crate) fn set_offset(&mut self, id: u32, offset: usize) -> bool {
        let Some(description) = self
            .descriptions
            .iter_mut()
            .flatten()
            .find(|description| description.id == id)
        else {
            return false;
        };
        description.offset = offset;
        true
    }

    pub(crate) fn acquire(&mut self, id: u32) -> bool {
        let Some(description) = self
            .descriptions
            .iter_mut()
            .flatten()
            .find(|description| description.id == id)
        else {
            return false;
        };
        let Some(references) = description.references.checked_add(1) else {
            return false;
        };
        description.references = references;
        true
    }

    pub(crate) fn release(&mut self, id: u32) -> Option<u32> {
        let slot = self
            .descriptions
            .iter()
            .position(|description| description.is_some_and(|description| description.id == id))?;
        let description = self.descriptions[slot].as_mut()?;
        description.references = description.references.checked_sub(1)?;
        if description.references != 0 {
            return None;
        }
        self.descriptions[slot]
            .take()
            .map(|description| description.handle)
    }
}

pub(crate) struct LinuxProcessResourceCore<const D: usize, const O: usize> {
    descriptors: [LinuxDescriptorEntry; D],
    descriptor_len: usize,
    objects: [u32; O],
    object_len: usize,
}

impl<const D: usize, const O: usize> LinuxProcessResourceCore<D, O> {
    pub(crate) const fn new() -> Self {
        Self {
            descriptors: [LinuxDescriptorEntry::EMPTY; D],
            descriptor_len: 0,
            objects: [0; O],
            object_len: 0,
        }
    }

    pub(crate) fn insert_descriptor<const N: usize>(
        &mut self,
        fd: usize,
        description_id: u32,
        close_on_exec: bool,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> bool {
        if self.descriptor_len == D || self.descriptor(fd).is_some() {
            return false;
        }
        if !descriptions.acquire(description_id) {
            return false;
        }
        self.descriptors[self.descriptor_len] = LinuxDescriptorEntry {
            fd,
            description_id,
            close_on_exec,
        };
        self.descriptor_len += 1;
        true
    }

    pub(crate) fn insert_object<const N: usize>(
        &mut self,
        description_id: u32,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> bool {
        if self.object_len == O || self.objects().contains(&description_id) {
            return false;
        }
        if !descriptions.acquire(description_id) {
            return false;
        }
        self.objects[self.object_len] = description_id;
        self.object_len += 1;
        true
    }

    pub(crate) fn descriptor(&self, fd: usize) -> Option<LinuxDescriptorEntry> {
        self.descriptors()
            .iter()
            .copied()
            .find(|entry| entry.fd == fd)
    }

    pub(crate) fn descriptors(&self) -> &[LinuxDescriptorEntry] {
        &self.descriptors[..self.descriptor_len]
    }

    pub(crate) fn objects(&self) -> &[u32] {
        &self.objects[..self.object_len]
    }

    pub(crate) fn close_descriptor<const N: usize>(
        &mut self,
        fd: usize,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> Option<u32> {
        let index = self.descriptors[..self.descriptor_len]
            .iter()
            .position(|entry| entry.fd == fd)?;
        let entry = self.descriptors[index];
        self.remove_descriptor_index(index);
        descriptions.release(entry.description_id)
    }

    pub(crate) fn release_object<const N: usize>(
        &mut self,
        description_id: u32,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> Option<u32> {
        let index = self.objects[..self.object_len]
            .iter()
            .position(|object| *object == description_id)?;
        for slot in index..self.object_len.saturating_sub(1) {
            self.objects[slot] = self.objects[slot + 1];
        }
        self.object_len -= 1;
        self.objects[self.object_len] = 0;
        descriptions.release(description_id)
    }

    fn remove_descriptor_index(&mut self, index: usize) {
        for slot in index..self.descriptor_len.saturating_sub(1) {
            self.descriptors[slot] = self.descriptors[slot + 1];
        }
        self.descriptor_len -= 1;
        self.descriptors[self.descriptor_len] = LinuxDescriptorEntry::EMPTY;
    }
}

pub(crate) struct LinuxResourceCloneCore<const D: usize, const O: usize> {
    descriptors: [LinuxDescriptorEntry; D],
    descriptor_len: usize,
    objects: [u32; O],
    object_len: usize,
}

impl<const D: usize, const O: usize> LinuxResourceCloneCore<D, O> {
    pub(crate) fn reserve<const N: usize>(
        parent: &LinuxProcessResourceCore<D, O>,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> Option<Self> {
        Self::reserve_with_failure(parent, descriptions, || false)
    }

    pub(crate) fn reserve_with_failure<const N: usize>(
        parent: &LinuxProcessResourceCore<D, O>,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
        mut fail_after_acquire: impl FnMut() -> bool,
    ) -> Option<Self> {
        let mut clone = Self {
            descriptors: [LinuxDescriptorEntry::EMPTY; D],
            descriptor_len: 0,
            objects: [0; O],
            object_len: 0,
        };
        for entry in parent.descriptors() {
            if !descriptions.acquire(entry.description_id) {
                let _ = clone.rollback(descriptions);
                return None;
            }
            clone.descriptors[clone.descriptor_len] = *entry;
            clone.descriptor_len += 1;
            if fail_after_acquire() {
                let _ = clone.rollback(descriptions);
                return None;
            }
        }
        for description_id in parent.objects() {
            if !descriptions.acquire(*description_id) {
                let _ = clone.rollback(descriptions);
                return None;
            }
            clone.objects[clone.object_len] = *description_id;
            clone.object_len += 1;
            if fail_after_acquire() {
                let _ = clone.rollback(descriptions);
                return None;
            }
        }
        Some(clone)
    }

    pub(crate) fn descriptors(&self) -> &[LinuxDescriptorEntry] {
        &self.descriptors[..self.descriptor_len]
    }

    pub(crate) fn objects(&self) -> &[u32] {
        &self.objects[..self.object_len]
    }

    pub(crate) fn commit(self, child: &mut LinuxProcessResourceCore<D, O>) -> bool {
        if child.descriptor_len != 0 || child.object_len != 0 {
            return false;
        }
        child.descriptors = self.descriptors;
        child.descriptor_len = self.descriptor_len;
        child.objects = self.objects;
        child.object_len = self.object_len;
        true
    }

    pub(crate) fn rollback<const N: usize>(
        self,
        descriptions: &mut LinuxOpenDescriptionTableCore<N>,
    ) -> [Option<u32>; N] {
        let mut released = [None; N];
        let mut released_len = 0usize;
        for entry in self.descriptors() {
            if let Some(handle) = descriptions.release(entry.description_id) {
                if released_len < N {
                    released[released_len] = Some(handle);
                    released_len += 1;
                }
            }
        }
        for description_id in self.objects() {
            if let Some(handle) = descriptions.release(*description_id) {
                if released_len < N {
                    released[released_len] = Some(handle);
                    released_len += 1;
                }
            }
        }
        released
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxSharedPageRecord {
    pub object_id: u32,
    pub page_index: usize,
    pub pfn: u64,
    pub references: usize,
    pub named: bool,
}

pub(crate) struct LinuxSharedPageTableCore<const N: usize> {
    pages: [Option<LinuxSharedPageRecord>; N],
}

impl<const N: usize> LinuxSharedPageTableCore<N> {
    pub(crate) const fn new() -> Self {
        Self { pages: [None; N] }
    }

    pub(crate) fn insert(&mut self, object_id: u32, page_index: usize, pfn: u64) -> bool {
        if object_id == 0 || pfn == 0 || self.get_any(object_id, page_index).is_some() {
            return false;
        }
        let Some(slot) = self.pages.iter().position(Option::is_none) else {
            return false;
        };
        self.pages[slot] = Some(LinuxSharedPageRecord {
            object_id,
            page_index,
            pfn,
            references: 1,
            named: true,
        });
        true
    }

    pub(crate) fn acquire_or_insert(
        &mut self,
        object_id: u32,
        page_index: usize,
        candidate_pfn: u64,
    ) -> Option<u64> {
        if let Some(record) = self
            .pages
            .iter_mut()
            .flatten()
            .find(|record| record.object_id == object_id && record.page_index == page_index)
        {
            record.references = linux_shared_reference_acquire(record.references)?;
            return Some(record.pfn);
        }
        self.insert(object_id, page_index, candidate_pfn)
            .then_some(candidate_pfn)
    }

    pub(crate) fn get(&self, object_id: u32, page_index: usize) -> Option<LinuxSharedPageRecord> {
        self.get_any(object_id, page_index)
            .filter(|record| record.named)
    }

    pub(crate) fn get_any(
        &self,
        object_id: u32,
        page_index: usize,
    ) -> Option<LinuxSharedPageRecord> {
        self.pages
            .iter()
            .flatten()
            .copied()
            .find(|record| record.object_id == object_id && record.page_index == page_index)
    }

    pub(crate) fn acquire(&mut self, object_id: u32, page_index: usize) -> bool {
        let Some(record) = self
            .pages
            .iter_mut()
            .flatten()
            .find(|record| record.object_id == object_id && record.page_index == page_index)
        else {
            return false;
        };
        let Some(references) = linux_shared_reference_acquire(record.references) else {
            return false;
        };
        record.references = references;
        true
    }

    pub(crate) fn remove_name(&mut self, object_id: u32) -> bool {
        let mut removed = false;
        for record in self.pages.iter_mut().flatten() {
            if record.object_id == object_id && record.named {
                record.named = false;
                removed = true;
            }
        }
        removed
    }

    pub(crate) fn release(&mut self, object_id: u32, page_index: usize) -> Option<u64> {
        let slot = self.pages.iter().position(|record| {
            record.is_some_and(|record| {
                record.object_id == object_id && record.page_index == page_index
            })
        })?;
        let record = self.pages[slot].as_mut()?;
        record.references = linux_shared_reference_release(record.references)?;
        if record.references != 0 {
            return None;
        }
        self.pages[slot].take().map(|record| record.pfn)
    }
}

impl LinuxPageBacking {
    pub(crate) fn pfn(self) -> u64 {
        match self {
            Self::Private { pfn } | Self::Shared { pfn, .. } => pfn,
        }
    }

    pub(crate) fn is_shared(self) -> bool {
        matches!(self, Self::Shared { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxProcessMappingCore {
    pub owner_pid: usize,
    pub addr: usize,
    pub len: usize,
    pub prot: usize,
    pub flags: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMappingRange {
    pub addr: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxBrkCore {
    pub start: usize,
    pub current: usize,
    pub limit: usize,
}

pub(crate) struct LinuxProcessMemoryCore<const N: usize> {
    pub pid: usize,
    pub root_paddr: u64,
    pub initial_stack: Option<(usize, usize)>,
    pub next_addr: usize,
    pub brk: LinuxBrkCore,
    mappings: [Option<LinuxProcessMappingCore>; N],
    mappings_len: usize,
}

impl<const N: usize> LinuxProcessMemoryCore<N> {
    pub(crate) fn new(pid: usize, root_paddr: u64) -> Option<Self> {
        if pid == 0 || root_paddr == 0 || root_paddr as usize % LINUX_PAGE_SIZE != 0 {
            return None;
        }
        Some(Self {
            pid,
            root_paddr,
            initial_stack: None,
            next_addr: LINUX_MMAP_BASE,
            brk: LinuxBrkCore {
                start: LINUX_BRK_BASE,
                current: LINUX_BRK_BASE,
                limit: LINUX_BRK_LIMIT,
            },
            mappings: [None; N],
            mappings_len: 0,
        })
    }

    pub(crate) fn mapping_count(&self) -> usize {
        self.mappings_len
    }

    pub(crate) fn push_mapping(&mut self, mapping: LinuxProcessMappingCore) -> bool {
        if mapping.owner_pid != self.pid
            || self.mappings_len >= N
            || !linux_user_page_range_valid(mapping.addr, mapping.len)
        {
            return false;
        }
        self.mappings[self.mappings_len] = Some(mapping);
        self.mappings_len += 1;
        true
    }

    pub(crate) fn set_initial_stack(&mut self, address: usize, len: usize) -> bool {
        if self.initial_stack.is_some() || !linux_user_page_range_valid(address, len) {
            return false;
        }
        self.initial_stack = Some((address, len));
        true
    }

    pub(crate) fn set_next_addr(&mut self, address: usize) -> bool {
        if address % LINUX_PAGE_SIZE != 0 || !(LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&address) {
            return false;
        }
        self.next_addr = address;
        true
    }

    pub(crate) fn set_brk(&mut self, start: usize, current: usize, limit: usize) -> bool {
        if start % LINUX_PAGE_SIZE != 0
            || limit % LINUX_PAGE_SIZE != 0
            || start < LINUX_USER_BASE
            || start > current
            || current > limit
            || limit > LINUX_USER_LIMIT
        {
            return false;
        }
        self.brk = LinuxBrkCore {
            start,
            current,
            limit,
        };
        true
    }
}

pub(crate) fn linux_user_page_range_valid(address: usize, len: usize) -> bool {
    address % LINUX_PAGE_SIZE == 0
        && len != 0
        && len % LINUX_PAGE_SIZE == 0
        && address >= LINUX_USER_BASE
        && address
            .checked_add(len)
            .map(|end| end <= LINUX_USER_LIMIT)
            .unwrap_or(false)
}

pub(crate) fn linux_shared_reference_acquire(references: usize) -> Option<usize> {
    (references != 0)
        .then(|| references.checked_add(1))
        .flatten()
}

pub(crate) fn linux_shared_reference_release(references: usize) -> Option<usize> {
    references.checked_sub(1)
}

pub(crate) fn linux_user_copy_chunk(
    address: usize,
    remaining: usize,
    page_size: usize,
) -> Option<usize> {
    if remaining == 0 || page_size == 0 {
        return None;
    }
    let in_page = address % page_size;
    Some(core::cmp::min(remaining, page_size - in_page))
}

pub(crate) fn linux_mapping_allows(prot: usize, write: bool) -> bool {
    if write {
        prot & LINUX_PROT_WRITE != 0
    } else {
        prot & (LINUX_PROT_READ | LINUX_PROT_WRITE) != 0
    }
}

pub(crate) fn linux_mapping_range_covered(
    mappings: &[LinuxMappingRange],
    address: usize,
    len: usize,
) -> bool {
    let Some(end) = address.checked_add(len) else {
        return false;
    };
    if len == 0 {
        return false;
    }

    let mut cursor = address;
    while cursor < end {
        let mut covered_until = cursor;
        for mapping in mappings {
            let Some(mapping_end) = mapping.addr.checked_add(mapping.len) else {
                continue;
            };
            if mapping.addr <= cursor && mapping_end > covered_until {
                covered_until = mapping_end;
            }
        }
        if covered_until == cursor {
            return false;
        }
        cursor = core::cmp::min(covered_until, end);
    }
    true
}

pub(crate) fn linux_process_memory_remove_index(pids: &[usize], pid: usize) -> Option<usize> {
    pids.iter().position(|candidate| *candidate == pid)
}

pub(crate) fn linux_mremap_requires_move(
    old_address: usize,
    old_len: usize,
    new_len: usize,
    fixed: Option<usize>,
    dont_unmap: bool,
) -> bool {
    dont_unmap || old_len != new_len || fixed.is_some_and(|new_address| new_address != old_address)
}
