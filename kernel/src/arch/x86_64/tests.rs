//! Contains architecture-specific tests.
//! always ran first before any other tests
// TODO: add more tests

use core::arch::asm;

use crate::terminal::FRAMEBUFFER_TERMINAL;

#[test_case]
fn a_long_mode() {
    let rax: u64;
    unsafe {
        core::arch::asm!(
            "
                mov rax, 0xFFFFFFFFFFFFFFFF
                mov {}, rax
            ",
            out(reg) rax
        );
    };

    assert_eq!(rax, 0xFFFFFFFFFFFFFFFF);
}

#[test_case]
fn interrupts() {
    unsafe { asm!("int3") }
}

#[test_case]
fn syscall() {
    let msg_raw = "Hello from syswrite!\n";
    let len = msg_raw.len();
    let msg = msg_raw.as_ptr();
    // sync
    unsafe {
        asm!(
            "
           mov rax, 0x10
           mov rdi, 1
           int 0x80
       "
        );
    }
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
        // should be equal because there is no flushing and we flushed before
        assert_eq!(
            FRAMEBUFFER_TERMINAL.read().stdout().as_bytes(),
            msg_raw.as_bytes()
        );
        // sync
        asm!(
            "
               mov rax, 0x10
               mov rdi, 1
               int 0x80
           "
        )
    }
}
