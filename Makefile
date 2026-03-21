DOCKER_NAME ?= rcore-docker
.PHONY: docker build_docker rv la all debug clean
	
docker:
	docker run --network host --rm -it -v ${PWD}:/mnt -w /mnt ${DOCKER_NAME} bash

build_docker: 
	docker build -t ${DOCKER_NAME} .

fmt:
	cd easy-fs; cargo fmt; cd ../easy-fs-fuse cargo fmt; cd ../os ; cargo fmt; cd ../user; cargo fmt; cd ..

rv:
	@make -C os LOG=$(LOG) OFFLINE=$(OFFLINE) rv
	@cp os/kernel-rv kernel-rv
	@cp os/kernel-rv kernel-rv.elf
	@cp os/target/riscv64gc-unknown-none-elf/release/os.bin kernel-rv.bin
	@cp os/sbi-qemu sbi-qemu

la:
	@make -C os LOG=$(LOG) OFFLINE=$(OFFLINE) la
	@cp os/kernel-la kernel-la

all: rv la

debug:
	@make -C os MODE=debug LOG=$(LOG) OFFLINE=$(OFFLINE) rv
	@cp os/kernel-rv kernel-rv
	@cp os/kernel-rv kernel-rv.elf
	@cp os/target/riscv64gc-unknown-none-elf/debug/os.bin kernel-rv.bin
	@cp os/sbi-qemu sbi-qemu

clean:
	@make -C os clean
	@make -C user clean
	@rm -f kernel-rv kernel-rv.elf kernel-rv.bin kernel-la sbi-qemu
