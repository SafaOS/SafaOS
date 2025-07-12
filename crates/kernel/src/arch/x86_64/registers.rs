use core::arch::asm;

pub fn rdmsr(msr: u32) -> usize {
    let (low, high): (u32, u32);
    unsafe {
        asm!(
            "rdmsr",
            in("ecx") msr, out("eax") low, out("edx") high
        );
    }

    (high as usize) << 32 | (low as usize)
}

pub unsafe fn wrmsr(msr: u32, value: u64) {
    let (low, high) = (value as u32, (value >> 32) as u32);
    unsafe {
        asm!(
            "wrmsr",
            in("ecx") msr, in("eax") low, in("edx") high
        );
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StackFrame {
    prev: *mut StackFrame,
    return_addr: *mut u8,
}

impl StackFrame {
    /// Gets the current Frame Pointer from the fp register
    pub unsafe fn get_current<'a>() -> &'a Self {
        unsafe {
            let fp: *mut Self;
            asm!("mov {}, rbp", out(reg) fp);
            &*fp
        }
    }

    /// Gets the return address from the Frame
    pub fn return_ptr(&self) -> *mut u8 {
        self.return_addr
    }

    /// Gets the previous Frame Pointer from this one
    pub unsafe fn prev(&self) -> Option<&Self> {
        let prev = self.prev;

        if prev.is_null() || !prev.is_aligned() || (prev as usize) < 0x1000 {
            return None;
        }
        unsafe { Some(&*prev) }
    }
}
