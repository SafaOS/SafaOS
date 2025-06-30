use safa_utils::{abi::raw::processes::AbiStructures, make_path, types::Name};

use crate::threading::{
    cpu_context::ContextPriority,
    expose::{SpawnFlags, pspawn, wait},
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
    let ret = wait(pid);

    assert_eq!(ret, 1);
    unsafe {
        crate::arch::enable_interrupts();
    }
}
