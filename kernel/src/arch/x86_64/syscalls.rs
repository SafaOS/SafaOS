// TODO: figure out errors
// for now errors are a big mess
use super::interrupts::InterruptFrame;
use crate::syscalls;
use core::arch::asm;
/// used sometimes for debugging syscalls
#[allow(dead_code)]
#[derive(Debug, Clone)]
#[repr(C)]
pub struct SyscallContext {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub frame: InterruptFrame,
}

#[no_mangle]
#[naked]
pub extern "x86-interrupt" fn syscall_base() {
    unsafe {
        asm!(
            "push rbx",
            "push rcx",
            "push rdx",
            "push rsi",
            "push rdi",
            "push rbp",
            "push r8",
            "push r9",
            "push r10",
            "push r11",
            "push r12",
            "push r13",
            "push r14",
            "push r15",
            "call syscall_base_mapper",
            "pop r15",
            "pop r14",
            "pop r13",
            "pop r12",
            "pop r11",
            "pop r10",
            "pop r9",
            "pop r8",
            "pop rbp",
            "pop rdi",
            "pop rsi",
            "pop rdx",
            "pop rcx",
            "pop rbx",
            "iretq",
            options(noreturn)
        )
    }
}

// FIXME: this is extremely unstable and fragile
// FIXME: returns usize to make sure rax is used instead of ax
#[no_mangle]
pub extern "C" fn syscall_base_mapper(a: usize, b: usize, c: usize, d: usize, e: usize) -> usize {
    let number: usize;
    unsafe {
        asm!("mov {}, rax", out(reg) number);
    }
    syscalls::syscall(number as u16, a, b, c, d, e) as usize
}
