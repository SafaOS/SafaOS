use core::arch::asm;

use crate::{
    arch::aarch64::gic::{IntGroup, IntID},
    info,
};

use super::exceptions::InterruptFrame;

// TODO: only works on qemu virt
pub const TIMER_IRQ: IntID = IntID::from_int_id(30);

#[inline(always)]
/// Resets the timer to count Nms again before tiggring interrupt
unsafe fn reset_timer(n: usize) {
    let freq: usize;
    asm!("mrs {}, cntfrq_el0", out(reg) freq);
    let value: u32 = ((freq / 1000) * n) as u32;
    asm!("msr cntp_tval_el0, {0:x}", in(reg) value);
}

pub fn init_generic_timer() {
    TIMER_IRQ
        .clear_pending()
        .set_group(IntGroup::NonSecure)
        .enable();

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
}

pub fn on_interrupt(ctx: &mut InterruptFrame) {
    TIMER_IRQ.clear_pending();
    unsafe {
        super::threading::context_switch(ctx, || reset_timer(10));
    }
}
