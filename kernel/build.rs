fn main() {
    println!("cargo:rustc-link-arg=-Tlinker.ld");
    println!("cargo:rerun-if-changed=linker.ld");
}
