use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
};

lazy_static! {
    pub static ref ROOT_REPO_PATH: PathBuf = env!("CARGO_MANIFEST_DIR").into();
}
use lazy_static::lazy_static;

#[path = "builder/cargo.rs"]
mod cargo;

#[path = "builder/make.rs"]
mod make;

const KERNEL_PATH: &'static str = "crates/kernel";
/// A bunch of binary crates which built results are included in the ramdisk in `sys:/bin/`
const USERSPACE_CRATES_PATH: &'static str = "crates-user";

/// Removes all environment variables that could break the build process
///
/// we don't use environment variables anywhere currently so this is perfectly safe
fn clear_env() {
    for (key, _) in std::env::vars() {
        if key.contains("RUSTUP") {
            unsafe {
                std::env::remove_var(key);
            }
        }
    }
}

/// A builder that builds SafaOS's images
pub struct Builder<'a> {
    root_repo_path: &'a Path,
    build_root_path: PathBuf,
    out_path: PathBuf,
    ramdisk: Vec<(PathBuf, PathBuf)>,
    is_tests: bool,
    verbose: bool,
}

#[macro_export]
macro_rules! log_verbose {
    ($builder:expr, $($args:tt)*) => {
        if $builder.verbose {
            eprintln!("[VERBOSE]: {}", format_args!($($args)*));
        }
    };
}

#[macro_export]
// TODO: make logging more pretty
macro_rules! log {
    ($($args:tt)*) => {
        eprintln!("[LOG]: {}", format_args!($($args)*));
    };
}

impl<'a> Builder<'a> {
    /// Constructs a new builder with the default settings
    pub fn create_advanced(root_repo_path: &'a Path, iso_name: &str) -> Self {
        Self {
            is_tests: false,
            verbose: false,
            root_repo_path,
            build_root_path: root_repo_path.join("out/iso_root"),
            out_path: root_repo_path.join(iso_name),
            ramdisk: Vec::new(),
        }
    }

    /// Create a SafaOS ISO builder
    /// this functions uses env!("CARGO_MANIFEST_DIR") as the root repo path
    /// and includes the ramdisk-include directory contents in the ramdisk
    pub fn create(iso_name: &str) -> Self {
        let root_repo_path = &*ROOT_REPO_PATH;

        let ramdisk_include_dir = root_repo_path
            .join("ramdisk-include")
            .read_dir()
            .expect("failed to open ramdisk include");

        let ramdisk_include = ramdisk_include_dir
            .filter_map(|s| s.ok())
            .map(|entry| (entry.path(), PathBuf::from(entry.file_name())));

        Builder::create_advanced(root_repo_path, iso_name).include_paths(ramdisk_include)
    }

    /// Builds an ISO that has tests either enabled or disabled for running tests.depends onv value
    pub fn set_testing(mut self, value: bool) -> Self {
        self.is_tests = value;
        self
    }

    pub fn set_verbose(mut self, value: bool) -> Self {
        self.verbose = value;
        self
    }

    /// Includes paths to ramdisk
    /// the paths layout are (real file path, relative path in ramdisk)
    pub fn include_paths(mut self, paths: impl Iterator<Item = (PathBuf, PathBuf)>) -> Self {
        self.ramdisk.extend(paths);
        self
    }

    /// Builds all the binary crates in [`USERSPACE_CRATES_PATH`] subdirecotry of self.root_repo_path
    /// returns a Vec of (the built executable path, the path in the ramdisk)
    fn build_userspace(&self) -> Vec<(PathBuf, PathBuf)> {
        let userspace_crates_path = self.root_repo_path.join(USERSPACE_CRATES_PATH);
        let userspace_crates_dir =
            fs::read_dir(userspace_crates_path).expect("failed to read the crates-user dir");

        let crates: Vec<PathBuf> = userspace_crates_dir
            .filter_map(|i| i.ok())
            .filter(|i| i.file_type().is_ok_and(|t| t.is_dir()))
            .map(|i| i.path())
            .collect();

        let mut results = Vec::with_capacity(crates.len());
        for cr in crates {
            let binaries = cargo::build_safaos(&cr, &["--release"]);
            for (path, name) in binaries {
                results.push((path, format!("bin/{}", name).into()));
            }
        }

        results
    }

    /// Builds and packages the ramdisk tar to the iso root
    fn package_ramdisk(&self, boot_build_path: &Path) {
        let userspace_binaries = self.build_userspace();
        let userspace_binaries = userspace_binaries.iter();

        let ramdisk = self.ramdisk.iter();
        let ramdisk = ramdisk.chain(userspace_binaries);

        fs::create_dir_all(&boot_build_path).expect("failed to create build directory");

        let ramdisk_tar_path = boot_build_path.join("ramdisk.tar");
        log!("building the ramdisk...");

        let ramdisk_tar = File::create(ramdisk_tar_path).expect("failed to create ramdisk.tar");
        // building the ramdisk archive
        let mut ramdisk_builder = tar::Builder::new(ramdisk_tar);

        let mut ramdisk_directories: Vec<&Path> = Vec::new();

        for (real_path, ramdisk_path) in ramdisk {
            log_verbose!(
                self,
                "building ramdisk: {} => {}",
                real_path.display(),
                ramdisk_path.display()
            );
            assert!(real_path.exists());

            if let Some(parent) = ramdisk_path.parent() {
                // make sure the parent was already created in the ramdisk otherwise add it
                if parent != Path::new("") && !ramdisk_directories.contains(&parent) {
                    // TODO: dirty solution that works for now but might break in the future
                    let mut header = tar::Header::new_ustar();
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_path(parent).expect("failed to create header");
                    header.set_size(0);
                    ramdisk_builder
                        .append(&header, std::io::empty())
                        .expect("failed to append header to ramdisk");

                    ramdisk_directories.push(parent);
                }
            }

            if real_path.is_dir() {
                ramdisk_builder
                    .append_dir_all(&real_path, &ramdisk_path)
                    .expect("failed to append dir to tar archive");
            } else {
                ramdisk_builder
                    .append_path_with_name(&real_path, &ramdisk_path)
                    .expect("failed to append to tar archive");
            }
        }

        ramdisk_builder
            .finish()
            .expect("failed to finish building the ramdisk.tar");
        log!("finished building ramdisk");
    }

    /// Builds the kernel to the iso root
    fn package_kernel(
        &self,
        boot_build_path: &Path,
        build_function: impl FnOnce(&Path, &'static [&'static str]) -> Vec<(PathBuf, String)>,
    ) {
        fs::create_dir_all(boot_build_path).expect("failed to create boot build dir");

        let kernel_crate_path = self.root_repo_path.join(KERNEL_PATH);
        let mut kernel_elf = build_function(&kernel_crate_path, &[]).into_iter();
        assert_eq!(
            kernel_elf.len(),
            1,
            "failed building the kernel: no kernel elf built or multiple kernel elfs built"
        );

        let (kernel_elf, _) = kernel_elf.next().unwrap();
        let kernel_build_path = boot_build_path.join("kernel");

        log_verbose!(
            self,
            "copying kernel: {} => {}",
            kernel_elf.display(),
            kernel_build_path.display()
        );

        fs::copy(kernel_elf, kernel_build_path)
            .expect("failed to copy the kernel elf to iso build dir");
    }

    /// Builds and copies limine to the iso_root
    fn package_limine(&self, boot_build_path: &Path) {
        log!("building limine...");
        let limine_path = self.root_repo_path.join("limine");
        let limine_build_path = boot_build_path.join("limine");
        let efi_boot_build_path = self.build_root_path.join("EFI/BOOT");

        fs::create_dir_all(&limine_build_path).expect("failed to create limine build dir");
        fs::create_dir_all(&efi_boot_build_path).expect("failed to create efi build dir");

        make::build(&limine_path);
        for src in [
            limine_path.join("limine-bios.sys"),
            limine_path.join("limine-bios-cd.bin"),
            limine_path.join("limine-uefi-cd.bin"),
            self.root_repo_path.join("limine.conf"),
        ] {
            log_verbose!(
                self,
                "building limine cp: {} => {}",
                src.display(),
                limine_build_path.display()
            );
            fs::copy(&src, limine_build_path.join(src.file_name().unwrap()))
                .expect("failed to copy file");
        }

        for src in ["BOOTIA32.EFI", "BOOTX64.EFI"] {
            let full_path = limine_path.join(src);
            log_verbose!(
                self,
                "building limine cp: {} => {}",
                full_path.display(),
                efi_boot_build_path.display()
            );

            fs::copy(full_path, efi_boot_build_path.join(src)).expect("failed to copy file");
        }

        log!("successfully built limine");
    }

    fn package_final_iso(self) {
        log!("packaging iso");
        let status = Command::new("xorriso")
            .arg("-as")
            .arg("mkisofs")
            .arg("-b")
            .arg("boot/limine/limine-bios-cd.bin")
            .arg("-no-emul-boot")
            .arg("-boot-load-size")
            .arg("4")
            .arg("-boot-info-table")
            .arg("--efi-boot")
            .arg("boot/limine/limine-uefi-cd.bin")
            .arg("-efi-boot-part")
            .arg("--efi-boot-image")
            .arg("--protective-msdos-label")
            .arg(self.build_root_path)
            .arg("-o")
            .arg(self.out_path)
            .spawn()
            .expect("failed to spawn xorriso")
            .wait()
            .expect("failed to wait for xorriso");
        if !status.success() {
            panic!("failed to build iso {}", status);
        }
        log!("ISO built successfully");
    }

    /// Builds the iso
    pub fn build(self) {
        clear_env();
        // the iso is structured like this:
        // /boot/kernel: the kernel elf
        // /boot/ramdisk.tar: the ramdisk
        // /boot/limine/: limine binaries and conf
        // /boot/EFI: efi boot files
        let boot_build_path = self.build_root_path.join("boot");

        let freestanding_build_function = if self.is_tests {
            cargo::build_tests_freestanding
        } else {
            cargo::build_freestanding
        };

        // the kernel
        self.package_kernel(&boot_build_path, freestanding_build_function);
        // the ramdisk
        self.package_ramdisk(&boot_build_path);
        // the bootloader
        self.package_limine(&boot_build_path);
        self.package_final_iso()
    }
}
