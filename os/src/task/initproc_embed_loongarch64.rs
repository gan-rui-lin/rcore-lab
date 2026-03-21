pub(super) const INITPROC_EMBED: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../user/build-user/elf/initcode.elf"
));
