# Memory Syscalls

This document tracks the memory syscall behavior that is implemented in the current tree. It covers the Linux-facing VM syscalls, the Zircon VMO/VMAR syscalls, the shell test command that exercises them, and the extra memory state exposed by `meminfo`.

## Scope

Relevant code:

- `src/syscall/syscall.rs`
- `src/kernel_objects/vmo.rs`
- `src/kernel_objects/vmar.rs`
- `src/user_level/services/user_shell.rs`
- `src/kernel_lowlevel/memory.rs`

## Summary

SMROS now has:

- a page-frame-backed Linux mapping registry used by `mmap`, `munmap`, `mprotect`, and `mremap`
- a global `brk` window with grow and shrink page accounting
- a handle-backed Zircon VMO registry with real `read`, `write`, `get_size`, `set_size`, and `op_range` behavior
- a software VMAR tree with mapping, protection, allocation, destroy, and unmap bookkeeping
- a boot-time EL0 `svc #0` smoke test for Linux `write`, `getpid`, `mmap`, and `exit`
- shell-visible memory stats for Linux mappings, `brk`, VMO state, and VMAR state
- shell `testsc` coverage for the broader syscall compatibility model, including Linux file/fd/poll/stat paths and Zircon time/debug/system/exception and hypervisor paths
- a richer FxFS-shaped object store with an in-memory object table and block-image persistence when virtio-blk is present
- user-level VirtIO block persistence for the FxFS image when QEMU provides `smros-fxfs.img`
- FxFS-backed Linux file descriptors for open/read/write/stat and file-backed `mmap`
- a minimal `/svc` fixed-message IPC layer that uses Zircon channels for component-manager, runner, and filesystem service requests

The model is still software bookkeeping layered on top of the real page allocator. It is suitable for syscall bring-up, the dynamic-loader bring-up path, and shell testing, but it is not yet a full per-process hardware page-table runtime.

## Boot-Time EL0 Validation

Before the shell starts, SMROS now drops into EL0 and issues real Linux-style syscalls through the active exception vector:

- `write`
- `getpid`
- `mmap`
- `exit`

The kernel records the EL0 syscall results on the EL1 side and prints the validation status before starting the shell. This validates the real `svc` path separately from the later shell `testsc` command.

## Linux Memory Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_mmap` | supports anonymous `MAP_PRIVATE`/`MAP_SHARED` mappings and FxFS-backed file mappings through Linux fd records, rounds length to pages, allocates page frames, tracks the virtual range, honors `MAP_FIXED` by replacing overlapping mappings, zero-fills the range, and copies file bytes when a file fd is supplied | implemented in software model |
| `sys_munmap` | unmaps page-aligned ranges, frees backing page frames, and splits surviving mapping fragments | implemented |
| `sys_mprotect` | updates protection bits on page-aligned subranges and splits mapping records as needed | implemented |
| `sys_brk` | maintains a global heap window at `0x4000_0000..0x400F_FFFF`, commits pages on growth, frees pages on shrink, and returns the current break | implemented, global not per-process |
| `sys_mremap` | resizes exact mappings, shrinks in place, grows in place when space is free, or moves the mapping for `MREMAP_MAYMOVE` / `MREMAP_FIXED` | implemented in software model |

### Linux Notes

- File-backed `mmap` is implemented for FxFS-backed Linux file descriptors. Other fd categories still return an error.
- Linux mappings are tracked in a dedicated kernel-side registry rooted at `0x5000_0000`.
- `mremap` preserves the mapping shape and page accounting, but it does not copy user data into a real hardware remap path.
- The shell dynamic-loader path uses this Linux mapping window to place the main PIE, interpreter, and stack.

## Zircon VMO Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_vmo_create` | creates handle-backed paged, resizable, contiguous, and emulated physical VMOs | implemented |
| `sys_vmo_read` | looks up the VMO handle and copies data from the software VMO buffer | implemented |
| `sys_vmo_write` | looks up the VMO handle, commits pages for the written range, and copies data into the software VMO buffer | implemented |
| `sys_vmo_get_size` | reports the real handle-backed VMO size | implemented |
| `sys_vmo_set_size` | resizes resizable VMOs and updates page allocation state | implemented |
| `sys_vmo_op_range` | supports `Commit`, `Decommit`, `Zero`, `Lock`, `Unlock`, `CacheSync`, `CacheInvalidate`, `CacheClean`, and `CacheCleanInvalidate` | implemented |

### VMO Notes

- `Commit` and `Decommit` change committed page state in the VMO object.
- `Zero` clears the selected byte range in the software VMO buffer.
- `Lock`, `Unlock`, and cache ops are accepted as successful software-model operations.

## Zircon VMAR Syscalls

| Syscall | Current Behavior | Status |
|---------|------------------|--------|
| `sys_vmar_map` | validates options and offsets, checks the VMO range, and creates a real VMAR mapping record | implemented |
| `sys_vmar_unmap` | removes page-aligned subranges and splits remaining mappings | implemented |
| `sys_vmar_protect` | changes protection bits on mapped subranges | implemented |
| `sys_vmar_allocate` | allocates a child VMAR, returns a new handle, and links it to the parent VMAR | implemented |
| `sys_vmar_destroy` | destroys child VMARs recursively and clears mapping bookkeeping | implemented |
| `sys_vmar_unmap_handle_close_thread_exit` | performs the same software unmap path used for stack-teardown style cleanup | implemented in software model |

### VMAR Notes

- The root software VMAR is created lazily and exposed through the shell test path.
- VMAR bookkeeping is real inside the kernel object model, but it is not yet synchronized to hardware page tables for live EL0 execution.

## Object-Layer Backing

### `Vmo`

`src/kernel_objects/vmo.rs` now provides:

- committed-page tracking per VMO page
- a byte buffer used by `read`, `write`, and `zero`
- resizable page accounting
- explicit page release when handles are closed

### `Vmar`

`src/kernel_objects/vmar.rs` now provides:

- sorted mapping bookkeeping
- overlap checks for new mappings
- split-aware `unmap`
- split-aware `protect`
- aligned free-region search for child VMAR allocation

## Shell Test Commands

Use the default run target to build, boot, attach the virtio-blk image and
virtio-net device, and verify the memory syscall paths:

```sh
make run
```

During boot, expect a line like:

```text
[EL0] Real EL0 -> SVC -> EL1 validation: SUCCESS
```

Then use these commands from the SMROS shell:

```sh
help
meminfo
testsc
meminfo
ps
top
```

What they cover:

- boot log EL0 validation: exercises real EL0 `svc` handling for Linux `write`, `getpid`, `mmap`, and `exit`
- `help`: confirms the command is exposed in the live shell
- `meminfo`: prints allocator state plus Linux mapping, `brk`, VMO, and VMAR counters
- `testsc`: runs the Linux and Zircon syscall smoke suite, including memory/object/channel paths
- `ps` and `top`: provide supporting process and page usage context around the memory test

## `testsc` Coverage

The shell's `testsc` command now exercises:

- Linux `write`
- Linux `getpid`
- Linux `getppid`
- Linux `gettid`
- Linux `execve`
- Linux `wait4`
- Linux `clock_gettime`
- Linux `nanosleep`
- Linux `getrandom`
- Linux `memfd_create`
- Linux `close_range`
- Linux signal action/mask/queue helpers
- Linux `signalfd4`
- Linux SysV semaphore, message queue, and shared-memory helpers
- Linux socket, socketpair, send/receive, address, and socket-option helpers
- Linux `openat` for modeled files and directories
- Linux `dup`, `dup3`, and `fcntl`
- Linux `getdents64`
- Linux `fstat`, `fstatat`, `statfs`, `fstatfs`, and `statx`
- Linux `writev`
- Linux `poll`
- Linux `lseek`, `ftruncate`, `fsync`, and `sync_file_range`
- component topology startup for `/bootstrap/fxfs` and `/bootstrap/user-init`
- minimal ELF loader metadata for bootstrap component binaries
- FxFS-shaped `/pkg/bin` lookup plus `/data` file write, append, truncate, seek/read, attribute, and replay checks
- `/svc` service connection and fixed request/reply checks over Zircon channels
- Linux `brk`
- Linux `mmap`
- Linux `mprotect`
- Linux `mremap`
- Linux `munmap`
- Zircon `vmo_create`
- Zircon `vmo_write`
- Zircon `vmo_read`
- Zircon `vmo_get_size`
- Zircon `vmo_set_size`
- Zircon `vmo_op_range`
- Zircon `vmar_map`
- Zircon `vmar_protect`
- Zircon `vmar_allocate`
- Zircon `vmar_unmap`
- Zircon `vmar_unmap_handle_close_thread_exit`
- Zircon `vmar_destroy`
- Zircon `handle_duplicate`
- Zircon `object_get_info`
- Zircon `object_set_property`
- Zircon `object_get_property`
- Zircon `channel_create`
- Zircon `channel_write`
- Zircon `channel_read`
- Zircon `process_create`
- Zircon `thread_create`
- Zircon `thread_start`
- Zircon `object_signal`
- Zircon `object_signal_peer`
- Zircon `object_wait_one`
- Zircon `object_wait_many`
- Zircon port create, queue, wait, wait-async, and cancel helpers
- Zircon socket stream, datagram, shared, shutdown, info, and threshold helpers
- Zircon FIFO create/read/write/signal/close behavior
- Zircon futex wait/wake/requeue/owner behavior
- Zircon `clock_get_monotonic`
- Zircon clock create/read/update
- Zircon timer create/set/cancel
- Zircon debuglog create/read/write
- Zircon debug read/write/send-command
- Zircon system event handles
- Zircon exception channel lifecycle helpers
- Zircon guest create/trap helpers
- Zircon VCPU create/resume/interrupt/read-state/write-state helpers
- Zircon SMC call modeling
- Zircon `nanosleep`
- Zircon `task_kill`
- Zircon `process_exit`
- Zircon `handle_close_many`
- Zircon `handle_close` for the VMO handle created by the test

## Extra Memory Info In `meminfo`

`meminfo` now reports:

- allocator totals, used pages, free pages, and page size
- Linux mapping count, mapped bytes, and committed Linux pages
- Linux `brk` start, current break, limit, and committed pages
- Zircon VMO count, VMO bytes, and committed VMO pages
- Zircon VMAR count, VMAR mapping count, and the root VMAR handle

## Remaining Gaps

The memory syscall layer is now complete as a kernel-side software model, but there are still system-level gaps:

- file-backed `mmap` is available for FxFS-backed Linux file descriptors and is used by the dynamic loader support path
- no per-process `brk`
- no live VMAR-to-hardware page-table synchronization in the booted shell path
- no full Zircon process-local handle table yet
