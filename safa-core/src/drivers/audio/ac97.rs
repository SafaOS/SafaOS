use core::{
    cell::{OnceCell, UnsafeCell},
    mem::ManuallyDrop,
};

use bitflags::bitflags;

use crate::{
    PhysAddr,
    arch::{self, without_interrupts},
    audio::{
        self,
        interface::{AudioCard, AudioInfo},
    },
    debug,
    drivers::{
        interrupts::{self, IRQInfo, InterruptReceiver},
        pci::{Bar, PCICommandReg, PCIDevice},
    },
    error,
    memory::{
        frame_allocator::{self, Frame, FramePtr},
        paging::PAGE_SIZE,
    },
    utils::{alloc::PageVec, locks::SpinLock},
    warn, write_ref,
};

bitflags! {
    #[derive(Debug, Clone, Copy)]
    struct BDConf: u16 {
        /// Fire interrupt on data transfer.
        const IOC = 1 << 15;
        /// Is last entry.
        const LAST_ENTRY = 1 << 14;
    }
}
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct BDesc {
    paddr: u32,
    samples: u16,
    config: BDConf,
}

impl BDesc {
    pub const fn paddr(&self) -> PhysAddr {
        PhysAddr::from(self.paddr as usize)
    }

    pub fn buf_mut(&mut self) -> &mut [u8; PAGE_SIZE] {
        unsafe { &mut *self.paddr().into_virt().into_ptr() }
    }

    pub fn new(paddr: PhysAddr) -> Self {
        assert!(
            *paddr <= u32::MAX as usize,
            "Expected 32bit physical address"
        );
        Self {
            paddr: *paddr as u32,
            samples: (PAGE_SIZE / size_of::<u16>()) as u16,
            config: BDConf::empty(),
        }
    }
}

#[derive(Debug)]
pub struct AC97Registers {
    nam_port: u16,
    nabm_port: u16,
}

impl AC97Registers {
    const GLOBAL_CONTROL_REG: u16 = 0x2C;
    const NAM_RESET_REG: u16 = 0x0;
    const PCM_OUTPUT_VOL_REG: u16 = 0x18;
    const MASTER_VOL_REG: u16 = 0x02;
    const TRANS_CONTROL_REG_OUT: u16 = 0x1B;
    const TRANS_STATUS_REG_OUT: u16 = 0x16;
    const BUF_DESC_BASE_OUT: u16 = 0x10;
    /// The index of the last valid entry. i.e len - 1
    const LAST_VALID_INDEX_OUT: u16 = 0x15;
    /// The index of the entry being currently processed.
    const CURR_ENT_OUT: u16 = 0x14;

    fn from_bars(bars: &[Bar]) -> Self {
        let Bar::IO(nam, _) = bars[0] else {
            panic!("AC97 Card expected IO Bar at 0")
        };

        let Bar::IO(nabm, _) = bars[1] else {
            panic!("AC97 Card expected IO Bar at 1")
        };

        Self {
            nam_port: nam as u16,
            nabm_port: nabm as u16,
        }
    }

    #[inline]
    /// Writes from an offset into the Native Audio Mixers register.
    unsafe fn write_nam(&self, off: u16, val: u16) {
        unsafe { arch::io::outw(self.nam_port + off, val) }
    }

    #[inline]
    /// Reads a word from an offset into the Native Audio Bus Master register.
    unsafe fn read_nabmw(&self, off: u16) -> u16 {
        unsafe { arch::io::inw(self.nabm_port + off) }
    }
    #[inline]
    /// Writes a word from an offset into the Native Audio Bus Master register.
    unsafe fn write_nabmw(&self, off: u16, val: u16) {
        unsafe { arch::io::outw(self.nabm_port + off, val) }
    }

    #[inline]
    /// Reads a byte from an offset into the Native Audio Bus Master register.
    unsafe fn read_nabmb(&self, off: u16) -> u8 {
        unsafe { arch::io::inb(self.nabm_port + off) }
    }

    #[inline]
    /// Writes a byte from an offset into the Native Audio Bus Master register.
    unsafe fn write_nabmb(&self, off: u16, val: u8) {
        unsafe { arch::io::outb(self.nabm_port + off, val) }
    }

    #[inline]
    /// Writes a dword from an offset into the Native Audio Bus Master register.
    unsafe fn write_nabm_dw(&self, off: u16, val: u32) {
        unsafe { arch::io::outl(self.nabm_port + off, val) }
    }

    unsafe fn reset(&self) {
        unsafe {
            // Resume to operational state
            self.write_nabm_dw(Self::GLOBAL_CONTROL_REG, 0x3u32);
            // Reset NAM Regiters by writing anything.
            self.write_nam(Self::NAM_RESET_REG, 0xaaaa);
            // Sets maximum volume for all outputs.
            self.write_nam(Self::PCM_OUTPUT_VOL_REG, 0);
            // Sets maximum volume for master output.
            self.write_nam(Self::MASTER_VOL_REG, 0);
        }
    }

    unsafe fn enable_transfer_int(&self) {
        unsafe {
            self.write_nabmb(Self::TRANS_CONTROL_REG_OUT, 0b11100);
        }
    }

    unsafe fn reset_transfer(&self, bdl_count: u8) {
        unsafe {
            self.write_nabmb(Self::LAST_VALID_INDEX_OUT, bdl_count - 1);
            let t_c_r = self.read_nabmb(Self::TRANS_CONTROL_REG_OUT);
            // Set transfer bit
            self.write_nabmb(Self::TRANS_CONTROL_REG_OUT, t_c_r | 1);
        }
    }

    unsafe fn init_bdl(&self, bdl: FramePtr<[BDesc]>) -> Result<(), ()> {
        unsafe {
            assert!(
                *bdl.phys_addr() as usize <= u32::MAX as usize,
                "Physical DMA Address isn't 32bit"
            );
            assert!(bdl.len() <= 32, "AC97 expected at most 32 entries");
            let paddr = *bdl.phys_addr() as u32;
            // self.write_nabmb(Self::TRANS_CONTROL_REG_OUT, 0x2);

            // if !sleep_until!(1000 ms, (self.read_nabmb(Self::TRANS_CONTROL_REG_OUT) & 0x2) == 0x2) {
            //     error!(AC97Registers, "Failed to reset transfer control register");
            //     return Err(());
            // }

            self.write_nabm_dw(Self::BUF_DESC_BASE_OUT, paddr);
            Ok(())
        }
    }
}

#[derive(Debug)]
struct AC97Queue {
    bdl: SpinLock<FramePtr<[BDesc]>>,
    queued_samples: SpinLock<PageVec<u8>>,
    write_ptr: UnsafeCell<u8>,
}

impl AC97Queue {
    pub fn create() -> Result<Self, ()> {
        let mut raw_frames = heapless::Vec::<Frame, 32>::new();
        let mut allocated = heapless::Vec::<BDesc, 32>::new();
        for i in 0..allocated.capacity() {
            let Some(frame) = frame_allocator::allocate_frame() else {
                break;
            };

            let paddr = frame.phys_addr();
            if *paddr > u32::MAX as usize {
                break;
            }

            raw_frames.push(frame).unwrap();
            let mut bd = BDesc::new(paddr);
            // Make things a bit smoother by delaying interrupts.
            if i.is_multiple_of(8) && i != 0 {
                bd.config = BDConf::IOC;
            }
            allocated.push(bd).unwrap();
        }

        if allocated.is_empty() {
            error!(AC97, "No DMA allocations were made");
            // FIXME: What is Drop?
            for frame in raw_frames {
                frame_allocator::deallocate_frame(frame);
            }
            return Err(());
        }

        let holder = frame_allocator::allocate_frame().ok_or(())?;
        let holder_paddr = holder.phys_addr();
        if *holder_paddr >= u32::MAX as usize {
            // FIXME: What is Drop?
            for frame in raw_frames {
                frame_allocator::deallocate_frame(frame);
            }

            frame_allocator::deallocate_frame(holder);
            error!(AC97, "Failed to allocate BDL");
            return Err(());
        }

        let mut reference = unsafe { holder.into_slice::<BDesc>(allocated.len()) };
        reference.copy_from_slice(&allocated);
        Ok(Self {
            bdl: SpinLock::new(reference),
            queued_samples: SpinLock::new(PageVec::with_capacity(
                &"AC97_QUEUE",
                allocated.len() * PAGE_SIZE * 2,
            )),
            write_ptr: UnsafeCell::new(0),
        })
    }

    fn queue(&self, queue: &mut PageVec<u8>, data: &[u8]) -> Result<usize, ()> {
        if queue.capacity() - queue.len() == 0 {
            return Err(());
        }
        let to_queue = data.len().min(queue.capacity() - queue.len());
        queue.extend_from_slice(&data[..to_queue]);
        Ok(to_queue)
    }

    fn transfer_direct(&self, mut data: &[u8], bdl: &mut [BDesc]) -> (usize, usize) {
        let max_size = bdl.len() * PAGE_SIZE;
        let size = data.len().min(max_size);
        let mut left = size;
        let mut i = 0;

        while left != 0 {
            let bd = &mut bdl[i];
            let to_copy = left.min(PAGE_SIZE);
            let (src, rest) = data.split_at(to_copy);
            data = rest;

            bd.buf_mut()[..to_copy].copy_from_slice(src);
            bd.samples = (to_copy / 2) as u16;

            // Set on bd init:
            // bdl.config = BDLConf::IOC;
            left -= to_copy;
            i += 1;
        }

        (i, size)
    }

    fn try_deque_pending(&self, bdl: &mut [BDesc]) -> Option<usize> {
        let mut queued = self.queued_samples.lock();
        if queued.len() == 0 {
            return None;
        }
        let queued_bytes = queued.len();

        Some(
            self.transfer_direct(
                queued
                    .drain(..(bdl.len() * PAGE_SIZE).min(queued_bytes))
                    .as_slice(),
                bdl,
            )
            .0,
        )
    }

    pub fn write_data(&self, data: &[u8]) -> Result<(usize, usize), ()> {
        without_interrupts(|| {
            let mut queue = self.queued_samples.lock();
            if let Some(mut bdl) = self.bdl.try_lock() {
                let (desc_count, transferred) = self.transfer_direct(data, &mut bdl);
                if desc_count != 0 {
                    core::mem::forget(bdl);
                }
                return Ok((desc_count, transferred));
            }

            Ok((0, self.queue(&mut queue, data)?))
        })
    }
}

#[derive(Debug)]
pub struct AC97 {
    registers: AC97Registers,
    irq_info: IRQInfo,
    queue: OnceCell<AC97Queue>,
}

impl AC97 {
    pub fn transfer_data(&self, data: &[u8]) -> Result<usize, ()> {
        let (reset_val, transferred) = self.queue.get().unwrap().write_data(data)?;
        unsafe {
            if reset_val != 0 {
                self.registers.reset_transfer(reset_val as u8);
            }
        }
        Ok(transferred)
    }

    unsafe fn init_bdl(&self) -> Result<(), ()> {
        let mut queue = AC97Queue::create()?;
        unsafe { self.registers.init_bdl(*queue.bdl.get_mut())? };
        self.queue
            .set(queue)
            .expect("Reinitilization of AC97 Queue");
        Ok(())
    }
}

unsafe impl Send for AC97 {}
unsafe impl Sync for AC97 {}

impl InterruptReceiver for AC97 {
    fn handle_interrupt(&'static self) {
        let ts = unsafe {
            self.registers
                .read_nabmw(AC97Registers::TRANS_STATUS_REG_OUT)
        };

        let last_valid_ent = (ts & 0b10) != 0;
        let last_transfer = (ts & 0b100) != 0;
        let ioc = (ts & 0b1000) != 0;
        let fifo_error = (ts & 0b10000) != 0;
        if fifo_error {
            warn!(AC97, "FIFO Error");
        }

        let read_ptr = if last_valid_ent {
            31
        } else {
            unsafe { self.registers.read_nabmb(AC97Registers::CURR_ENT_OUT) }
        };

        if ioc || last_valid_ent || last_transfer {
            unsafe {
                let queue = self.queue.get().unwrap_unchecked();
                let write_ptr = &mut *queue.write_ptr.get();

                assert!(queue.bdl.is_locked());
                let mut our_guard = ManuallyDrop::new(queue.bdl.make_guard_unchecked());

                // Prefetch entries as fast as possible.
                if let Some(count) =
                    queue.try_deque_pending(&mut our_guard[*write_ptr as usize..read_ptr as usize])
                {
                    *write_ptr += count as u8;
                }

                if last_valid_ent || last_transfer {
                    if *write_ptr != 0 {
                        self.registers.reset_transfer(*write_ptr as u8);
                    } else {
                        drop(ManuallyDrop::into_inner(our_guard));
                    }

                    *write_ptr = 0;
                }
            }
        }

        unsafe {
            // Clear interrupt.
            self.registers
                .write_nabmw(AC97Registers::TRANS_STATUS_REG_OUT, 0x1C)
        };
    }
}

impl PCIDevice for AC97 {
    const CLASS_SUBCLASS: (u8, u8) = (0x04, 0x01);
    fn create(mut info: crate::drivers::pci::PCIDeviceInfo) -> Self
    where
        Self: Sized,
    {
        let general_header = info.unwrap_general();
        write_ref!(
            general_header.common.command,
            PCICommandReg::BUS_MASTER | PCICommandReg::IO_SPACE
        );
        let bars = general_header.get_bars();
        debug!(AC97, "info: {info:#?}, bars: {bars:#?}");

        let registers = AC97Registers::from_bars(&bars);
        let irq_info = info
            .get_best_irq_info(&[] /* FIXME: We currently don't allocate the BARs */)
            .expect("AC97 must support interrupts");
        Self {
            registers,
            irq_info,
            queue: OnceCell::new(),
        }
    }

    fn start(&'static self) -> bool {
        unsafe {
            self.registers.reset();

            if self.init_bdl().is_err() {
                warn!(AC97, "Failed to initialize BDL, failing...");
                return false;
            }

            self.registers.enable_transfer_int();
        };

        interrupts::register_irq(self.irq_info.clone(), interrupts::IntTrigger::Edge, self);
        audio::register_interface(self);
        true
    }
}

impl AudioCard for AC97 {
    fn info(&self) -> crate::audio::interface::AudioInfo {
        AudioInfo::new(48000, 16, 2)
    }
    fn name(&self) -> &'static str {
        "ac97"
    }

    fn transfer_buf_size(&self) -> usize {
        // FIXME: feels illegal
        without_interrupts(|| self.queue.get().unwrap().queued_samples.lock().len())
    }

    fn transfer_data(&self, data: &[u8]) -> Result<usize, ()> {
        self.transfer_data(data)
    }
}
