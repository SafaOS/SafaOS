use safa_utils::{abi::raw::processes::ProcessStdio, make_path, types::Name};

use crate::{
    arch::without_interrupts,
    threading::{
        cpu_context::ContextPriority,
        expose::{SpawnFlags, pspawn, wait_for_process},
    },
};

#[test_case]
fn spawn_test() {
    without_interrupts(|| {
        let pid = pspawn(
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
