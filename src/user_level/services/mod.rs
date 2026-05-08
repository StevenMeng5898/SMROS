//! User-space service implementations and compatibility layers.

pub mod compat_apps;
pub mod component;
pub mod docker_compat;
pub mod elf;
pub mod fxfs;
pub mod host_share;
pub mod net;
pub mod run_elf;
pub mod svc;
pub(crate) mod user_logic;
pub mod user_shell;
