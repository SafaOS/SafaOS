use std::{
    fs,
    process::{Command, Stdio},
};
use tar;

const ISO_PATH: &str = "safaos.iso";
// (dir relative from build.rs, dir in ramdisk)
// or (file relative from build.rs, path in ramdisk)
const RAMDISK_CONTENT: &[(&str, &str)] = &[
    ("bin/zig-out/bin/", "bin"),
    ("Shell/zig-out/bin/Shell", "bin/Shell"),
    ("TestBot/zig-out/bin/TestBot", "bin/TestBot"),
    ("ramdisk-include/", ""),
];

fn cleanup() {
    let _ = fs::remove_dir_all("iso_root");
}

// TODO: consider moving to shell scripts
// it might be more convenient to use rust + cargo for managing release builds vs debug builds and
// etc

pub fn build() {
    cleanup();
    println!("please ensure that you are in the root of the SafaOS repository");
    println!("Building a SafaOS image to ./safaos.iso...");
    submodules_init();
    build_limine();
    setup_iso_root();
    put_kernel_img();
    put_limine();
    build_ramdisk();
    make_iso();
}

fn make_iso() {
    println!("Archiving iso...");
    execute(&format!(
        "xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
            -no-emul-boot -boot-load-size 4 -boot-info-table \
            --efi-boot boot/limine/limine-uefi-cd.bin \
            -efi-boot-part --efi-boot-image --protective-msdos-label \
            iso_root -o {ISO_PATH}"
    ));
}

fn build_ramdisk() {
    println!("Building ramdisk...");
    compile_ramdisk_contents();
    println!("Archiving ramdisk...");
    // TODO: clean up code
    let file = fs::File::create("iso_root/boot/ramdisk.tar").unwrap();
    let mut tar_builder = tar::Builder::new(file);

    let mut added_dirs = std::collections::HashSet::<&std::path::Path>::new();

    for (src, dest) in RAMDISK_CONTENT {
        let (src, dest) = (std::path::Path::new(src), std::path::Path::new(dest));
        if src.is_file() {
            if let Some(parent) = dest.parent() {
                if !added_dirs.contains(parent) {
                    let mut empty_header = tar::Header::new_ustar();
                    empty_header.set_path(parent).unwrap();
                    empty_header.set_entry_type(tar::EntryType::Directory);
                    empty_header.set_size(0);
                    empty_header.set_cksum();

                    tar_builder.append(&empty_header, std::io::empty()).unwrap();
                    added_dirs.insert(parent);
                }
            }
            tar_builder
                .append_file(
                    dest,
                    &mut fs::File::open(src)
                        .expect("ramdisk contents corrupt file missing, edit RAMDISK_CONTENT"),
                )
                .unwrap();
        } else if src.is_dir() {
            added_dirs.insert(dest);
            tar_builder.append_dir_all(dest, src).unwrap();
        } else {
            panic!(
                "ramdisk content is nethier a file nor directory (or doesn't exists), edit RAMDISK_CONTENT"
            );
        }
    }
    tar_builder.finish().unwrap();
}

fn compile_ramdisk_contents() {
    println!("Compiling ramdisk contents...");
    execute_at("Shell", "zig build");
    execute_at("bin", "zig build");
    execute_at("TestBot", "zig build");
}

fn put_limine() {
    println!("Putting limine...");
    execute(
        "cp -v limine.conf limine/limine-bios.sys limine/limine-bios-cd.bin limine/limine-uefi-cd.bin iso_root/boot/limine",
    );

    execute("cp -v limine/BOOTX64.EFI limine/BOOTIA32.EFI iso_root/EFI/BOOT");
}

fn put_kernel_img() {
    println!("Putting kernel image...");
    let path = build_with_cargo("kernel", "--features test");
    execute(&format!("cp -v {path} iso_root/boot/kernel"));
}

fn setup_iso_root() {
    println!("Setting up iso root...");
    // using `mkdir` instead of `fs::create_dir_all` for better logging (see `execute`)
    // peformance doesn't matter here
    execute("mkdir -p iso_root/boot/limine");
    execute("mkdir -p iso_root/EFI/BOOT");
}

fn submodules_init() {
    execute("git submodule update --init --recursive");
}

fn build_limine() {
    println!("Building limine...");
    if !fs::exists("limine").is_ok_and(|e| e) {
        execute(
            "git clone https://github.com/limine-bootloader/limine.git --branch=v8.x-binary --depth=1",
        );
    }

    execute("make -C limine");
}

/// Builds the crate at `at` as an executable using cargo with args `args` and returns the path to the executable as a string
fn build_with_cargo(at: &'static str, args: &str) -> String {
    execute_at(
        at,
        &format!(
            r#"json=$(cargo build {args} --message-format=json-render-diagnostics)
            printf "%s" "$json" | jq -js '[.[] | select(.reason == "compiler-artifact") | select(.executable != null)] | last | .executable'"#
        ),
    )
}

fn execute(command: &str) -> String {
    println!("Executing: `{}`", command);
    // FIXME: use cmd on windows #9
    let results = Command::new("bash")
        .arg("-c")
        .arg(command)
        .stderr(Stdio::inherit())
        .output()
        .expect("failed to execute bash");
    if !results.status.success() {
        panic!("Command failed ({}): {}", command, results.status);
    }

    String::from_utf8_lossy(&results.stdout).into_owned()
}

fn execute_at(dir: &'static str, command: &str) -> String {
    execute(&format!("cd {dir} && {command}"))
}
