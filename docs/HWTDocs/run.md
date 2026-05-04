```
ts=$(date '+%m%d_%H%M%S'); make all && qemu-system-riscv64 -machine virt -kernel kernel-rv -m 1G -nographic -smp 1 -bios default -drive file=sdcard-rv.img,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 -no-reboot -device virtio-net-device,netdev=net -netdev user,id=net -rtc base=utc > "docs/HWTDocs/logs/allrv_full_${ts}.log" 2> "docs/HWTDocs/rvlogs/allrv_full_${ts}.err"

```

```
SINGLE_TEST=all LTP_START_FROM=waitpid10 LOG=OFF OFFLINE=1 CARGO_NET_OFFLINE=true \
timeout 360s bash run.sh -f sdcard-rv.img -t rv \
> docs/HWTDocs/logs/switch.log
```

```
cd /home/hwt/桌面/sources/实验和项目类/os区域赛/rcore-lab
ts=$(date +%m%d_%H%M%S)
LOG=OFF timeout 600s bash run-la.sh -t all --no-data-disk \
  > "docs/HWTDocs/lalogs/la_full_${ts}.log" \
  2> "docs/HWTDocs/lalogs/la_full_${ts}.err"

```
