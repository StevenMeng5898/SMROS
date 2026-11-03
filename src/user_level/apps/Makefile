# SMROS guest samples. Build AArch64 dynamic PIEs into host_shared/ so the
# shell `run` command can launch them from /shared after a kernel rebuild.
#
#   make -C src/user_level/apps smros
#   make
#   make run
# then in the SMROS shell:
#   run pthread_mutex_futex.elf uncontended
#   run pthread_mutex_futex.elf contended
#   run tensor_neon_mul.elf
#   run epoll_coro.elf

CC ?= gcc
CC_AARCH64 ?= aarch64-linux-gnu-gcc
RUSTC ?= rustc
HOST_SHARED ?= $(abspath $(CURDIR)/../../../host_shared)

CFLAGS ?= -O2 -Wall -Wextra -pthread
NEON_HOST_CFLAGS ?= -O3 -Wall -Wextra
AARCH64_CFLAGS ?= -O2 -Wall -Wextra -fPIE -pthread
AARCH64_NEON_CFLAGS ?= -O3 -Wall -Wextra -fPIE -march=armv8-a
AARCH64_LDFLAGS ?= -pie -Wl,-dynamic-linker,/lib/ld-linux-aarch64.so.1 -Wl,-z,now

FUTEX_SRC := pthread_mutex_futex.c
NEON_SRC := tensor_neon_mul.c
FUTEX_HOST := pthread_mutex_futex
NEON_HOST := tensor_neon_mul.host
FUTEX_ELF := pthread_mutex_futex.elf
NEON_ELF := tensor_neon_mul.elf
EPOLL_SRC := epoll_coro.rs
EPOLL_ELF := epoll_coro.elf
EPOLL_LIB := epoll_coro.a

.PHONY: all smros host clean uncontended contended neon-host install

all: smros

smros: $(FUTEX_ELF) $(NEON_ELF) $(EPOLL_ELF) install

host: $(FUTEX_HOST) $(NEON_HOST)

$(FUTEX_HOST): $(FUTEX_SRC)
	$(CC) $(CFLAGS) -o $@ $< -pthread

$(NEON_HOST): $(NEON_SRC)
	$(CC) $(NEON_HOST_CFLAGS) -o $@ $<

$(FUTEX_ELF): $(FUTEX_SRC)
	$(CC_AARCH64) $(AARCH64_CFLAGS) $(AARCH64_LDFLAGS) -pthread -o $@ $<

$(NEON_ELF): $(NEON_SRC)
	$(CC_AARCH64) $(AARCH64_NEON_CFLAGS) $(AARCH64_LDFLAGS) -o $@ $<

$(EPOLL_LIB): $(EPOLL_SRC)
	$(RUSTC) --edition 2021 --crate-type staticlib --target aarch64-unknown-none \
		-C panic=abort -C opt-level=s -C debuginfo=0 -C strip=symbols -C relocation-model=pic \
		-o $@ $<

$(EPOLL_ELF): $(EPOLL_LIB)
	$(CC_AARCH64) -pie $(AARCH64_LDFLAGS) -o $@ \
		-Wl,--whole-archive $(EPOLL_LIB) -Wl,--no-whole-archive -lc
	aarch64-linux-gnu-strip $@

install: $(FUTEX_ELF) $(NEON_ELF) $(EPOLL_ELF)
	cp -f $(FUTEX_ELF) $(NEON_ELF) $(EPOLL_ELF) $(HOST_SHARED)/
	@echo "installed $(HOST_SHARED)/$(FUTEX_ELF)"
	@echo "installed $(HOST_SHARED)/$(NEON_ELF)"
	@echo "installed $(HOST_SHARED)/$(EPOLL_ELF)"
	@echo "rebuild SMROS (make) so /shared picks up the new ELFs"

uncontended: $(FUTEX_HOST)
	strace -f -e trace=futex ./$(FUTEX_HOST) uncontended

contended: $(FUTEX_HOST)
	strace -f -e trace=futex ./$(FUTEX_HOST) contended

neon-host: $(NEON_HOST)
	./$(NEON_HOST)

clean:
	rm -f $(FUTEX_HOST) $(NEON_HOST) $(FUTEX_ELF) $(NEON_ELF) $(EPOLL_ELF) $(EPOLL_LIB)
