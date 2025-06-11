use bitfield_struct::bitfield;
use lazy_static::lazy_static;

use crate::{
    arch::aarch64::gic::{its::rdbase, GICITS_BASE},
    time,
    utils::locks::Mutex,
    PhysAddr, VirtAddr,
};

#[bitfield(u64)]
pub struct GITSCWriter {
    /// Writing this bit has the following effects:
    ///
    /// 0b0 No effect on the processing commands by the ITS.
    ///
    /// 0b1 Restarts the processing of commands by the ITS if it stalled because of a command
    /// error
    retry: bool,
    #[bits(4)]
    __: (),
    #[bits(15)]
    /// Bits [19:5] of the offset from GITS_CBASER. Bits [4:0] of the offset are zero.
    offset: u16,
    #[bits(44)]
    __: (),
}

impl GITSCWriter {
    pub fn get_ptr() -> *mut Self {
        (*GICITS_BASE + 0x0088).into_ptr::<Self>()
    }

    pub fn read() -> Self {
        unsafe { core::ptr::read_volatile(Self::get_ptr()) }
    }

    pub unsafe fn write(self) {
        unsafe { core::ptr::write_volatile(Self::get_ptr(), self) }
    }
}

#[bitfield(u64)]
pub struct GITSCReader {
    /// Reports whether the processing of commands is stalled because of a command error.
    ///
    /// 0b0 ITS command queue is not stalled because of a command error.
    ///
    /// 0b1 ITS command queue is stalled because of a command error
    stalled: bool,
    #[bits(4)]
    __: (),
    #[bits(15)]
    /// Bits [19:5] of the offset from GITS_CBASER. Bits [4:0] of the offset are zero.
    offset: u16,
    #[bits(44)]
    __: (),
}

impl GITSCReader {
    pub fn get_ptr() -> *mut Self {
        (*GICITS_BASE + 0x0090).into_ptr::<Self>()
    }

    pub fn read() -> Self {
        unsafe { core::ptr::read_volatile(Self::get_ptr()) }
    }

    pub unsafe fn write(self) {
        unsafe { core::ptr::write_volatile(Self::get_ptr(), self) }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum ITSCommandID {
    #[allow(unused)]
    Unknown = 0x0,
    Sync = 0x05,
    MapD = 0x08,
    MapC = 0x09,
    MapI = 0x0B,
}

#[derive(Debug)]
#[repr(C)]
pub struct ITSCommand {
    // dword 0
    id: ITSCommandID,
    d0_par0: u8,
    d0_par1: u8,
    d0_par2: u8,
    d0_par3: u32,
    // dword 1
    dw1: u64,
    // dword 2
    dw2: u64,
    // dword 3
    dw3: u64,
}

impl ITSCommand {
    const RD_BASE_OFF: u8 = 16;
    const VALID_OFF: u8 = 63;

    pub const fn zeroed() -> Self {
        unsafe { core::mem::zeroed() }
    }
    /// Creates a SYNC command targeting GICR for processor 0
    pub fn sync() -> Self {
        Self::new_sync(rdbase())
    }

    /// Creates a SYNC command with rd_base = `rd_base`
    pub const fn new_sync(rd_base: usize) -> Self {
        Self {
            id: ITSCommandID::Sync,
            dw2: (rd_base << Self::RD_BASE_OFF) as u64,
            ..Self::zeroed()
        }
    }

    pub const fn new_mapc(icid: u16, rd_base: usize, valid: bool) -> Self {
        Self {
            id: ITSCommandID::MapC,
            dw2: icid as u64
                | (rd_base << Self::RD_BASE_OFF) as u64
                | ((valid as u64) << Self::VALID_OFF),
            ..Self::zeroed()
        }
    }

    pub const fn new_mapd(size: u8, itt_addr: PhysAddr, valid: bool) -> Self {
        Self {
            id: ITSCommandID::MapD,
            dw1: ((size - 1) & 0xF) as u64,
            dw2: (itt_addr.into_raw() as u64) | ((valid as u64) << Self::VALID_OFF),
            ..Self::zeroed()
        }
    }

    pub const fn new_mapi(device_id: u32, event_id: u32, icid: u16) -> Self {
        Self {
            id: ITSCommandID::MapI,
            d0_par3: device_id,
            dw1: event_id as u64,
            dw2: icid as u64,
            ..Self::zeroed()
        }
    }
}

pub struct ITSCommandQueue {
    commands: *mut [ITSCommand],
    next_index: usize,
}

impl ITSCommandQueue {
    pub fn new(base_addr: VirtAddr, size: usize) -> Self {
        let len = size / size_of::<ITSCommand>();
        let ptr = base_addr.into_ptr::<ITSCommand>();

        let commands = unsafe { core::slice::from_raw_parts_mut(ptr, len) };
        Self {
            commands,
            next_index: 0,
        }
    }

    pub fn init(&mut self) {
        unsafe {
            GITSCWriter::new().with_offset(0).with_retry(false).write();
        }
    }

    fn write_command(&mut self, index: usize, command: ITSCommand) {
        unsafe {
            let command: [u64; 4] = core::mem::transmute(command);

            let ptr = (self.commands as *mut ITSCommand).add(index);
            let dword0 = ptr as *mut u64;
            let dword1 = dword0.add(1);
            let dword2 = dword0.add(2);
            let dword3 = dword0.add(3);

            core::ptr::write_volatile(dword0, command[0]);
            core::ptr::write_volatile(dword1, command[1]);
            core::ptr::write_volatile(dword2, command[2]);
            core::ptr::write_volatile(dword3, command[3]);
        }
    }

    pub fn add_command(&mut self, command: ITSCommand) {
        let index = self.next_index;
        self.write_command(index, command);

        self.next_index += 1;
        if self.next_index >= self.commands.len() {
            self.next_index = 0;
        }

        unsafe {
            GITSCWriter::new()
                .with_retry(false)
                .with_offset((self.next_index) as u16)
                .write();
        }
    }

    /// Waits until all commands has been handled by the ITS
    pub fn poll(&self) {
        let timeout = 1000;
        let start_time = time!();

        while GITSCReader::read().offset() < GITSCWriter::read().offset() {
            let now = time!();

            if now >= start_time + timeout {
                panic!(
                    "time out after {}ms while waiting for the ITS to read commands",
                    time!() - start_time
                );
            }

            core::hint::spin_loop();
        }
    }
}

unsafe impl Send for ITSCommandQueue {}
unsafe impl Sync for ITSCommandQueue {}

lazy_static! {
    pub static ref GITS_COMMAND_QUEUE: Mutex<ITSCommandQueue> = {
        let (addr, size) = super::map_command_quene();
        Mutex::new(ITSCommandQueue::new(addr, size))
    };
}
