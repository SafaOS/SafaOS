use macros::test_module;

#[test_module]
pub mod testing_module {
    use alloc::vec::Vec;

    use crate::cross_println;
    use crate::println;
    use core::arch::asm;

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

    // syscall tests
    fn syscall() {
        let msg = "Hello from syswrite!\n";
        let len = msg.len();
        let msg = msg.as_ptr();

        unsafe {
            asm!(
                "mov rax, 3
                mov rdi, 1
                mov rsi, r9
                mov rdx, r10
                int 0x80", in("r9") msg, in("r10") len
            )
        }
    }
}
