```bash
timeout 240 qemu-system-riscv64 -machine virt -kernel kernel-rv -m 128M -nographic -smp 1 -bios default -drive file=sdcard-rv.img,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 -netdev user,id=net 2>&1

```
