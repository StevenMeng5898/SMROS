include!("driver_logic_shared.rs");

pub(crate) fn mmio_slot_base(base: usize, slot: usize, stride: usize) -> Option<usize> {
    smros_driver_mmio_slot_base_body!(base, slot, stride)
}

pub(crate) fn virtio_identity_valid(
    magic: u32,
    device_id: u32,
    expected_device: u32,
    vendor: u32,
    expected_vendor: u32,
) -> bool {
    smros_driver_virtio_identity_valid_body!(
        magic,
        device_id,
        expected_device,
        vendor,
        expected_vendor
    )
}

pub(crate) fn virtio_version_supported(version: u32, legacy: u32, modern: u32) -> bool {
    smros_driver_virtio_version_supported_body!(version, legacy, modern)
}

pub(crate) fn virtio_version_is_modern(version: u32, modern: u32) -> bool {
    smros_driver_virtio_version_is_modern_body!(version, modern)
}

pub(crate) fn virtio_queue_size_valid(max_queue: u32, queue_size: u16) -> bool {
    smros_driver_virtio_queue_size_valid_body!(max_queue, queue_size)
}

pub(crate) fn virtio_feature_present(features: u64, feature: u64) -> bool {
    smros_driver_virtio_feature_present_body!(features, feature)
}

pub(crate) fn virtio_block_accepted_features(features: u64, flush: u64, config_wce: u64) -> u64 {
    smros_driver_virtio_block_accepted_features_body!(features, flush, config_wce)
}

pub(crate) fn virtio_driver_features(accepted: u64, version_1: u64, modern: bool) -> u64 {
    smros_driver_virtio_driver_features_body!(accepted, version_1, modern)
}

pub(crate) fn virtio_net_accepted_features(
    features: u64,
    mac: u64,
    status: u64,
    version_1: u64,
    modern: bool,
) -> u64 {
    smros_driver_virtio_net_accepted_features_body!(features, mac, status, version_1, modern)
}

pub(crate) fn block_capacity_bytes(blocks: usize, block_size: usize) -> usize {
    smros_driver_block_capacity_bytes_body!(blocks, block_size)
}

pub(crate) fn block_range_valid(
    offset: usize,
    len: usize,
    blocks: usize,
    block_size: usize,
) -> bool {
    smros_driver_block_range_valid_body!(offset, len, blocks, block_size)
}

pub(crate) fn block_len_valid(len: usize, block_size: usize) -> bool {
    smros_driver_block_len_valid_body!(len, block_size)
}

pub(crate) fn block_id_valid(block: usize, capacity_blocks: usize) -> bool {
    smros_driver_block_id_valid_body!(block, capacity_blocks)
}

pub(crate) fn net_tx_frame_len_valid(
    frame_len: usize,
    max_frame: usize,
    header_len: usize,
    buffer_size: usize,
) -> bool {
    smros_driver_net_tx_frame_len_valid_body!(frame_len, max_frame, header_len, buffer_size)
}

pub(crate) fn net_rx_packet_len_valid(
    packet_len: usize,
    header_len: usize,
    buffer_size: usize,
) -> bool {
    smros_driver_net_rx_packet_len_valid_body!(packet_len, header_len, buffer_size)
}

pub(crate) fn net_rx_frame_len(packet_len: usize, header_len: usize) -> Option<usize> {
    smros_driver_net_rx_frame_len_body!(packet_len, header_len)
}

pub(crate) fn net_rx_output_len_valid(frame_len: usize, out_len: usize) -> bool {
    smros_driver_net_rx_output_len_valid_body!(frame_len, out_len)
}
