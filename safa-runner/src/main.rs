fn main() {
    safa_builder::build();
    let mut args = std::env::args();
    args.next();

    let iso_path = "safaos.iso";

    let uefi = true;

    let mut cmd = std::process::Command::new("qemu-system-x86_64");
    if uefi {
        cmd.arg("-display")
            .arg("gtk")
            .arg("-bios")
            .arg(ovmf_prebuilt::ovmf_pure_efi());
        cmd.arg("-drive")
            .arg(format!("format=raw,file={iso_path}"))
            .arg("-serial")
            .arg("stdio")
            .arg("-m")
            .arg("512M");
    }

    let mut kvm = true;
    let mut gui = true;

    for arg in args {
        match arg.as_str() {
            "no-kvm" => kvm = false,
            "no-gui" => gui = false,
            "debugger" => {
                cmd.arg("-s").arg("-S");
                println!("listening on port 1234 for debugger...");
            }
            arg => panic!("Unknown argument {}", arg),
        }
    }

    if kvm {
        cmd.arg("-enable-kvm");
    }
    if !gui {
        cmd.arg("-display").arg("none");
    }

    let mut child = cmd.spawn().unwrap();
    child.wait().unwrap();
}
