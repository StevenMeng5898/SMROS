include!("main_logic_shared.rs");

pub(crate) const KERNEL_HEAP_SIZE: usize = 0x0400_0000;

pub(crate) fn bump_alloc_next(
    pos: usize,
    size: usize,
    align: usize,
    heap_size: usize,
) -> Option<(usize, usize)> {
    smros_main_bump_alloc_next_body!(pos, size, align, heap_size)
}
