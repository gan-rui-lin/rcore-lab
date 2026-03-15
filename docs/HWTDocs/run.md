# 项目运行命令

本文补充当前仓库中实际可用的 RISC-V 运行命令，重点覆盖构建、启动 QEMU 和运行当前内置的 LTP 用例。

## 1. 构建内核

当前 `os/Makefile` 中写死了 `OFFLINE :=`，因此如果本机不希望在构建时访问网络，不能只写 `OFFLINE=1 bash run.sh ...`，需要直接给 `make` 传参：

```bash
make OFFLINE=1 rv
```

这条命令会完成：

- 构建用户态程序
- 构建 RISC-V 内核 `kernel-rv`
- 生成 `sbi-qemu`

如果本机网络正常，也可以直接使用仓库脚本：

```bash
bash run.sh -f sdcard-rv.img -t rv
```

## 2. 启动 QEMU

在已经完成 `make OFFLINE=1 rv` 之后，可直接手动启动 QEMU：

```bash
qemu-system-riscv64 \
  -machine virt \
  -kernel kernel-rv \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net
```

如果希望限制最长运行时间，避免终端一直挂住，可以加 `timeout`：

```bash
timeout 240 qemu-system-riscv64 \
  -machine virt \
  -kernel kernel-rv \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net 2>&1
```

## 3. 一条命令完成构建和运行

离线环境下建议分两步执行；如果想合并成连续的两条命令，可以这样跑：

```bash
make OFFLINE=1 rv
timeout 240 qemu-system-riscv64 \
  -machine virt \
  -kernel kernel-rv \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net 2>&1
```

## 4. 当前 LTP 运行方式

当前仓库中的 `user/src/bin/initcode.rs` 已经默认启用 LTP：

- `ENABLE_LTP_TEST = true`
- `LTP_TESTS` 当前仅包含 `lseek01`

因此只要按上面的命令启动系统，开机后就会自动执行当前配置的 LTP 用例。

本仓库当前实际验证通过的一轮 LTP 运行命令如下：

```bash
make OFFLINE=1 rv
qemu-system-riscv64 \
  -machine virt \
  -kernel kernel-rv \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net
```

预期看到的关键输出为：

```text
=== LTP Test Start ===
[LTP] RUN  lseek01
...
[LTP] PASS lseek01
=== LTP Test End ===
Total: 1, Passed: 1, Failed: 0 (Timeout: 0)
```

## 5. 常用补充命令

如果要查看当前 `initcode` 里到底启用了哪些测试：

```bash
sed -n '1,120p' user/src/bin/initcode.rs
sed -n '276,340p' user/src/bin/initcode.rs
```

如果想只看某个测试相关输出，推荐把运行结果保存到文件后再筛选：

```bash
timeout 240 qemu-system-riscv64 \
  -machine virt \
  -kernel kernel-rv \
  -m 128M \
  -nographic \
  -smp 1 \
  -bios default \
  -drive file=sdcard-rv.img,if=none,format=raw,id=x0 \
  -device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0 \
  -device virtio-net-device,netdev=net,bus=virtio-mmio-bus.1 \
  -netdev user,id=net 2>&1 | tee /tmp/rcore-ltp.log
```

```bash
rg -n "\\[LTP\\]|TPASS|TFAIL|TBROK|Summary:" /tmp/rcore-ltp.log
```
