// build.rs
use std::{
    collections::HashSet,
    env::current_dir,
    fs::{self, File},
    io::empty,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use tar::{Builder, Header};
const ISO_PATH: &str = "safaos.iso";
// (dir relative from build.rs, dir in ramdisk)
// or (file relative from build.rs, path in ramdisk)
const RAMDISK_CONTENT: &[(&str, &str)] = &[
    ("bin/zig-out/bin/", "bin"),
    ("Shell/zig-out/bin/Shell", "bin/Shell"),
    ("TestBot/zig-out/bin/TestBot", "bin/TestBot"),
    ("ramdisk-include/", ""),
];

fn limine_make() -> Output {
    if !fs::exists("limine").unwrap() {
        Command::new("git")
            .arg("clone")
            .arg("https://github.com/limine-bootloader/limine.git")
            .arg("--branch=v8.x-binary")
            .arg("--depth=1")
            .output()
            .unwrap();
    }

    Command::new("make")
        .arg("-C")
        .arg("limine")
        .output()
        .unwrap()
}

fn out(mut output: Output) {
    output.stdout.append(&mut output.stderr);
    eprintln!("{}", String::from_utf8_lossy(&output.stdout))
}
fn setup_iso_root() {
    fs::create_dir_all("iso_root/boot/limine").unwrap();
    fs::create_dir_all("iso_root/EFI/BOOT").unwrap();
}

fn put_kernel_img() {
    let kernel = PathBuf::from(std::env::var_os("CARGO_BIN_FILE_KERNEL_kernel").unwrap());
    out(Command::new("mv")
        .arg("-v")
        .arg(kernel)
        .arg("iso_root/boot/kernel")
        .output()
        .unwrap());
}

fn put_limine_config() {
    out(Command::new("cp")
        .arg("-v")
        .arg("limine.conf")
        .arg("limine/limine-bios.sys")
        .arg("limine/limine-bios-cd.bin")
        .arg("limine/limine-uefi-cd.bin")
        .arg("iso_root/boot/limine")
        .output()
        .unwrap())
}

fn put_boot_files() {
    out(Command::new("cp")
        .arg("-v")
        .arg("limine/BOOTX64.EFI")
        .arg("iso_root/EFI/BOOT")
        .output()
        .unwrap());

    out(Command::new("cp")
        .arg("-v")
        .arg("limine/BOOTIA32.EFI")
        .arg("iso_root/EFI/BOOT")
        .output()
        .unwrap());
}

fn make_iso() {
    // command too long ):
    out(Command::new("bash")
        .arg("-c")
        .arg(format!(
            "xorriso -as mkisofs -b boot/limine/limine-bios-cd.bin \
		-no-emul-boot -boot-load-size 4 -boot-info-table \
		--efi-boot boot/limine/limine-uefi-cd.bin \
		-efi-boot-part --efi-boot-image --protective-msdos-label \
		iso_root -o {ISO_PATH}"
        ))
        .output()
        .unwrap())
}

fn compile_programs() -> Output {
    Command::new("make")
        .arg("-C")
        .arg("programs")
        .output()
        .unwrap();
    Command::new("bash")
        .arg("-c")
        .arg("cd Shell && zig build")
        .output()
        .unwrap();
    Command::new("bash")
        .arg("-c")
        .arg("cd bin && zig build")
        .output()
        .unwrap();
    Command::new("bash")
        .arg("-c")
        .arg("cd TestBot && zig build")
        .output()
        .unwrap()
}

fn make_ramdisk() {
    let file = File::create("iso_root/boot/ramdisk.tar").unwrap();
    let mut tar_builder = Builder::new(file);

    let mut added_dirs = HashSet::<&Path>::new();

    for (src, dest) in RAMDISK_CONTENT {
        let (src, dest) = (Path::new(src), Path::new(dest));
        if src.is_file() {
            if let Some(parent) = dest.parent() {
                if !added_dirs.contains(parent) {
                    let mut empty_header = Header::new_ustar();
                    empty_header.set_path(parent).unwrap();
                    empty_header.set_entry_type(tar::EntryType::Directory);
                    empty_header.set_size(0);
                    empty_header.set_cksum();

                    tar_builder.append(&empty_header, empty()).unwrap();
                    added_dirs.insert(parent);
                }
            }

            tar_builder
                .append_file(
                    dest,
                    &mut File::open(src)
                        .expect("ramdisk contents corrupt file missing, edit RAMDISK_CONTENT"),
                )
                .unwrap();
        } else if src.is_dir() {
            added_dirs.insert(dest);
            tar_builder.append_dir_all(dest, src).unwrap();
        } else {
            panic!("ramdisk content is nethier a file nor directory (or doesn't exists), edit RAMDISK_CONTENT");
        }
    }

    tar_builder.finish().unwrap();
}

fn cleanup() {
    let _ = fs::remove_dir_all("iso_root");
}
/// TODO: spilt into more functions and make it work on other oses like windows
fn main() {
    cleanup();
    out(limine_make());
    setup_iso_root();

    put_kernel_img();
    put_limine_config();
    put_boot_files();

    out(compile_programs());
    make_ramdisk();
    make_iso();
    let iso_path = ISO_PATH;
    println!("cargo:rerun-if-changed={}", iso_path);
    println!("cargo:rerun-if-changed=limine");
    println!("cargo:rerun-if-changed=programs/build");
    println!("cargo:rerun-if-changed=programs");

    // pass the disk image paths as env variables to the `main.rs`
    println!("cargo:rustc-env=ISO_PATH={}", iso_path);
}
