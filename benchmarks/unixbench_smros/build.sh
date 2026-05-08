#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
src_dir="${repo_root}/benchmarks/unixbench_smros"
out="${repo_root}/host_shared/unixbench.elf"

aarch64-linux-gnu-gcc \
  -std=gnu99 \
  -O2 \
  -ffreestanding \
  -fno-stack-protector \
  -fno-builtin \
  -fPIE \
  -nostdlib \
  -nodefaultlibs \
  -pie \
  -Wl,-e,_start \
  -Wl,-dynamic-linker,/lib/ld-linux-aarch64.so.1 \
  -Wl,-z,now \
  -Wl,--build-id=none \
  -o "${out}" \
  "${src_dir}/start.S" \
  "${src_dir}/main.c" \
  "${src_dir}/smros_unix.c" \
  "${src_dir}/syscall.c" \
  "${src_dir}/pipe.c" \
  "${src_dir}/fstime.c" \
  "${src_dir}/arith.c" \
  "${src_dir}/hanoi.c"

stat -c '%n %s bytes' "${out}"
