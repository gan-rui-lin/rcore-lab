```
timeout 60 qemu-system-riscv64 \
  -machine virt \
  -nographic \
  -bios bootloader/rustsbi-qemu.bin \
  -device loader,file=kernel-rv.bin,addr=0x80200000 \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net 2>&1 | tee /tmp/rcore-ltp.log

```

```
rg -n "\\[LTP\\] (RUN|PASS|FAIL|TIMEOUT)|=== LTP Test|Total:|TPASS|TFAIL|TBROK" /tmp/rcore-ltp.log

```

```bash
python3 tools/ltp_log_summary.py /tmp/rcore-ltp.log
```

```bash
python3 tools/extract_oskernel2025_ltp_cases.py --check
LTP_PROFILE=oskernel2025-riscv OFFLINE=1 make rv
```
