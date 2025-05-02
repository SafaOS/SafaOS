use safa_utils::{abi::raw::processes::AbiStructures, make_path, types::Name};

use crate::threading::expose::{pspawn, wait, SpawnFlags};

#[test_case]
fn spawn_test() {
    unsafe { core::arch::asm!("cli") }
    let pid = pspawn(
        Name::try_from("TEST_CASE").unwrap(),
        make_path!("sys", "/bin/true"),
        &[],
        &[],
        SpawnFlags::empty(),
        AbiStructures::default(),
    )
    .unwrap();
    let ret = wait(pid);

    assert_eq!(ret, 1);
    unsafe { core::arch::asm!("sti") }
}
