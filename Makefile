DOCKER_NAME ?= rcore-docker
.PHONY: docker build_docker all run
	
docker:
	docker run --network host --rm -it -v ${PWD}:/mnt -w /mnt ${DOCKER_NAME} bash

build_docker: 
	docker build -t ${DOCKER_NAME} .

fmt:
	cd easy-fs; cargo fmt; cd ../easy-fs-fuse cargo fmt; cd ../os ; cargo fmt; cd ../user; cargo fmt; cd ..

all:
	@make -C os LOG=$(LOG) all
	@cp os/kernel-qemu kernel-qemu
	@cp os/sbi-qemu sbi-qemu

debug:
	@make -C os MODE=debug LOG=$(LOG) kernel-qemu sbi-qemu
	@cp os/kernel-qemu kernel-qemu
	@cp os/sbi-qemu sbi-qemu

clean:
	@make -C os clean
	@make -C user clean
	@rm -f kernel-qemu sbi-qemu
