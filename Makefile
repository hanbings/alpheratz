ARCH    ?= x86_64
PROFILE ?= debug

ifeq ($(PROFILE),release)
  CARGO_FLAGS := --release
else
  CARGO_FLAGS :=
endif

# Paths (before arch dispatch so $(DISK_IMG) etc. are available)

OUT_DIR  := target/$(ARCH)
EFI      := $(OUT_DIR)/alpheratz.efi
DISK_IMG := $(OUT_DIR)/disk.img
VARS     := $(OUT_DIR)/vars.fd

# Architecture

ifeq ($(ARCH),x86_64)
  RUST_TARGET  := x86_64-unknown-uefi
  CARGO_BIN    := target/$(RUST_TARGET)/$(PROFILE)/alpheratz.efi
  BOOT_EFI     := BOOTX64.EFI
  QEMU         := qemu-system-x86_64
  QEMU_MACHINE := -machine q35
  QEMU_MEM     := 256M
  QEMU_DRIVE   := -drive file=$(DISK_IMG),format=raw
  QEMU_NET     := -device e1000,netdev=net0 -netdev user,id=net0
  FIRMWARE     := /usr/share/OVMF/OVMF_CODE_4M.fd
  VARS_TMPL    := /usr/share/OVMF/OVMF_VARS_4M.fd

else ifeq ($(ARCH),aarch64)
  RUST_TARGET  := aarch64-unknown-uefi
  CARGO_BIN    := target/$(RUST_TARGET)/$(PROFILE)/alpheratz.efi
  BOOT_EFI     := BOOTAA64.EFI
  QEMU         := qemu-system-aarch64
  QEMU_MACHINE := -machine virt -cpu cortex-a72
  QEMU_MEM     := 256M
  QEMU_DRIVE   := -drive file=$(DISK_IMG),format=raw,if=virtio
  QEMU_NET     := -device virtio-net-device,netdev=net0 -netdev user,id=net0
  FIRMWARE     := /usr/share/AAVMF/AAVMF_CODE.fd
  VARS_TMPL    := /usr/share/AAVMF/AAVMF_VARS.fd

else ifeq ($(ARCH),riscv64)
  RUST_TARGET  := riscv64gc-unknown-none-elf
  CARGO_BIN    := target/$(RUST_TARGET)/$(PROFILE)/alpheratz
  BOOT_EFI     := BOOTRISCV64.EFI
  OBJCOPY      := riscv64-linux-gnu-objcopy
  PE_FORMAT    := pei-riscv64-little
  QEMU         := qemu-system-riscv64
  QEMU_MACHINE := -machine virt
  QEMU_MEM     := 256M
  QEMU_DRIVE   := -drive file=$(DISK_IMG),format=raw,if=virtio
  QEMU_NET     := -device virtio-net-device,netdev=net0 -netdev user,id=net0
  FIRMWARE     := /usr/share/qemu-efi-riscv64/RISCV_VIRT_CODE.fd
  VARS_TMPL    := /usr/share/qemu-efi-riscv64/RISCV_VIRT_VARS.fd

else ifeq ($(ARCH),loongarch64)
  RUST_TARGET  := loongarch64-unknown-none
  CARGO_BIN    := target/$(RUST_TARGET)/$(PROFILE)/alpheratz
  BOOT_EFI     := BOOTLOONGARCH64.EFI
  OBJCOPY      := loongarch64-linux-gnu-objcopy
  PE_FORMAT    := pei-loongarch64
  QEMU         := qemu-system-loongarch64
  QEMU_MACHINE := -machine virt
  QEMU_MEM     := 2G
  QEMU_DRIVE   := -drive file=$(DISK_IMG),format=raw,if=virtio
  QEMU_NET     := -device virtio-net-pci,netdev=net0 -netdev user,id=net0
  FIRMWARE     := /usr/share/qemu-efi-loongarch64/QEMU_EFI.fd
  VARS_TMPL    := /usr/share/qemu-efi-loongarch64/QEMU_VARS.fd

else
  $(error Unsupported ARCH=$(ARCH). Use x86_64, aarch64, riscv64, or loongarch64)
endif

# Targets

.PHONY: all build efi disk run clean

all: efi

$(OUT_DIR):
	mkdir -p $(OUT_DIR)

build:
	cargo build --target $(RUST_TARGET) $(CARGO_FLAGS)

efi: build | $(OUT_DIR)
ifneq ($(OBJCOPY),)
	$(OBJCOPY) --remove-section=.rela.text --remove-section=.rela.data \
		--remove-section=.rela.rodata --subsystem efi-app \
		-O $(PE_FORMAT) $(CARGO_BIN) $(EFI)
	python3 gen-reloc.py $(CARGO_BIN) $(EFI)
else
	cp $(CARGO_BIN) $(EFI)
endif

disk: efi
	dd if=/dev/zero of=$(DISK_IMG) bs=1M count=64
	/sbin/mkfs.vfat -F 32 $(DISK_IMG)
	mmd -i $(DISK_IMG) ::EFI
	mmd -i $(DISK_IMG) ::EFI/BOOT
	mcopy -i $(DISK_IMG) $(EFI) ::EFI/BOOT/$(BOOT_EFI)
	mcopy -i $(DISK_IMG) example.toml ::EFI/BOOT/bootloader.toml

run: disk
	cp $(VARS_TMPL) $(VARS)
	$(QEMU) \
		$(QEMU_MACHINE) \
		-m $(QEMU_MEM) \
		-nographic \
		-drive if=pflash,format=raw,file=$(FIRMWARE),readonly=on \
		-drive if=pflash,format=raw,file=$(VARS) \
		$(QEMU_DRIVE) \
		$(QEMU_NET)

clean:
	cargo clean
	rm -rf target/x86_64 target/aarch64 target/riscv64 target/loongarch64
