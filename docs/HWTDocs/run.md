```
qemu-system-riscv64 -machine virt -kernel {os_file} -m {mem} -nographic -smp {smp} -bios default -drive file={fs},if=none,format=raw,id=x0 \
                    -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 -no-reboot -device virtio-net-device,netdev=net -netdev user,id=net \
                    -rtc base=utc \
                    -drive file=disk.img,if=none,format=raw,id=x1 -device virtio-blk-device,drive=x1,bus=virtio-mmio-bus.1

```

```
ts=$(date '+%m%d_%H%M%S'); make all && qemu-system-riscv64 -machine virt -kernel kernel-rv -m 1G -nographic -smp 1 -bios default -drive file=sdcard-rv.img,if=none,format=raw,id=x0 -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 -no-reboot -device virtio-net-device,netdev=net -netdev user,id=net -rtc base=utc > "docs/HWTDocs/logs/allrv_full_${ts}.log" 2> "docs/HWTDocs/logs/allrv_full_${ts}.err"

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

我刚修复commit冲突后发现ltp测例通过的少了很多从allrv02.log变成了allrv03.log，注意“TPASS”表示通过，"FAIL LTP"只表示某个测例测完并不表示失败，相关逻辑可以看/home/hwt/桌面/sources/实验和项目类/os区域赛/oskernel-testsuits-cooperation/doc/ltp/ltp.md，/home/hwt/桌面/sources/实验和项目类/os区域赛/autotest-for-oskernel/kernel/judge/judge_ltp-musl.py和ltp_testcode.sh,测例逻辑在/home/hwt/桌面/sources/实验和项目类/os区域赛/testsuits-for-oskernel-pre-2025/ltp-full-20240524/testcases下，运行命令为“SINGLE_TEST=glibc-basic LOG=OFF OFFLINE=1 CARGO_NET_OFFLINE=true timeout 180s bash run.sh -f sdcard-rv.img -t rv > allrv01.log”
