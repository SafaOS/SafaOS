use crate::{println, serial, threading::expose::thread_exit};
use macros::test_module;

#[test_module]
pub mod testing_module {
    use crate::alloc::string::ToString;
    use alloc::vec::Vec;

    use crate::memory::frame_allocator;
    use crate::memory::paging::PAGE_SIZE;
    use crate::println;
    use crate::threading::expose::pspawn;
    use crate::threading::expose::wait;
    use crate::threading::expose::SpawnFlags;
    use crate::utils::alloc::PageVec;
    use core::arch::asm;
    use core::mem::MaybeUninit;
    use safa_utils::make_path;

    fn serial() {}
    fn print() {}

    #[cfg(target_arch = "x86_64")]
    fn long_mode() {
        let rax: u64;
        unsafe {
            asm!(
                "
                    mov rax, 0xFFFFFFFFFFFFFFFF
                    mov {}, rax
                ",
                out(reg) rax
            );
        };

        assert_eq!(rax, 0xFFFFFFFFFFFFFFFF);
    }

    #[cfg(target_arch = "x86_64")]
    fn interrupts() {
        unsafe { asm!("int3") }
    }

    fn allocator() {
        let mut test = Vec::new();

        for i in 0..100 {
            test.push(i);
        }

        println!("{:#?}\nAllocated Vec with len {}", test, test.len());
    }

    fn page_allocator_test() {
        let mut test = PageVec::with_capacity(50);

        let page = [MaybeUninit::<u8>::uninit(); PAGE_SIZE];
        for _ in 0..50 {
            test.push(page);
        }
    }

    #[cfg(target_arch = "x86_64")]
    // syscall tests
    fn syscall() {
        let msg = "Hello from syswrite!\n";
        let len = msg.len();
        let msg = msg.as_ptr();

        unsafe {
            // writing "Hello from syswrite!\n" to the terminal
            asm!(
                "mov rax, 3
                mov rdi, 1
                mov rsi, 0
                mov rdx, r9
                mov rcx, r10
                mov r8, 0
                int 0x80", in("r9") msg, in("r10") len
            );
            // sync
            asm!(
                "
                mov rax, 17
                mov rdi, 1
                int 0x80
            "
            )
        }
    }

    fn frame_allocator() {
        let mut frames = heapless::Vec::<_, 1024>::new();
        for _ in 0..frames.capacity() {
            frames
                .push(frame_allocator::allocate_frame().unwrap())
                .unwrap();
        }

        for i in 1..frames.capacity() {
            assert_ne!(frames[i - 1].start_address(), frames[i].start_address());
        }

        let last_frame = frames[frames.len() - 1];
        for frame in frames.iter() {
            frame_allocator::deallocate_frame(*frame);
        }
        let allocated = frame_allocator::allocate_frame().unwrap();
        assert_eq!(allocated, last_frame);

        frame_allocator::deallocate_frame(allocated);
    }
    fn spawn() {
        unsafe { core::arch::asm!("cli") }
        let pid = pspawn(
            "TEST_CASE".to_string(),
            make_path!("sys", "/bin/true"),
            &[],
            SpawnFlags::CLONE_RESOURCES,
        )
        .unwrap();
        let ret = wait(pid);

        assert_eq!(ret, 1);
        unsafe { core::arch::asm!("sti") }
    }

    fn userspace() {
        unsafe { core::arch::asm!("cli") }
        let pid = pspawn(
            "TEST_BOT".to_string(),
            make_path!("sys", "bin/safa-tests"),
            &[],
            SpawnFlags::empty(),
        )
        .unwrap();
        let ret = wait(pid);

        assert_eq!(ret, 0);
        unsafe { core::arch::asm!("sti") }
    }
}

pub fn main() -> ! {
    testing_module::test_main();
    // printing this to the serial makes `test.sh` know that the kernel tests were succesful
    serial!("finished initing ...\n");
    println!("finished running tests...");
    println!("\x1B[38;2;0;255;0mBoot success! press ctrl + shift + C to start the shell\x1B[0m");
    // interrupts are typically disabled during syscalls
    unsafe { core::arch::asm!("cli") }
    thread_exit(0)
}
