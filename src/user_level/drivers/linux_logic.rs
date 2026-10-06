include!("linux_logic_shared.rs");

pub(crate) fn id_match(id: u32, table_id: u32, any_id: u32) -> bool {
    smros_linux_id_match_body!(id, table_id, any_id)
}

pub(crate) fn pci_id_match(
    vendor: u32,
    device: u32,
    table_vendor: u32,
    table_device: u32,
    any_id: u32,
) -> bool {
    smros_linux_pci_id_match_body!(vendor, device, table_vendor, table_device, any_id)
}

pub(crate) fn of_match(equal: bool, wildcard: bool) -> bool {
    smros_linux_of_match_body!(equal, wildcard)
}

pub(crate) fn virtio_id_match(device_id: u32, table_id: u32, any_id: u32) -> bool {
    smros_linux_virtio_id_match_body!(device_id, table_id, any_id)
}

pub(crate) fn virtio_device_present(device_id: u32) -> bool {
    smros_linux_virtio_device_present_body!(device_id)
}

pub(crate) fn id_table_end(vendor: u32, device: u32) -> bool {
    smros_linux_id_table_end_body!(vendor, device)
}

pub(crate) fn module_license_ok(len: usize) -> bool {
    smros_linux_module_license_ok_body!(len)
}

pub(crate) fn chrdev_minor_valid(minor: u32, max: u32, dynamic: u32) -> bool {
    smros_linux_chrdev_minor_valid_body!(minor, max, dynamic)
}

pub(crate) fn irq_number_valid(irq: u32, max: u32) -> bool {
    smros_linux_irq_number_valid_body!(irq, max)
}

pub(crate) fn irq_shared(flags: u32, shared: u32) -> bool {
    smros_linux_irq_shared_body!(flags, shared)
}

pub(crate) fn ioremap_len_valid(len: usize, max: usize) -> bool {
    smros_linux_ioremap_len_valid_body!(len, max)
}

pub(crate) fn dma_size_valid(size: usize, max: usize) -> bool {
    smros_linux_dma_size_valid_body!(size, max)
}

pub(crate) fn file_copy_len(count: usize, remaining: usize) -> usize {
    smros_linux_file_copy_len_body!(count, remaining)
}

pub(crate) fn bus_kind_valid(kind: u32, platform: u32, virtio: u32, pci: u32) -> bool {
    smros_linux_bus_kind_valid_body!(kind, platform, virtio, pci)
}

pub(crate) fn probe_status_ok(code: i32) -> bool {
    smros_linux_probe_status_ok_body!(code)
}
