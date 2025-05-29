use core::arch::asm;

use crate::terminal::FRAMEBUFFER_TERMINAL;

#[test_case]
fn syscall() {
    let msg_raw = "Hello from syswrite!\n";
    let len = msg_raw.len();
    let msg = msg_raw.as_ptr();
    // sync
    unsafe {
        asm!(
            "
           mov x0, 1
           svc #0x10
           "
        );
    }
    unsafe {
        // writing "Hello from syswrite!\n" to the terminal
        asm!(
            "
            mov x0, 1
            mov x1, 0
            mov x4, 0
            svc #0x3", in("x2") msg, in("x3") len
        );
        // should be equal because there is no flushing and we flushed before
        assert_eq!(
            FRAMEBUFFER_TERMINAL.read().stdout().as_bytes(),
            msg_raw.as_bytes()
        );
        // sync
        asm!(
            "
            mov x0, 1
            svc #0x10
           "
        )
    }
}
