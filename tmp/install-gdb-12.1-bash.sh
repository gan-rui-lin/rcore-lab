#!/usr/bin/env bash
set -euo pipefail

# Build/install GDB 12.1 for loongarch64 and riscv64 targets into /usr/bin/gdb-12.1
# Run this script step by step (copy/paste each block) if you prefer.

# 0) Prereqs (expect is required by GDB build)
sudo apt-get update
sudo apt-get install -y build-essential expect texinfo bison flex python3

# 1) Fetch source
cd /tmp
wget http://ftp.gnu.org/gnu/gdb/gdb-12.1.tar.gz
rm -rf gdb-12.1
 tar -zxvf gdb-12.1.tar.gz

# 2) Build loongarch64 target GDB
cd gdb-12.1
rm -rf build-la64
mkdir build-la64
cd build-la64
../configure --prefix=/usr/bin/gdb-12.1 \
  --target=loongarch64-unknown-linux-gnu \
  --program-suffix=-loongarch64-unknown-linux-gnu
make -j16
sudo make install

# 3) Build riscv64 target GDB
cd ..
rm -rf build-rv64
mkdir build-rv64
cd build-rv64
../configure --prefix=/usr/bin/gdb-12.1 \
  --target=riscv64-unknown-linux-gnu \
  --program-suffix=-riscv64-unknown-linux-gnu
make -j16
sudo make install

# 如果是 bash 用户，应该写入 ~/.bashrc 而不是 ~/.zshrc
# 4) Add PATH to ~/.bashrc (so the new GDBs are found)
if ! grep -q '/usr/bin/gdb-12.1/bin' ~/.bashrc; then
  echo 'export PATH=/usr/bin/gdb-12.1/bin:$PATH' >> ~/.bashrc
fi

source ~/.bashrc

echo 'Done. New binaries should be:'
ls -l /usr/bin/gdb-12.1/bin/gdb-*-unknown-linux-gnu
