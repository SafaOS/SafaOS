use core::sync::atomic::{AtomicI16, AtomicU8, Ordering};

use crate::arch::x86_64::outb;

use super::apic::{IOREDTBL, send_eoi, write_ioapic_irq};

const PIT_CHANNEL_0: u16 = 0x40;
const PIT_COMMAND_CHANNEL: u16 = 0x43;

fn set_freq(freq: u32) {
    let command: u8 = 0b00_11_010_0;

    outb(PIT_COMMAND_CHANNEL, command);
    outb(PIT_CHANNEL_0, (freq & 0xFF) as u8);
    outb(PIT_CHANNEL_0, (freq >> 8) as u8);
}

pub static PIT_IRQ: AtomicU8 = AtomicU8::new(2);
pub static PIT_COUNTER: AtomicI16 = AtomicI16::new(0);

pub extern "x86-interrupt" fn pit_handler() {
    PIT_COUNTER.fetch_sub(1, Ordering::Relaxed);
    send_eoi();
}

/// prepares the PIT to sleep for `ms` milliseconds
/// make sure that timer is disabled until [`sleep`] is called
pub fn prepare_sleep(ms: u32) {
    set_freq(1193);
    PIT_COUNTER.store(ms as i16, Ordering::Relaxed);
}

#[inline(always)]
/// sleeps for amount of milliseconds specified by [`prepare_sleep`]
/// returns the ticks that passed during the sleep according to `get_ticks`
pub fn calibrate_sleep<F, G, FR, GR>(lapic_id: u8, before_sleep: F, after_sleep: G) -> GR
where
    F: Fn() -> FR,
    G: Fn(FR) -> GR,
{
    enable(lapic_id);
    let ticks = before_sleep();

    while PIT_COUNTER.load(Ordering::Relaxed) > 0 {
        core::hint::spin_loop();
    }

    let result = after_sleep(ticks);
    disable(lapic_id);
    result
}

#[inline(always)]
pub fn enable(lapic_id: u8) {
    let irq = PIT_IRQ.load(Ordering::Relaxed);
    unsafe {
        let pit = IOREDTBL::new().with_vector(0x22).with_destination(lapic_id);
        crate::serial!("enabled!\n");
        write_ioapic_irq(irq, pit);
    }
}

#[inline(always)]
pub fn disable(_lapic_id: u8) {
    let irq = PIT_IRQ.load(Ordering::Relaxed);

    unsafe {
        let pit = IOREDTBL::new().with_vector(0x0).with_masked(true);
        write_ioapic_irq(irq, pit);
    }
}

/// initializes the pit
/// TODO: USE
pub fn init(irq: u8) {
    PIT_IRQ.store(irq, Ordering::Relaxed);
}
