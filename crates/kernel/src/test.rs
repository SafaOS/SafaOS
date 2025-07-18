use core::any::type_name;

use crate::{
    arch::power::shutdown,
    eve::KERNEL_STDIO,
    info, sleep,
    threading::{
        cpu_context::ContextPriority,
        expose::{SpawnFlags, pspawn, wait_for_process},
    },
};
use safa_utils::{
    abi::raw::processes::{AbiStructures, ProcessStdio},
    make_path,
    types::Name,
};

#[macro_export]
macro_rules! test_log {
    ($($arg:tt)*) => {
        $crate::logln_ext!("test", "92", $($arg)*)
    };
}

macro_rules! ok {
    ($last_time_us: expr_2021) => {{
        let end_time_us = $crate::time!(us);
        let delta_time_us = end_time_us - $last_time_us;
        let delta_time_ms = delta_time_us / 1000;
        $crate::logln!(
            "[ \x1B[92m OK   \x1B[0m  ]\x1b[90m:\x1B[0m delta {}ms ({}us)",
            delta_time_ms,
            delta_time_us
        );
    }};
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
    // memory tests
    High,
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
        } else if const_str::contains!(name, "memory::") {
            TestPiritory::High
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
    test_log!("sleeping for 5 second(s) until kernel finishes startup...");
    sleep!(5000 ms);
    _ = *KERNEL_STDIO;

    let tests_iter = tests
        .iter()
        .filter(|x| x.piritory() == TestPiritory::Highest);
    let tests_iter = tests_iter.chain(tests.iter().filter(|x| x.piritory() == TestPiritory::High));
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

    test_log!("running {} tests", tests.len());
    let first_log_ms = crate::time!(ms);

    for test in tests_iter {
        unsafe {
            crate::arch::disable_interrupts();
        }
        test_log!("running test \x1B[90m{}\x1B[0m...", test.name(),);
        let last_log = crate::time!(us);
        test.run();
        ok!(last_log);
        unsafe {
            crate::arch::enable_interrupts();
        }
    }
    info!(
        "finished running tests in {}ms",
        crate::time!(ms) - first_log_ms
    );

    // printing 'PLEASE EXIT' to the serial makes `safa-helper test` know that the kernel tests were successful
    info!("PLEASE EXIT, automatically attempting exiting after 1000ms, PLEASE EXIT");
    sleep!(1000 ms);
    shutdown()
}

// runs the userspace test script
// always runs last because it is given the lowest priority (`[TestPiritory::Lowest`] because it is in this module)
#[test_case]
fn userspace_test_script() {
    use crate::drivers::vfs::expose::File;

    let stdio = File::open(make_path!("dev", "/ss")).unwrap();
    let stdio = ProcessStdio::new(Some(stdio.fd()), Some(stdio.fd()), Some(stdio.fd()));

    let pid = pspawn(
        Name::try_from("Tester").unwrap(),
        make_path!("sys", "bin/safa-tests"),
        &[],
        &[],
        SpawnFlags::empty(),
        ContextPriority::Medium,
        AbiStructures { stdio },
        None,
    )
    .unwrap();
    // thread yields, so works even when interrupts are disabled
    let ret = wait_for_process(pid);

    assert_eq!(ret, Some(0));
}
