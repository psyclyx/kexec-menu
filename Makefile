# kexec-menu — build without Nix
#
# Prerequisites:
#   - Rust toolchain with targets:
#       rustup target add x86_64-unknown-linux-musl
#       rustup target add aarch64-unknown-linux-musl
#   - musl cross-compilers (for cross builds):
#       x86_64-linux-musl-gcc   (if cross-compiling x86_64)
#       aarch64-linux-musl-gcc  (for aarch64)
#
# Usage:
#   make                    # build x86_64 release binary
#   make aarch64            # build aarch64 release binary
#   make all                # build both
#   make test               # run unit tests
#   make clean              # remove build artifacts

CARGO ?= cargo
RUSTFLAGS_STATIC = -C target-feature=+crt-static

X86_64_TARGET  = x86_64-unknown-linux-musl
AARCH64_TARGET = aarch64-unknown-linux-musl

X86_64_BIN  = target/$(X86_64_TARGET)/release/kexec-menu
AARCH64_BIN = target/$(AARCH64_TARGET)/release/kexec-menu

# Cross-linker overrides (set these if your musl-gcc has a different name)
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER   ?= x86_64-linux-musl-gcc
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER  ?= aarch64-linux-musl-gcc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER

.PHONY: x86_64 aarch64 all test clean

x86_64: $(X86_64_BIN)
aarch64: $(AARCH64_BIN)
all: x86_64 aarch64

$(X86_64_BIN): Cargo.toml Cargo.lock $(shell find crates -name '*.rs' -o -name 'Cargo.toml')
	RUSTFLAGS="$(RUSTFLAGS_STATIC)" $(CARGO) build --release --target $(X86_64_TARGET)

$(AARCH64_BIN): Cargo.toml Cargo.lock $(shell find crates -name '*.rs' -o -name 'Cargo.toml')
	RUSTFLAGS="$(RUSTFLAGS_STATIC)" $(CARGO) build --release --target $(AARCH64_TARGET)

test:
	$(CARGO) test --target $(X86_64_TARGET)

clean:
	$(CARGO) clean
