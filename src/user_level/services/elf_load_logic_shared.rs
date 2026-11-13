// ELF load planning that does not walk a segment page by page.
//
// A PT_LOAD with `p_filesz == 0` and a huge `p_memsz` used to pass the
// overflow-only vaddr check and then spin from the lowest to the highest
// page. Admission is decided from the segment bounds before any page loop,
// and BSS is one run covering the whole segment.

use alloc::vec::Vec;

pub const ELF_LOAD_EINVAL: u8 = 22;
pub const ELF_LOAD_ENOMEM: u8 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfLoadSegment {
    pub vaddr: u64,
    pub mem_size: u64,
    pub file_size: u64,
    pub prot: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElfLoadRun {
    pub start: usize,
    pub len: usize,
    pub prot: usize,
}

fn page_down(value: u64, page_size: u64) -> Option<u64> {
    if page_size == 0 {
        return None;
    }
    Some(value - value % page_size)
}

fn page_up(value: u64, page_size: u64) -> Option<u64> {
    if page_size == 0 || value == 0 {
        return if page_size == 0 { None } else { Some(0) };
    }
    let adjusted = value.checked_add(page_size - 1)?;
    Some(adjusted - adjusted % page_size)
}

fn segment_pages(segment: &ElfLoadSegment, page_size: u64) -> Option<(u64, u64)> {
    if segment.mem_size == 0 {
        return None;
    }
    let end = segment.vaddr.checked_add(segment.mem_size)?;
    let start = page_down(segment.vaddr, page_size)?;
    let aligned_end = page_up(end, page_size)?;
    if aligned_end <= start {
        return None;
    }
    Some((start, aligned_end))
}

/// Reject a segment that cannot be placed in the EL0 window, or whose
/// `mem_size` is larger than that window.
pub fn admit_elf_segment(
    segment: &ElfLoadSegment,
    base: u64,
    window_start: u64,
    window_end: u64,
    page_size: u64,
) -> Result<(), u8> {
    if page_size == 0 || window_end <= window_start {
        return Err(ELF_LOAD_EINVAL);
    }
    if segment.mem_size == 0 {
        return Ok(());
    }
    if segment.file_size > segment.mem_size {
        return Err(ELF_LOAD_EINVAL);
    }
    let window = window_end - window_start;
    if segment.mem_size > window {
        return Err(ELF_LOAD_ENOMEM);
    }
    let abs_start = base.checked_add(segment.vaddr).ok_or(ELF_LOAD_EINVAL)?;
    let abs_end = abs_start
        .checked_add(segment.mem_size)
        .ok_or(ELF_LOAD_EINVAL)?;
    if abs_start < window_start || abs_end > window_end {
        return Err(ELF_LOAD_EINVAL);
    }
    let (start, end) = segment_pages(segment, page_size).ok_or(ELF_LOAD_EINVAL)?;
    let aligned_lo = base.checked_add(start).ok_or(ELF_LOAD_EINVAL)?;
    let aligned_hi = base.checked_add(end).ok_or(ELF_LOAD_EINVAL)?;
    if aligned_lo < window_start || aligned_hi > window_end {
        return Err(ELF_LOAD_EINVAL);
    }
    Ok(())
}

fn page_protection(page: u64, page_size: u64, segments: &[ElfLoadSegment]) -> Option<usize> {
    let page_end = page.checked_add(page_size)?;
    let mut protection = None;
    for segment in segments {
        if segment.mem_size == 0 {
            continue;
        }
        let segment_end = segment.vaddr.checked_add(segment.mem_size)?;
        if segment.vaddr < page_end && page < segment_end {
            protection = Some(protection.unwrap_or(0) | segment.prot);
        }
    }
    let mut protection = protection?;
    if protection & 2 != 0 && protection & 4 != 0 {
        protection = (protection | 1 | 2) & !4;
    }
    Some(protection)
}

/// Plan anonymous mapping runs for `segments`.
///
/// The result has one entry per constant-protection interval, including a
/// filesz=0 BSS segment as a single run. It does not visit each page.
pub fn plan_elf_load(
    segments: &[ElfLoadSegment],
    base: u64,
    window_start: u64,
    window_end: u64,
    page_size: u64,
) -> Result<Vec<ElfLoadRun>, u8> {
    if page_size == 0 || window_end <= window_start {
        return Err(ELF_LOAD_EINVAL);
    }
    let mut bounds = Vec::new();
    let mut live = 0usize;
    for segment in segments {
        admit_elf_segment(segment, base, window_start, window_end, page_size)?;
        if segment.mem_size == 0 {
            continue;
        }
        let (start, end) = segment_pages(segment, page_size).ok_or(ELF_LOAD_EINVAL)?;
        bounds
            .try_reserve(2)
            .map_err(|_| ELF_LOAD_ENOMEM)?;
        bounds.push(start);
        bounds.push(end);
        live += 1;
    }
    if live == 0 {
        return Err(ELF_LOAD_EINVAL);
    }
    bounds.sort_unstable();
    bounds.dedup();
    let min_start = *bounds.first().ok_or(ELF_LOAD_EINVAL)?;
    let max_end = *bounds.last().ok_or(ELF_LOAD_EINVAL)?;
    let span = max_end.checked_sub(min_start).ok_or(ELF_LOAD_EINVAL)?;
    if span > window_end - window_start {
        return Err(ELF_LOAD_ENOMEM);
    }
    let abs_lo = base.checked_add(min_start).ok_or(ELF_LOAD_EINVAL)?;
    let abs_hi = base.checked_add(max_end).ok_or(ELF_LOAD_EINVAL)?;
    if abs_lo < window_start || abs_hi > window_end {
        return Err(ELF_LOAD_EINVAL);
    }

    let mut runs: Vec<ElfLoadRun> = Vec::new();
    for pair in bounds.windows(2) {
        let start = pair[0];
        let end = pair[1];
        if end <= start {
            continue;
        }
        let Some(prot) = page_protection(start, page_size, segments) else {
            continue;
        };
        let start_usize = usize::try_from(start).map_err(|_| ELF_LOAD_EINVAL)?;
        let len = usize::try_from(end - start).map_err(|_| ELF_LOAD_EINVAL)?;
        if let Some(previous) = runs.last_mut() {
            if previous.prot == prot && previous.start.saturating_add(previous.len) == start_usize {
                previous.len = previous.len.checked_add(len).ok_or(ELF_LOAD_ENOMEM)?;
                continue;
            }
        }
        runs.try_reserve(1).map_err(|_| ELF_LOAD_ENOMEM)?;
        runs.push(ElfLoadRun {
            start: start_usize,
            len,
            prot,
        });
    }
    if runs.is_empty() {
        return Err(ELF_LOAD_EINVAL);
    }
    Ok(runs)
}
