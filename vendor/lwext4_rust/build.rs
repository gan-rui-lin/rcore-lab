use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR missing"));
    let c_path = manifest_dir
        .join("c/lwext4")
        .canonicalize()
        .expect("cannot canonicalize path");

    let lwext4_make = manifest_dir.join("c/lwext4/toolchain/musl-generic.cmake");
    let lwext4_patch = manifest_dir
        .join("c/lwext4-make.patch")
        .canonicalize()
        .unwrap();

    if !lwext4_make.exists() {
        println!("Retrieve lwext4 source code");
        let git_status = Command::new("git")
            .args(&["submodule", "update", "--init", "--recursive"])
            .status()
            .expect("failed to execute process: git submodule");
        assert!(git_status.success());

        println!("To patch lwext4 src");
        Command::new("git")
            .args(&["apply", lwext4_patch.to_str().unwrap()])
            .current_dir(c_path.clone())
            .spawn()
            .expect("failed to execute process: git apply patch");

        fs::copy(
            manifest_dir.join("c/musl-generic.cmake"),
            manifest_dir.join("c/lwext4/toolchain/musl-generic.cmake"),
        )
        .unwrap();
    }

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let lwext4_lib = &format!("lwext4-{}", arch);
    let lwext4_lib_path = manifest_dir.join(format!("c/lwext4/lib{}.a", lwext4_lib));
    let force_rebuild_lwext4 = env::var("LWEXT4_BLOCK_DEV_CACHE_SIZE").is_ok();
    if force_rebuild_lwext4 || !lwext4_lib_path.exists() {
        let status = Command::new("make")
            .args(&[
                "musl-generic",
                "-C",
                c_path.to_str().expect("invalid path of lwext4"),
            ])
            .arg(&format!("ARCH={}", arch))
            .status()
            .expect("failed to execute process: make lwext4");
        assert!(status.success());

        if !manifest_dir.join("src/bindings.rs").exists() {
            let cc = &format!("{}-linux-musl-gcc", arch);
            let output = Command::new(cc)
                .args(["-print-sysroot"])
                .output()
                .expect("failed to execute process: gcc -print-sysroot");

            let sysroot = core::str::from_utf8(&output.stdout).unwrap();
            let sysroot = sysroot.trim_end();
            let sysroot_inc = &format!("-I{}/include/", sysroot);

            generates_bindings_to_rust(&manifest_dir, sysroot_inc);
        }
    }

    /* No longer need to implement the libc.a
    let libc_name = &format!("c-{}", arch);
    let libc_dir = env::var("LIBC_BUILD_TARGET_DIR").unwrap_or(String::from("./"));
    let libc_dir = PathBuf::from(libc_dir)
        .canonicalize()
        .expect("cannot canonicalize LIBC_BUILD_TARGET_DIR");

    println!("cargo:rustc-link-lib=static={libc_name}");
    println!(
        "cargo:rustc-link-search=native={}",
        libc_dir.to_str().unwrap()
    );
    */

    println!("cargo:rustc-link-lib=static={lwext4_lib}");
    println!(
        "cargo:rustc-link-search=native={}",
        c_path.to_str().unwrap()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("c/wrapper.h").to_str().unwrap()
    );
    println!("cargo:rerun-if-changed={}", c_path.to_str().unwrap());
    println!("cargo:rerun-if-env-changed=LWEXT4_BLOCK_DEV_CACHE_SIZE");
}

fn generates_bindings_to_rust(manifest_dir: &Path, mpath: &str) {
    let bindings = bindgen::Builder::default()
        .use_core()
        // The input header we would like to generate bindings for.
        .header(
            manifest_dir
                .join("c/wrapper.h")
                .to_str()
                .expect("invalid wrapper.h path"),
        )
        //.clang_arg("--sysroot=/path/to/sysroot")
        .clang_arg(mpath)
        //.clang_arg("-I../../ulib/axlibc/include")
        .clang_arg(format!(
            "-I{}",
            manifest_dir
                .join("c/lwext4/include")
                .to_str()
                .expect("invalid include path")
        ))
        .clang_arg(format!(
            "-I{}",
            manifest_dir
                .join("c/lwext4/build_musl-generic/include/")
                .to_str()
                .expect("invalid build include path")
        ))
        .layout_tests(false)
        // Tell cargo to invalidate the built crate whenever any of the included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Finish the builder and generate the bindings.
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = manifest_dir.join("src");
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
