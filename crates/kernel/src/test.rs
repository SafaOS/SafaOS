use core::any::type_name;

use crate::{
    arch::power::shutdown,
    info,
    threading::expose::{pspawn, wait, SpawnFlags},
};
use safa_utils::{
    abi::raw::processes::{AbiStructures, TaskStdio},
    make_path,
    types::Name,
};

macro_rules! log {
    ($($arg:tt)*) => {
        $crate::logln!("[ \x1B[92m test \x1B[0m  ]\x1b[90m:\x1B[0m {}", format_args!($($arg)*))
    };
}

macro_rules! ok {
    ($last_time: expr) => {
        $crate::logln!(
            "[ \x1B[92m OK   \x1B[0m  ]\x1b[90m:\x1B[0m delta {}ms",
            $crate::time!() - $last_time
        );
    };
}

pub trait Testable {
    fn run(&self);
    #[inline(always)]
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }
    #[inline(always)]
    fn piritory(&self) -> TestPiritory {
        get_test_piritory::<Self>()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Represents the priority of a test.
pub enum TestPiritory {
    // crate::arch tests must be ran before other tests to ensure fail order
    Highest,
    Medium,
    // tests that run last, given to this module tests
    Lowest,
}

const fn get_test_piritory<T: ?Sized>() -> TestPiritory {
    const {
        let name = type_name::<T>();
        if const_str::contains!(name, "test::") {
            TestPiritory::Lowest
        } else if const_str::contains!(name, "arch::") {
            TestPiritory::Highest
        } else {
            TestPiritory::Medium
        }
    }
}

impl<T: Fn()> Testable for T {
    fn run(&self) {
        self();
    }
}

pub fn test_runner(tests: &[&dyn Testable]) -> ! {
    let tests_iter = tests
        .iter()
        .filter(|x| x.piritory() == TestPiritory::Highest);
    let tests_iter = tests_iter.chain(
        tests
            .iter()
            .filter(|x| x.piritory() == TestPiritory::Medium),
    );
    let tests_iter = tests_iter.chain(
        tests
            .iter()
            .filter(|x| x.piritory() == TestPiritory::Lowest),
    );

    log!("running {} tests", tests.len());
    let first_log = crate::time!();

    for test in tests_iter {
        log!("running test \x1B[90m{}\x1B[0m...", test.name(),);
        let last_log = crate::time!();
        test.run();
        ok!(last_log);
    }
    info!("finished running tests in {}ms", crate::time!() - first_log);

    // printing this to the serial makes `test.sh` know that the kernel tests were successful
    info!("PLEASE EXIT");
    shutdown()
}

// runs the userspace test script
// always runs last because it is given the lowest priority (`[TestPiritory::Lowest`] because it is in this module)
#[test_case]
fn userspace_test_script() {
    unsafe { core::arch::asm!("cli") }
    use crate::drivers::vfs::expose::File;

    let stdio = File::open(make_path!("dev", "/ss")).unwrap();
    let stdio = TaskStdio::new(Some(stdio.fd()), Some(stdio.fd()), Some(stdio.fd()));

    let pid = pspawn(
        Name::try_from("Tester").unwrap(),
        make_path!("sys", "bin/safa-tests"),
        &[],
        &[],
        SpawnFlags::empty(),
        AbiStructures { stdio },
    )
    .unwrap();
    let ret = wait(pid);

    assert_eq!(ret, 0);
    unsafe { core::arch::asm!("sti") }
}
