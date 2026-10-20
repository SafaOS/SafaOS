pub type IrqNum = u32;

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum IrqStatus {
    Handled,
    None,
}

pub type IrqRoutine = extern "C" fn(irq: IrqNum, private: *mut ()) -> IrqStatus;
