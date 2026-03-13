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
#
# Component builds (from source — downloads automatically):
#   make busybox ARCH=x86_64           # build static busybox from source
#   make bcachefs-tools ARCH=x86_64    # build static bcachefs binary from source
#   make kernel ARCH=x86_64            # download + build minimal kernel from source
#
# UKI build targets (see README for full instructions):
#   make logo                          # generate boot logo PPM
#   make initrd ARCH=x86_64            # assemble initrd (needs BUSYBOX, CRYPTSETUP, BCACHEFS)
#   make uki    ARCH=x86_64            # full UKI (orchestrates all above)

CARGO ?= cargo
RUSTFLAGS_STATIC = -C target-feature=+crt-static

# Architecture (x86_64 or aarch64)
ARCH ?= x86_64

X86_64_TARGET  = x86_64-unknown-linux-musl
AARCH64_TARGET = aarch64-unknown-linux-musl

ifeq ($(ARCH),x86_64)
  RUST_TARGET = $(X86_64_TARGET)
else ifeq ($(ARCH),aarch64)
  RUST_TARGET = $(AARCH64_TARGET)
else
  $(error Unsupported ARCH=$(ARCH). Use x86_64 or aarch64)
endif

X86_64_BIN  = target/$(X86_64_TARGET)/release/kexec-menu
AARCH64_BIN = target/$(AARCH64_TARGET)/release/kexec-menu
BIN         = target/$(RUST_TARGET)/release/kexec-menu

# Cross-linker overrides (set these if your musl-gcc has a different name)
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER   ?= x86_64-linux-musl-gcc
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER  ?= aarch64-linux-musl-gcc
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER

# Build output directory
BUILD_DIR ?= build
LOGO_PPM   = $(BUILD_DIR)/logo.ppm
INITRD     = $(BUILD_DIR)/initrd.cpio
KERNEL_OUT = $(BUILD_DIR)/vmlinuz
UKI_OUT    = $(BUILD_DIR)/kexec-menu.efi

# Paths to static binaries for initrd (user-provided or built from source)
BUSYBOX    ?=
CRYPTSETUP ?=
BCACHEFS   ?=

# Component build versions (for building from source)
BUSYBOX_VERSION   ?= 1.36.1
BCACHEFS_VERSION  ?= v1.11.0
KERNEL_VERSION    ?= 6.12.6

# Kernel build inputs
KERNEL_SRC ?=
CMDLINE    ?= console=tty0

# Logo colors (override for theming)
LOGO_BG     ?=
LOGO_FG     ?=
LOGO_ACCENT ?=

# Optional inputs
EXTRA_CONFIG ?=
STATIC_JSON  ?=
EXTRA_DIR    ?=
RESCUE_SHELL ?=

CARGO_FEATURES ?=

RUST_SOURCES = $(shell find crates -name '*.rs' -o -name 'Cargo.toml')

# ─── Binary targets ──────────────────────────────────────────────────

.PHONY: x86_64 aarch64 all test clean logo initrd kernel uki busybox bcachefs-tools

x86_64: $(X86_64_BIN)
aarch64: $(AARCH64_BIN)
all: x86_64 aarch64

$(X86_64_BIN): Cargo.toml Cargo.lock $(RUST_SOURCES)
	RUSTFLAGS="$(RUSTFLAGS_STATIC)" $(CARGO) build --release --target $(X86_64_TARGET) $(if $(CARGO_FEATURES),--features $(CARGO_FEATURES))

$(AARCH64_BIN): Cargo.toml Cargo.lock $(RUST_SOURCES)
	RUSTFLAGS="$(RUSTFLAGS_STATIC)" $(CARGO) build --release --target $(AARCH64_TARGET) $(if $(CARGO_FEATURES),--features $(CARGO_FEATURES))

test:
	$(CARGO) test --target $(X86_64_TARGET)

# ─── Component builds (from source) ─────────────────────────────────

busybox: $(BUILD_DIR)/busybox

$(BUILD_DIR)/busybox: scripts/mkbusybox.sh uki/initrd/busybox.config | $(BUILD_DIR)
	ARCH=$(ARCH) BUSYBOX_VERSION=$(BUSYBOX_VERSION) BUILD_DIR=$(BUILD_DIR) OUTPUT=$@ ./scripts/mkbusybox.sh

bcachefs-tools: $(BUILD_DIR)/bcachefs

$(BUILD_DIR)/bcachefs: scripts/mkbcachefs.sh | $(BUILD_DIR)
	ARCH=$(ARCH) BCACHEFS_VERSION=$(BCACHEFS_VERSION) BUILD_DIR=$(BUILD_DIR) OUTPUT=$@ ./scripts/mkbcachefs.sh

# ─── UKI build targets ───────────────────────────────────────────────

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

logo: $(LOGO_PPM)

$(LOGO_PPM): scripts/mklogo.sh | $(BUILD_DIR)
	$(if $(LOGO_BG),LOGO_BG="$(LOGO_BG)" )$(if $(LOGO_FG),LOGO_FG="$(LOGO_FG)" )$(if $(LOGO_ACCENT),LOGO_ACCENT="$(LOGO_ACCENT)" )./scripts/mklogo.sh > $(LOGO_PPM)

initrd: $(INITRD)

$(INITRD): $(BIN) scripts/mkinitrd.sh | $(BUILD_DIR)
	KEXEC_MENU=$(BIN) BUSYBOX=$(BUSYBOX) CRYPTSETUP=$(CRYPTSETUP) BCACHEFS=$(BCACHEFS) \
	$(if $(STATIC_JSON),STATIC_JSON=$(STATIC_JSON) )$(if $(EXTRA_DIR),EXTRA_DIR=$(EXTRA_DIR) )$(if $(filter 1,$(RESCUE_SHELL)),RESCUE_SHELL=1 )OUTPUT=$(INITRD) ./scripts/mkinitrd.sh

kernel: $(KERNEL_OUT)

$(KERNEL_OUT): $(INITRD) $(LOGO_PPM) scripts/mkkernel.sh | $(BUILD_DIR)
	$(if $(KERNEL_SRC),KERNEL_SRC=$(KERNEL_SRC) )KERNEL_VERSION=$(KERNEL_VERSION) ARCH=$(ARCH) \
	BUILD_DIR=$(BUILD_DIR) INITRAMFS=$(INITRD) CMDLINE="$(CMDLINE)" LOGO=$(LOGO_PPM) \
	$(if $(EXTRA_CONFIG),EXTRA_CONFIG=$(EXTRA_CONFIG) )OUTPUT=$(KERNEL_OUT) ./scripts/mkkernel.sh

uki: $(UKI_OUT)

$(UKI_OUT): $(KERNEL_OUT) | $(BUILD_DIR)
	cp $(KERNEL_OUT) $(UKI_OUT)
	@echo "UKI built: $(UKI_OUT)"

# ─── Cleanup ─────────────────────────────────────────────────────────

clean:
	$(CARGO) clean
	rm -rf $(BUILD_DIR)
