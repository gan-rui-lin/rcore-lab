import os

base_address = 0x80400000
step = 0x20000
linker = "src/linker.ld"

app_id = 0
build_dir = os.getenv("BUILD_DIR", default="build-user")
apps = os.listdir(f"{build_dir}/app")
apps.sort()
chapter = os.getenv("CHAPTER")
mode = os.getenv("MODE", default = "release")
arch = os.getenv("ARCH", default = "rv")
if arch == "la":
    target = "loongarch64-unknown-none"
else:
    target = "riscv64gc-unknown-none-elf"
cargo_target_dir = os.getenv("CARGO_TARGET_DIR", default="target-user")
if mode == "release" :
	mode_arg = "--release"
else :
    mode_arg = ""

for app in apps:
    app = app[: app.find(".")]
    os.system(
        "CARGO_TARGET_DIR=%s cargo rustc --bin %s %s --target %s -- -Clink-args=-Ttext=%x"
        % (cargo_target_dir, app, mode_arg, target, base_address + step * app_id)
    )
    print(
        "[build.py] application %s start with address %s"
        % (app, hex(base_address + step * app_id))
    )
    if chapter == '3':
        app_id = app_id + 1
