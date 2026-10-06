macro_rules! smros_linux_id_match_body {
    ($id:expr, $table_id:expr, $any_id:expr) => {{
        $table_id == $any_id || $id == $table_id
    }};
}

macro_rules! smros_linux_pci_id_match_body {
    ($vendor:expr, $device:expr, $table_vendor:expr, $table_device:expr, $any_id:expr) => {{
        ($table_vendor == $any_id || $vendor == $table_vendor)
            && ($table_device == $any_id || $device == $table_device)
    }};
}

macro_rules! smros_linux_of_match_body {
    ($equal:expr, $wildcard:expr) => {{
        $wildcard || $equal
    }};
}

macro_rules! smros_linux_virtio_id_match_body {
    ($device_id:expr, $table_id:expr, $any_id:expr) => {{
        $table_id == $any_id || $device_id == $table_id
    }};
}

macro_rules! smros_linux_virtio_device_present_body {
    ($device_id:expr) => {{
        $device_id != 0
    }};
}

macro_rules! smros_linux_id_table_end_body {
    ($vendor:expr, $device:expr) => {{
        $vendor == 0 && $device == 0
    }};
}

macro_rules! smros_linux_module_license_ok_body {
    ($len:expr) => {{
        $len != 0
    }};
}

macro_rules! smros_linux_chrdev_minor_valid_body {
    ($minor:expr, $max:expr, $dynamic:expr) => {{
        $minor == $dynamic || $minor < $max
    }};
}

macro_rules! smros_linux_irq_number_valid_body {
    ($irq:expr, $max:expr) => {{
        $irq != 0 && $irq < $max
    }};
}

macro_rules! smros_linux_irq_shared_body {
    ($flags:expr, $shared:expr) => {{
        $flags & $shared != 0
    }};
}

macro_rules! smros_linux_ioremap_len_valid_body {
    ($len:expr, $max:expr) => {{
        $len != 0 && $len <= $max
    }};
}

macro_rules! smros_linux_dma_size_valid_body {
    ($size:expr, $max:expr) => {{
        $size != 0 && $size <= $max
    }};
}

macro_rules! smros_linux_file_copy_len_body {
    ($count:expr, $remaining:expr) => {{
        if $count <= $remaining {
            $count
        } else {
            $remaining
        }
    }};
}

macro_rules! smros_linux_bus_kind_valid_body {
    ($kind:expr, $platform:expr, $virtio:expr, $pci:expr) => {{
        $kind == $platform || $kind == $virtio || $kind == $pci
    }};
}

macro_rules! smros_linux_probe_status_ok_body {
    ($code:expr) => {{
        $code == 0
    }};
}
