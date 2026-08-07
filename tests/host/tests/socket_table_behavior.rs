pub const INVALID_HANDLE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HandleValue(pub u32);

#[derive(Clone, Copy)]
pub enum ObjectType {
    Socket,
}

#[repr(u32)]
pub enum Rights {
    Transfer = 1 << 1,
    Read = 1 << 2,
    Write = 1 << 3,
    GetProperty = 1 << 6,
    SetProperty = 1 << 7,
    SignalPeer = 1 << 13,
    Inspect = 1 << 15,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZxError {
    ErrInternal,
    ErrNoMemory,
    ErrInvalidArgs,
    ErrAccessDenied,
    ErrNotFound,
    ErrOutOfRange,
    ErrBadState,
    ErrShouldWait,
    ErrPeerClosed,
}

pub type ZxResult<T = ()> = Result<T, ZxError>;

pub fn default_rights_for_object(_object_type: ObjectType) -> u32 {
    Rights::Transfer as u32
        | Rights::Read as u32
        | Rights::Write as u32
        | Rights::GetProperty as u32
        | Rights::SetProperty as u32
        | Rights::SignalPeer as u32
        | Rights::Inspect as u32
}

pub fn rights_contain(rights: u32, required: u32) -> bool {
    rights & required == required
}

pub mod kernel_objects {
    pub use crate::{default_rights_for_object, rights_contain, ObjectType, Rights};

    pub mod channel {
        pub const CHANNEL_SIGNAL_READABLE: u32 = 1 << 0;
        pub const CHANNEL_SIGNAL_WRITABLE: u32 = 1 << 1;
        pub const CHANNEL_SIGNAL_PEER_CLOSED: u32 = 1 << 2;
    }
}

pub(crate) mod object_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/object_logic_shared.rs"
    ));

    pub fn handle_is_valid(handle: u32, invalid: u32) -> bool {
        smros_ko_handle_is_valid_body!(handle, invalid)
    }

    pub fn signal_update(current: u32, clear_mask: u32, set_mask: u32) -> u32 {
        smros_ko_signal_update_body!(current, clear_mask, set_mask)
    }
}

pub(crate) mod socket_logic {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../src/kernel_objects/socket_logic_shared.rs"
    ));

    pub fn options_valid(options: u32, mask: u32) -> bool {
        smros_socket_options_valid_body!(options, mask)
    }

    pub fn mask_options(options: u32, mask: u32) -> u32 {
        smros_socket_mask_options_body!(options, mask)
    }

    pub fn ring_index(read_pos: usize, offset: usize, capacity: usize) -> usize {
        smros_socket_ring_index_body!(read_pos, offset, capacity)
    }

    pub fn remaining_capacity(len: usize, capacity: usize) -> usize {
        smros_socket_remaining_capacity_body!(len, capacity)
    }

    pub fn min_count(left: usize, right: usize) -> usize {
        smros_socket_min_count_body!(left, right)
    }

    pub fn refresh_read_signals(
        signals: u32,
        len: usize,
        threshold: usize,
        readable_signal: u32,
        threshold_signal: u32,
    ) -> u32 {
        smros_socket_refresh_read_signals_body!(
            signals,
            len,
            threshold,
            readable_signal,
            threshold_signal
        )
    }

    pub fn refresh_write_signals(
        signals: u32,
        write_disabled: bool,
        remaining: usize,
        threshold: usize,
        writable_signal: u32,
        threshold_signal: u32,
    ) -> u32 {
        smros_socket_refresh_write_signals_body!(
            signals,
            write_disabled,
            remaining,
            threshold,
            writable_signal,
            threshold_signal
        )
    }
}

#[path = "../../../src/kernel_objects/socket.rs"]
mod socket;

use kernel_objects::channel::CHANNEL_SIGNAL_READABLE;
use socket::{socket_table, SOCKET_DATAGRAM};

#[test]
fn datagram_reads_truncate_one_packet_and_consume_empty_packets() {
    let sockets = socket_table();
    let (writer, reader) = sockets
        .create_pair(SOCKET_DATAGRAM)
        .expect("create datagram pair");

    let first: Vec<u8> = (0..700).map(|index| (index % 251) as u8).collect();
    let second = b"second packet stays intact";
    assert_eq!(sockets.write(writer, &first), Ok(first.len()));
    assert_eq!(sockets.write(writer, second), Ok(second.len()));

    let mut short = [0u8; 300];
    assert_eq!(sockets.read(reader, 0, &mut short), Ok(short.len()));
    assert_eq!(&short[..], &first[..short.len()]);
    assert_eq!(
        sockets
            .info(reader)
            .expect("reader socket")
            .rx_buf_available,
        second.len() as u64
    );

    let mut remaining = [0u8; 64];
    assert_eq!(sockets.read(reader, 0, &mut remaining), Ok(second.len()));
    assert_eq!(&remaining[..second.len()], second);

    assert_eq!(sockets.write(writer, &[]), Ok(0));
    assert_ne!(
        sockets.signals(reader).expect("reader signals") & CHANNEL_SIGNAL_READABLE,
        0
    );
    assert_eq!(sockets.read(reader, 0, &mut []), Ok(0));
    assert_eq!(
        sockets.signals(reader).expect("reader signals") & CHANNEL_SIGNAL_READABLE,
        0
    );

    assert!(sockets.close(writer));
    assert!(sockets.close(reader));
}
