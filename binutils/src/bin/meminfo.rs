use extra::{json::MemInfo, tri_io};
use safa_api::errors::SysResult;

enum Mode {
    Bytes,
    KiB,
    MiB,
}

impl Default for Mode {
    fn default() -> Self {
        Self::MiB
    }
}

fn main() -> SysResult {
    let mut verbose = true;
    let mut mode = Mode::default();

    for arg in std::env::args() {
        match arg.as_str() {
            "-r" | "--raw" => verbose = false,
            "-k" => mode = Mode::KiB,
            "-b" => mode = Mode::Bytes,
            "-m" => mode = Mode::MiB,
            _ => {}
        }
    }

    let info = tri_io!(MemInfo::fetch());
    if !verbose {
        match mode {
            Mode::Bytes => println!("{}Bs/{}Bs", info.used(), info.total()),
            Mode::KiB => println!("{}KiBs/{}KiBs", info.used_kib(), info.total_kib()),
            Mode::MiB => println!("{}MiBs/{}MiBs", info.used_mib(), info.total_mib()),
        }
    } else {
        match mode {
            Mode::Bytes => println!(
                "{}Bs used of {}Bs, {}Bs free",
                info.used(),
                info.total(),
                info.free()
            ),
            Mode::KiB => println!(
                "{}KiBs used of {}KiBs, {}KiBs free",
                info.used_kib(),
                info.total_kib(),
                info.free_kib()
            ),
            Mode::MiB => println!(
                "{}MiBs used of {}MiBs, {}MiBs free",
                info.used_mib(),
                info.total_mib(),
                info.free_mib()
            ),
        }
    }

    SysResult::Success
}
