//! Build-time snapshot of the repository-local host_shared/ directory.
//!
//! The guest currently has virtio block and net drivers, but no live 9p or
//! virtio-fs filesystem driver. This module embeds host_shared/ during the
//! kernel build so FxFS can expose it at /shared.

include!(concat!(env!("OUT_DIR"), "/host_share.rs"));
