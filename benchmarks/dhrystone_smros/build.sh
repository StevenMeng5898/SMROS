#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
src_dir="${repo_root}/benchmarks/dhrystone_smros"
out="${repo_root}/host_shared/dhrystone.elf"

mkdir -p "$(dirname "${out}")"

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
  "${src_dir}/smros_dhry.c" \
  "${src_dir}/dhry_1.c" \
  "${src_dir}/dhry_2.c"

stat -c '%n %s bytes' "${out}"
