use core::{arch::asm, cell::SyncUnsafeCell};

use hfdt_rs::Cells;

use crate::{
    arch::aarch64::{
        cpu::CPUDevice,
        interrupts::{self, DeviceIrq, IntGroup},
        registers::MPIDR,
    },
    info,
};

use super::exceptions::InterruptFrame;

pub const TIMER_TICK_PER_MS: usize = crate::scheduler::TIME_PER_QUANTUM as usize;

struct Timer {
    _secure_int: Cells<'static>,
    non_secure_int: Cells<'static>,
    interrupt_parent: hfdt_rs::Node<'static>,
}

impl CPUDevice for Timer {
    const COMPATIBLE: &'static [&'static str] = &["arm,armv8-timer", "arm,armv7-timer"];
    fn create(node: hfdt_rs::Node<'static>) -> Result<Self, &'static str> {
        let cont = node
            .interrupt_parent()
            .expect("Failed to get interrupt parent");
        let mut interrupts = node
            .interrupts(&cont)
            .expect("No interrupts found for timer");

        let secure = interrupts.next().expect("No secure interrupt found");
        let non_secure = interrupts.next().expect("No non-secure interrupt found");

        Ok(Timer {
            _secure_int: secure,
            non_secure_int: non_secure,
            interrupt_parent: cont,
        })
    }
}
static TIMER_IRQ: SyncUnsafeCell<Option<DeviceIrq>> = SyncUnsafeCell::new(None);

/// Returns the IRQ number of the timer
pub fn irq_num() -> u32 {
    unsafe { ((&*TIMER_IRQ.get()).as_ref().unwrap()).irq() }
}

#[inline(always)]
/// Resets the timer to count Nms again before tiggring interrupt
pub unsafe fn reset_timer(n: usize) {
    unsafe {
        let freq: usize;
        asm!("mrs {}, cntfrq_el0", out(reg) freq);
        let value: u32 = ((freq / 1000) * n) as u32;
        asm!("msr cntp_tval_el0, {0:x}", in(reg) value);
    }
}

pub fn setup_generic_timer() {
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
pub fn init_generic_timer() {
    let timer_irq_ptr = TIMER_IRQ.get();

    let timer = Timer::lookup().expect("No arm timer found");

    let timer_irq = interrupts::register_device_irq(
        &timer.interrupt_parent,
        timer.non_secure_int,
        IntGroup::NonSecure,
    )
    .expect("Failed to register timer interrupt");
    crate::debug!("Registered timer irq");
    unsafe { *timer_irq_ptr = Some(timer_irq) };
}

pub fn on_interrupt(ctx: &mut InterruptFrame, is_fiq: bool) {
    unsafe {
        super::threading::context_switch(ctx, |time| {
            (&*TIMER_IRQ.get()).as_ref().unwrap().send_eoi(is_fiq);
            reset_timer(if time == 0 {
                TIMER_TICK_PER_MS
            } else {
                time as usize
            })
        });
    }
}
