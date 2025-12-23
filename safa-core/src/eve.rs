//! Eve is the kernel's main loop (PID 0)
//! it is responsible for managing a few things related to it's children

use crate::memory::paging::PAGE_SIZE;
use crate::scheduler::Scheduler;
use crate::utils::alloc::PageString;
use crate::utils::path::make_path;
use crate::{fs, logging};
use crate::{serial, thread};
use safa_abi::fs::OpenOptions;
use safa_abi::process::ProcessStdio;
use spin::Lazy;

pub(super) static KERNEL_STDIO: Lazy<ProcessStdio> = Lazy::new(|| {
    let stdin =
        fs::FileRef::open_with_options(make_path!("dev", "tty"), OpenOptions::READ).unwrap();
    let stdout =
        fs::FileRef::open_with_options(make_path!("dev", "tty"), OpenOptions::WRITE).unwrap();
    let stderr = stdout.dup();
    ProcessStdio::new(Some(stdout.fd()), Some(stdin.fd()), Some(stderr.fd()))
});

pub fn main() -> ! {
    *logging::SERIAL_LOG.write() = Some(PageString::with_capacity(&"Journal", PAGE_SIZE * 4));
    crate::info!("eve has been awaken ...");

    crate::drivers::pci::init();

    // NOTE: May deadlock because the journal could request memory while lock is held (this is why we allocate 4 pages).
    crate::memory::vmm::with_root(|vmm| vmm.debug_regions());

    serial!("Hello, world!, running tests...\n",);

    #[cfg(not(test))]
    {
        use crate::process::spawn::{SpawnFlags, pspawn};
        use crate::thread::ContextPriority;
        use crate::utils::types::Name;

        // start the shell
        pspawn(
            Name::try_from("Shell").unwrap(),
            // Maybe we can make a const function or a macro for this
            make_path!("sys", "bin/safa"),
            &["sys:/bin/safa", "-i"],
            &[b"PATH=sys:/bin", b"SHELL=sys:/bin/safa"],
            SpawnFlags::empty(),
            ContextPriority::Medium,
            *KERNEL_STDIO,
            None,
        )
        .unwrap();
    }

    #[cfg(test)]
    {
        use crate::thread::{ContextPriority, Tid};

        fn run_tests(_tid: Tid, _arg: &()) -> ! {
            crate::kernel_testmain();
            unreachable!()
        }

        crate::process::current::kernel_thread_spawn(
            run_tests,
            &(),
            Some(ContextPriority::Medium),
            None,
        )
        .expect("failed to spawn Test Thread");
    }

    thread::current::exit(0)
}

pub fn idle_function() -> ! {
    crate::serial!("entered idle\n");
    let scheduler = Scheduler::get().expect("IDLE Started without scheduler");
    scheduler.idle()
}
