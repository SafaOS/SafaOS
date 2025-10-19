fn main() {
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("failed to get target arch");
    println!("cargo:rustc-link-arg=-Tlinker-{target_arch}.ld");
    println!("cargo:rerun-if-changed=linker-{target_arch}.ld");
}
