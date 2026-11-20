# C and C++ Formal Coverage

This inventory is separate from `docs/VERUS_COVERAGE.md`. Verus does not compile
these files. A passing runtime test is not a formal proof.

Labels match the Verus audit:

- 已证明: a saved CBMC, Frama-C, or VeriFast run discharged the claimed property.
- 待证明: in scope for a C prover and not yet discharged.
- 公理或可信计算基: trusted by the C proof rather than proved in this tree.
- 规范假设: an explicit specification assumption.
- 超出范围: not part of the kernel TCB for this C audit.
- 未覆盖: present and not otherwise classified.

The upstream Open POSIX Test Suite is pinned in
`third_party/posixtest/source.lock.json` and checked out under `target/posix`.
That checkout and the existing `third_party/posixtest/patches/` series are not
modified and are not listed here or in `docs/VERUS_COVERAGE.md`.
`scripts/posix/runtime/` is the SMROS compat layer, same class as the DDK
compat layer, and is 待证明. `tests/guest/` is SMROS's own guest probes, not
the upstream suite and not kernel TCB; those files stay 待证明 rather than
超出范围. `benchmarks/` is a performance suite, not a conformance suite, and
stays 未覆盖 until a C proof is assigned. Demos stay 超出范围:
`include/smros/examples/hello_linux.c` and
`src/user_level/apps/tensor_neon_mul.c`.

Priority for the in-scope DDK layer is CBMC for bounds, overflow, and null
dereference. ACSL with Frama-C Wp is appropriate once a function contract can
be written. VeriFast is reserved for a separation-logic property CBMC and Wp
cannot state. No C proof is discharged in this change, so nothing is 已证明.

## Classified C and C++ Files

- `benchmarks/dhrystone_smros/dhry.h`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/dhrystone_smros/dhry_1.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/dhrystone_smros/dhry_2.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/dhrystone_smros/main.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/dhrystone_smros/smros_dhry.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/dhrystone_smros/smros_dhry.h`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/channels.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/freestanding.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/handles.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/main.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/perftest.h`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/smros_zircon.h`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/time.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/fuchsia_microbenchmarks_smros/vmo.cc`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/arith.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/fstime.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/hanoi.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/main.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/pipe.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/smros_unix.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/smros_unix.h`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `benchmarks/unixbench_smros/syscall.c`
  - 未覆盖
    Performance test, not a conformance suite. Not yet assigned a C proof.
- `include/asm/io.h`
  - 待证明
    MMIO accessors used by the compat layer. Priority CBMC for width and null.
- `include/linux/compiler.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/device.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/dma-mapping.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/err.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/errno.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/export.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/gfp.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/init.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/interrupt.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/io.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/ioport.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/kernel.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/mod_devicetable.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/module.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/moduleparam.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/pci.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/printk.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/slab.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/stddef.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/string.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/linux/types.h`
  - 待证明
    Linux-shaped header in the DDK compat layer. Priority CBMC; no contract is discharged yet.
- `include/smros/ddk.h`
  - 待证明
    C DDK surface. Contracts belong with the compat .c files, not with Verus.
- `include/smros/examples/hello_linux.c`
  - 超出范围
    Sample driver, not kernel TCB.
- `scripts/posix/runtime/include/sched.h`
  - 待证明
    SMROS POSIX compat header. Not yet discharged. The upstream suite under `target/posix` is not this file.
- `scripts/posix/runtime/include/unistd.h`
  - 待证明
    SMROS POSIX compat header. Not yet discharged. The upstream suite under `target/posix` is not this file.
- `scripts/posix/runtime/smros_posix_compat.c`
  - 待证明
    SMROS POSIX compat runtime. Not yet discharged. The upstream suite under `target/posix` is not this file.
- `src/user_level/apps/pthread_mutex_futex.c`
  - 待证明
    EL0 futex mutex demo. Secondary CBMC target. Kernel futex proofs stay on the Verus track.
- `src/user_level/apps/tensor_neon_mul.c`
  - 超出范围
    EL0 NEON GEMM demo, not kernel TCB.
- `src/user_level/drivers/linuxcompat/compat.c`
  - 待证明
    Freestanding C runtime for the Linux DDK: kmalloc, ioremap, IRQ, and DMA helpers. Priority CBMC (bounds, overflow, null). Not yet discharged.
- `src/user_level/drivers/linuxcompat/edu.c`
  - 待证明
    Sample Linux PCI driver built on the compat headers. Priority CBMC / ACSL when a contract is written.
- `src/user_level/drivers/linuxcompat/pci.c`
  - 待证明
    PCI config and BAR helpers in the DDK compat layer. Priority CBMC.
- `tests/guest/f1_seccomp.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/f2_landlock.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/f3_pkey.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/f4_iouring.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/f5_ptrace.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/fail_closed.h`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/g1_stopcont.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/g1b_stop.c`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
- `tests/guest/poc_common.h`
  - 待证明
    SMROS guest test, not the upstream POSIX suite. Not yet discharged.
