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
        if address % LINUX_PAGE_SIZE != 0
            || !(LINUX_MMAP_BASE..LINUX_BRK_BASE).contains(&address)
        {
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
    (references != 0).then(|| references.checked_add(1)).flatten()
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
