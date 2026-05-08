#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
out="${repo_root}/host_shared/bench.elf"
src_dir="${repo_root}/benchmarks/fuchsia_microbenchmarks_smros"

aarch64-linux-gnu-g++ \
  -std=c++17 \
  -O2 \
  -ffreestanding \
  -fno-exceptions \
  -fno-rtti \
  -fno-stack-protector \
  -fno-threadsafe-statics \
  -fno-use-cxa-atexit \
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
  "${src_dir}/main.cc" \
  "${src_dir}/channels.cc" \
  "${src_dir}/vmo.cc" \
  "${src_dir}/handles.cc" \
  "${src_dir}/time.cc" \
  "${src_dir}/freestanding.cc"

stat -c '%n %s bytes' "${out}"
