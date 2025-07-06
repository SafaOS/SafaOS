use core::arch::asm;

use crate::{
    arch::aarch64::{
        gic::{IntGroup, IntID},
        registers::MPIDR,
    },
    info,
};

use super::exceptions::InterruptFrame;

const TIMER_TICK_PER_MS: usize = 5;
// TODO: only works on qemu virt
pub const TIMER_IRQ: IntID = IntID::from_int_id(30);

#[inline(always)]
/// Resets the timer to count Nms again before tiggring interrupt
unsafe fn reset_timer(n: usize) {
    unsafe {
        let freq: usize;
        asm!("mrs {}, cntfrq_el0", out(reg) freq);
        let value: u32 = ((freq / 1000) * n) as u32;
        asm!("msr cntp_tval_el0, {0:x}", in(reg) value);
    }
}

pub fn init_generic_timer() {
    TIMER_IRQ
        .clear_pending_all()
        .set_group_all(IntGroup::NonSecure)
        .enable_all();

    extern "C" fn setup_inner(_: usize) {
        let freq: usize;
        unsafe {
            asm!("mrs {}, cntfrq_el0", out(reg) freq);
        }

        unsafe {
            // Enables timer interrupt
            reset_timer(TIMER_TICK_PER_MS);
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

        let mpidr = MPIDR::read();

        info!(
            "initialized generic timer with freq: {}Mhz for CPU: {}",
            freq / 1000 / 1000,
            mpidr.cpuid()
        );
    }

    setup_inner(0);
    super::arch_utils::parked_cpus_do(setup_inner, 0);
}

pub fn on_interrupt(ctx: &mut InterruptFrame, is_fiq: bool) {
    unsafe {
        super::threading::context_switch(ctx, || {
            TIMER_IRQ.clear_pending().deactivate(is_fiq);
            reset_timer(TIMER_TICK_PER_MS)
        });
    }
}
