use bitfield_struct::bitfield;
use core::num::NonZero;

#[bitfield(u8)]
struct PitCommand {
    /// BCD/Binary Mode, 0 = 16-bit-binary
    binary_mode: bool,
    /**
    0 0 0 = Mode 0 (interrupt on terminal count)
    0 0 1 = Mode 1 (hardware re-triggerable one-shot)
    0 1 0 = Mode 2 (rate generator)
    0 1 1 = Mode 3 (square wave generator)
    1 0 0 = Mode 4 (software triggered strobe)
    1 0 1 = Mode 5 (hardware triggered strobe)
    1 1 0 = Mode 2 (rate generator, same as 010b)
    1 1 1 = Mode 3 (square wave generator, same as 011b)
    */
    #[bits(3)]
    operating_mode: u8,
    lo_byte_access: bool,
    hi_byte_access: bool,
    /**
    Select channel :
                    0 0 = Channel 0
                    0 1 = Channel 1
                    1 0 = Channel 2
                    1 1 = Read-back command (8254 only)
    */
    #[bits(2)]
    channel: u8,
}

macro_rules! ms_to_count {
    ($amount: expr) => {
        const {
            const FREQ_KHZ: f64 = 1193.182;
            (FREQ_KHZ * $amount as f64) as u32
        }
    };
}

/// Returns the frequency of the TSC in MHz
pub unsafe fn calibrate_tsc() -> NonZero<u64> {
    const MS: u32 = 10;
    const COUNT: u32 = ms_to_count!(MS);

    let start_lo: u32;
    let start_high: u32;
    let end_lo: u32;
    let end_high: u32;

    // Credits to ToaruOS for this segment
    // I was too lazy to exactly understand this but I broke it down
    unsafe {
        core::arch::asm!(
        "
            /* Disables and sets gating for channel 2 */
            in   al  , 0x61
            and  al  , 0xDD
            or   al  , 1
            out 0x61, al
            // Configure channel 2
            mov al, {}
            out 0x43, al
            // lower value
            mov al, {}
            out 0x42, al
            in  al, 0x60
            // higher value
            mov al, {}
            out 0x42, al
            // Re-enable
            in al, 0x61
            and al, 0xDE
            out 0x61, al
            // Pulse high
            or al, 1
            out 0x61, al
            // Store TSC before
            rdtsc
            mov {:e}, eax
            mov {:e}, edx
            /* In QEMU and VirtualBox, this seems to flip low.
            * On real hardware and VMware it flips high. */
            in al, 0x61
            and al, 0x20
            jz 3f

            /* Loop until output goes low? */
            2:
                in al, 0x61
                and al, 0x20
                jnz 2b

                rdtsc
                jmp 4f

            /* Loop until output goes high */
            3:
                in al, 0x61
                and al, 0x20
                jz 3b

                rdtsc
            4:
        ", const {
            PitCommand::new()
            .with_hi_byte_access(true)
            .with_lo_byte_access(true)
            .with_operating_mode(0b001)
            .with_channel(2)
            .into_bits()
        }, const COUNT as u8, const (COUNT >> 8) as u8, out(reg) start_lo, out(reg) start_high, out("eax") end_lo, out("edx") end_high);
    }

    let start_tsc = ((start_high as u64) << u32::BITS) | start_lo as u64;
    let end_tsc = ((end_high as u64) << u32::BITS) | end_lo as u64;

    let diff_tsc = end_tsc - start_tsc;
    NonZero::new(diff_tsc / MS as u64 / 1000).expect("TSC Calibration returned zero?")
}
