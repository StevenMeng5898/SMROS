# Verus

Verus verification is kept separate from the `smros` kernel crate so the ARM64 `no_std` build stays unchanged.

The first verified slice is the standalone proof file at `verification/syscall/src/lib.rs`, which models the overflow-safe address-range helpers used by `src/syscall/syscall.rs`.

Commands:

- `make verus-setup`
- `make verus-syscall`

The setup script downloads the pinned Verus release into `/.tools/verus/current` and installs the Rust toolchain requested by that Verus release.
