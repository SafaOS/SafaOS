use core::arch::asm;

use crate::info;

use super::{exceptions::InterruptFrame, gic};

// TODO: only works on qemu virt
const TIMER_IRQ: u32 = 30;

#[inline(always)]
/// Resets the timer to count Nms again before tiggring interrupt
unsafe fn reset_timer(n: usize) {
    let freq: usize;
    asm!("mrs {}, cntfrq_el0", out(reg) freq);
    let value: u32 = ((freq / 1000) * n) as u32;
    asm!("msr cntp_tval_el0, {0:x}", in(reg) value);
}

pub fn init_generic_timer() {
    gic::clear_pending(TIMER_IRQ);
    gic::enable(TIMER_IRQ);

    let freq: usize;
    unsafe {
        asm!("mrs {}, cntfrq_el0", out(reg) freq);
    }

    unsafe {
        // Enables timer interrupt
        reset_timer(10);
        asm!(
            "
            mov x1, #{flags}
            mrs x2, cntp_ctl_el0
            orr x2, x2, x1
            msr cntp_ctl_el0, x2
            ",
            flags = const 0b001,
        );
    }
    info!(
        "initialized generic timer with freq: {}Mhz",
        freq / 1000 / 1000
    );
    loop {}
}

pub fn on_interrupt(_ctx: &mut InterruptFrame) {
    unsafe {
        super::serial::write_str(".");
        gic::clear_pending(TIMER_IRQ);
        reset_timer(10);
    }
}
