# Building
TARGET_RV := riscv64gc-unknown-none-elf
TARGET_LA := loongarch64-unknown-none
MODE := release
FEATURES ?= ext4
LOG ?= TRACE
KERNEL_RV_ELF := target/$(TARGET_RV)/$(MODE)/os
KERNEL_RV_BIN := $(KERNEL_RV_ELF).bin
KERNEL_LA_ELF := target/$(TARGET_LA)/$(MODE)/os
KERNEL_RV := kernel-rv
KERNEL_LA := kernel-la
DISASM_TMP := target/$(TARGET_RV)/$(MODE)/asm
FS_IMG := ../user/target/$(TARGET_RV)/$(MODE)/fs.img
APPS := ../user/src/bin/*
OFFLINE :=

# BOARD
BOARD := qemu
SBI ?= rustsbi
BOOTLOADER := ../bootloader/$(SBI)-$(BOARD).bin
SBI_QEMU := sbi-qemu
SBI_LOAD_ADDR ?= 0x80000000

# Building mode argument
ifeq ($(MODE), release)
	MODE_ARG := --release
endif

# KERNEL ENTRY
KERNEL_ENTRY_PA := 0x80200000

# Binutils
OBJDUMP := rust-objdump --arch-name=riscv64
OBJCOPY := rust-objcopy --binary-architecture=riscv64
OBJCOPY_ELF := rust-objcopy

CHAPTER ?= $(shell git rev-parse --abbrev-ref HEAD | sed -nE 's/^ch([0-9]+).*$$/\1/p')
CHAPTER := $(if $(CHAPTER),$(CHAPTER),0)
TEST ?= $(CHAPTER)
BASE ?= 1

# Disassembly
DISASM ?= -x

# Run usertests or usershell
TEST ?=

build: env $(KERNEL_RV_BIN) fs-img 

rv: env kernel-rv $(KERNEL_RV_BIN) sbi-qemu
la: env-la kernel-la
all: rv la

env:
ifeq ($(OFFLINE),)
	(rustup target list | grep "riscv64gc-unknown-none-elf (installed)") || rustup target add $(TARGET_RV)
	cargo install cargo-binutils
	rustup component add rust-src
	rustup component add llvm-tools-preview
endif

env-la:
ifeq ($(OFFLINE),)
	(rustup target list | grep "loongarch64-unknown-none (installed)") || rustup +nightly-2024-08-01 target add $(TARGET_LA)
endif

$(KERNEL_RV_BIN): kernel
	@$(OBJCOPY) $(KERNEL_RV_ELF) --strip-all -O binary $@

kernel-rv: kernel
	@cp $(KERNEL_RV_ELF) $(KERNEL_RV)

kernel-la: env-la
	@make -C ../user build ARCH=la MODE=$(MODE) TEST=$(TEST) CHAPTER=$(CHAPTER) BASE=$(BASE)
	@echo Platform: loongarch64
	@LOG=$(LOG) cargo +nightly-2024-08-01 build $(MODE_ARG) --target $(TARGET_LA) --features $(FEATURES)
	@cp $(KERNEL_LA_ELF) $(KERNEL_LA)

sbi-qemu:
	@$(OBJCOPY_ELF) -I binary -O elf64-littleriscv -B riscv:rv64 \
		--rename-section .data=.text,alloc,code,load \
		--change-section-vma .data=$(SBI_LOAD_ADDR) \
		$(BOOTLOADER) $(SBI_QEMU) 2>/dev/null || \
	$(OBJCOPY_ELF) -I binary -O elf64-littleriscv -B riscv:rv64 \
		$(BOOTLOADER) $(SBI_QEMU) || \
	cp $(BOOTLOADER) $(SBI_QEMU)

fs-img: $(APPS)
	@make -C ../user build MODE=$(MODE) TEST=$(TEST) CHAPTER=$(CHAPTER) BASE=$(BASE)
	@rm -f $(FS_IMG)
	@cd ../easy-fs-fuse && cargo run --release -- -s ../user/build/app/ -t ../user/target/riscv64gc-unknown-none-elf/release/

kernel:
	@make -C ../user build MODE=$(MODE) TEST=$(TEST) CHAPTER=$(CHAPTER) BASE=$(BASE)
	@echo Platform: $(BOARD)
	@LOG=$(LOG) cargo build $(MODE_ARG) --features $(FEATURES)

clean:
	@cargo clean

disasm: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_RV_ELF) | less

disasm-vim: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_RV_ELF) > $(DISASM_TMP)
	@vim $(DISASM_TMP)
	@rm $(DISASM_TMP)

run: build run-inner

run-inner:
	@qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_RV_BIN),addr=$(KERNEL_ENTRY_PA) \
		-drive file=$(FS_IMG),if=none,format=raw,id=x0 \
        -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0

debug: build
	@tmux new-session -d \
		"qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_RV_BIN),addr=$(KERNEL_ENTRY_PA) -s -S" && \
		tmux split-window -h "riscv64-unknown-elf-gdb -ex 'file $(KERNEL_RV_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'" && \
		tmux -2 attach-session -d


gdbserver: build
	@qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_RV_BIN),addr=$(KERNEL_ENTRY_PA) -s -S

gdbclient:
	@riscv64-unknown-elf-gdb -ex 'file $(KERNEL_RV_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'

.PHONY: all rv la build env env-la kernel kernel-rv kernel-la clean disasm disasm-vim run-inner fs-img gdbserver gdbclient sbi-qemu
