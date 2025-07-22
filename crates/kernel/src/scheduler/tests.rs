use crate::process::spawn::SpawnFlags;
use crate::utils::types::Name;
use crate::{process, utils::path::make_path};

use safa_abi::raw::processes::ProcessStdio;

use crate::{
    arch::without_interrupts,
    scheduler::{cpu_context::ContextPriority, expose::wait_for_process},
};

#[test_case]
fn spawn_test() {
    without_interrupts(|| {
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
        let ret = wait_for_process(pid);

        assert_eq!(ret, Some(1));
    });
}
