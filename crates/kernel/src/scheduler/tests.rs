use crate::process::spawn::SpawnFlags;
use crate::thread::{self, ContextPriority};

use crate::utils::types::Name;
use crate::{process, utils::path::make_path};

use safa_abi::process::ProcessStdio;

#[test_case]
fn spawn_test() {
    let pid = process::spawn::pspawn(
        Name::try_from("TEST_CASE").unwrap(),
        make_path!("sys", "/bin/true"),
        &[],
        &[],
        SpawnFlags::empty(),
        ContextPriority::Medium,
        ProcessStdio::default(),
        None,
    )
    .unwrap();

    let ret = thread::current::wait_for_process(pid);

    assert_eq!(ret, Some(1));
}
