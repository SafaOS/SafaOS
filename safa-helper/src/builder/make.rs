use super::log;
use std::{path::Path, process::Command};

/// Builds the given path using make.
/// panicks if unsuccessful
pub fn build(path: &Path) {
    log!("building {} with make", path.display());
    let status = Command::new("make")
        .arg("-C")
        .arg(path)
        .spawn()
        .expect("failed to spawn make process")
        .wait()
        .expect("failed to wait for make process");
    if !status.success() {
        panic!("make -C {} failed with status {}", path.display(), status);
    }
    log!("make -C {} exited successfully", path.display());
}
