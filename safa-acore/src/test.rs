use core::any::type_name;

use crate::{
    arch::{self, without_interrupts},
    logging,
};

// use crate::timer::{DurationFmt, SystemInstant};

#[macro_export]
macro_rules! test_log {
    ($($arg:tt)*) => {
        $crate::logging::info!("test", $($arg)*)
    };
}

macro_rules! ok {
    ($instant: expr_2021) => {{
        // let elapsed = $instant.elapsed();
        // $crate::logln!(
        //     "[ \x1B[92m OK   \x1B[0m  ]\x1b[90m:\x1B[0m delta {}",
        //     $crate::timer::DurationFmt::new(elapsed)
        // );
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
    // let first_log_instant = SystemInstant::now();

    for test in tests_iter {
        without_interrupts(|| {
            test_log!("running test \x1B[90m{}\x1B[0m...", test.name(),);
            // let instant = SystemInstant::now();
            test.run();
            ok!(instant);
        })
    }

    // let elapsed = first_log_instant.elapsed();
    // logging::info!("finished running tests in {}", DurationFmt::new(elapsed));

    // printing 'PLEASE EXIT' to the serial makes `safa-helper test` know that the kernel tests were successful
    logging::info!(
        "test",
        "PLEASE EXIT, automatically attempting exiting after 1000ms, PLEASE EXIT"
    );
    loop {
        arch::halt()
    }
}
