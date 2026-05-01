macro_rules! smros_main_align_up_body {
    ($pos:expr, $align:expr) => {{
        if $align == 0 {
            None
        } else {
            let offset = $pos % $align;
            if offset == 0 {
                Some($pos)
            } else {
                $pos.checked_add($align - offset)
            }
        }
    }};
}

macro_rules! smros_main_bump_alloc_next_body {
    ($pos:expr, $size:expr, $align:expr, $heap_size:expr) => {{
        match smros_main_align_up_body!($pos, $align) {
            Some(aligned_pos) => match aligned_pos.checked_add($size) {
                Some(next_pos) if next_pos <= $heap_size => Some((aligned_pos, next_pos)),
                _ => None,
            },
            None => None,
        }
    }};
}
