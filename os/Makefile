# Building
TARGET := riscv64gc-unknown-none-elf
MODE := release
FEATURES ?= ext4
LOG ?= OFF
KERNEL_ELF := target/$(TARGET)/$(MODE)/os
KERNEL_BIN := $(KERNEL_ELF).bin
KERNEL_QEMU := kernel-qemu
DISASM_TMP := target/$(TARGET)/$(MODE)/asm
FS_IMG := ../user/target/$(TARGET)/$(MODE)/fs.img
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

CHAPTER ?= $(shell git rev-parse --abbrev-ref HEAD | sed -nE 's/^ch([0-9]+).*$/\1/p')
CHAPTER := $(if $(CHAPTER),$(CHAPTER),0)
TEST ?= $(CHAPTER)
BASE ?= 1

# Disassembly
DISASM ?= -x

# Run usertests or usershell
TEST ?=

build: env $(KERNEL_BIN) fs-img 

all: env kernel-qemu sbi-qemu

env:
ifeq ($(OFFLINE),)
	(rustup target list | grep "riscv64gc-unknown-none-elf (installed)") || rustup target add $(TARGET)
	cargo install cargo-binutils
	rustup component add rust-src
	rustup component add llvm-tools-preview
endif

$(KERNEL_BIN): kernel
	@$(OBJCOPY) $(KERNEL_ELF) --strip-all -O binary $@

kernel-qemu: kernel
	@cp $(KERNEL_ELF) $(KERNEL_QEMU)

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
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) | less

disasm-vim: kernel
	@$(OBJDUMP) $(DISASM) $(KERNEL_ELF) > $(DISASM_TMP)
	@vim $(DISASM_TMP)
	@rm $(DISASM_TMP)

run: build run-inner

run-inner:
	@qemu-system-riscv64 \
		-machine virt \
		-nographic \
		-bios $(BOOTLOADER) \
		-device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) \
		-drive file=$(FS_IMG),if=none,format=raw,id=x0 \
        -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0

debug: build
	@tmux new-session -d \
		"qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) -s -S" && \
		tmux split-window -h "riscv64-unknown-elf-gdb -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'" && \
		tmux -2 attach-session -d


gdbserver: build
	@qemu-system-riscv64 -machine virt -nographic -bios $(BOOTLOADER) -device loader,file=$(KERNEL_BIN),addr=$(KERNEL_ENTRY_PA) -s -S

gdbclient:
	@riscv64-unknown-elf-gdb -ex 'file $(KERNEL_ELF)' -ex 'set arch riscv:rv64' -ex 'target remote localhost:1234'

.PHONY: all build env kernel clean disasm disasm-vim run-inner fs-img gdbserver gdbclient kernel-qemu sbi-qemu
