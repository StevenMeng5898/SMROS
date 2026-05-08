macro_rules! smros_driver_mmio_slot_base_body {
    ($base:expr, $slot:expr, $stride:expr) => {{
        match $slot.checked_mul($stride) {
            Some(offset) => $base.checked_add(offset),
            None => None,
        }
    }};
}

macro_rules! smros_driver_virtio_identity_valid_body {
    ($magic:expr, $device_id:expr, $expected_device:expr, $vendor:expr, $expected_vendor:expr) => {{
        $magic == 0x7472_6976u32 && $device_id == $expected_device && $vendor == $expected_vendor
    }};
}

macro_rules! smros_driver_virtio_version_supported_body {
    ($version:expr, $legacy:expr, $modern:expr) => {{
        $version == $legacy || $version == $modern
    }};
}

macro_rules! smros_driver_virtio_version_is_modern_body {
    ($version:expr, $modern:expr) => {{
        $version == $modern
    }};
}

macro_rules! smros_driver_virtio_queue_size_valid_body {
    ($max_queue:expr, $queue_size:expr) => {{
        $queue_size != 0 && $max_queue >= $queue_size as u32
    }};
}

macro_rules! smros_driver_virtio_feature_present_body {
    ($features:expr, $feature:expr) => {{
        $features & $feature != 0
    }};
}

macro_rules! smros_driver_virtio_block_accepted_features_body {
    ($features:expr, $flush:expr, $config_wce:expr) => {{
        $features & ($flush | $config_wce)
    }};
}

macro_rules! smros_driver_virtio_driver_features_body {
    ($accepted:expr, $version_1:expr, $modern:expr) => {{
        if $modern {
            $accepted | $version_1
        } else {
            $accepted
        }
    }};
}

macro_rules! smros_driver_virtio_net_accepted_features_body {
    ($features:expr, $mac:expr, $status:expr, $version_1:expr, $modern:expr) => {{
        let mut accepted = 0u64;
        if $features & $mac != 0 {
            accepted |= $mac;
        }
        if $features & $status != 0 {
            accepted |= $status;
        }
        if $modern {
            accepted |= $version_1;
        }
        accepted
    }};
}

macro_rules! smros_driver_block_capacity_bytes_body {
    ($blocks:expr, $block_size:expr) => {{
        match $blocks.checked_mul($block_size) {
            Some(bytes) => bytes,
            None => usize::MAX,
        }
    }};
}

macro_rules! smros_driver_block_range_valid_body {
    ($offset:expr, $len:expr, $blocks:expr, $block_size:expr) => {{
        match $blocks.checked_mul($block_size) {
            Some(capacity) => match $offset.checked_add($len) {
                Some(end) => end <= capacity,
                None => false,
            },
            None => false,
        }
    }};
}

macro_rules! smros_driver_block_len_valid_body {
    ($len:expr, $block_size:expr) => {{
        $len == $block_size
    }};
}

macro_rules! smros_driver_block_id_valid_body {
    ($block:expr, $capacity_blocks:expr) => {{
        $block < $capacity_blocks
    }};
}

macro_rules! smros_driver_net_tx_frame_len_valid_body {
    ($frame_len:expr, $max_frame:expr, $header_len:expr, $buffer_size:expr) => {{
        $frame_len <= $max_frame
            && match $frame_len.checked_add($header_len) {
                Some(total) => total <= $buffer_size,
                None => false,
            }
    }};
}

macro_rules! smros_driver_net_rx_packet_len_valid_body {
    ($packet_len:expr, $header_len:expr, $buffer_size:expr) => {{
        $packet_len >= $header_len && $packet_len <= $buffer_size
    }};
}

macro_rules! smros_driver_net_rx_frame_len_body {
    ($packet_len:expr, $header_len:expr) => {{
        $packet_len.checked_sub($header_len)
    }};
}

macro_rules! smros_driver_net_rx_output_len_valid_body {
    ($frame_len:expr, $out_len:expr) => {{
        $frame_len <= $out_len
    }};
}
