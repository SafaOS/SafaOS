use safa_utils::{abi::raw::processes::AbiStructures, make_path, types::Name};

use crate::threading::{
    cpu_context::ContextPriority,
    expose::{SpawnFlags, pspawn, wait_for_task},
};

#[test_case]
fn spawn_test() {
    unsafe {
        crate::arch::disable_interrupts();
    }
    let pid = pspawn(
        Name::try_from("TEST_CASE").unwrap(),
        make_path!("sys", "/bin/true"),
        &[],
        &[],
        SpawnFlags::empty(),
        ContextPriority::Medium,
        AbiStructures::default(),
    )
    .unwrap();
    let ret = wait_for_task(pid);

    assert_eq!(ret, Some(1));
    unsafe {
        crate::arch::enable_interrupts();
    }
}
