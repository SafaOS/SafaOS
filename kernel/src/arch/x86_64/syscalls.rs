// TODO: figure out errors
// for now errors are a big mess
use super::interrupts::InterruptFrame;
use crate::{debug, utils::errors::ErrorStatus};
use core::arch::global_asm;
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
global_asm!(
    "
.section .rodata
syscall_table:
    .quad sysexit
    .quad sysyield
    .quad sysopen
    .quad syswrite
    .quad sysread
    .quad sysclose
    .quad syscreate
    .quad syscreatedir
    .quad sysdiriter_open
    .quad sysdiriter_close
    .quad sysdiriter_next
    .quad syswait
    .quad sysfstat
    .quad sysspawn
    .quad syschdir
    .quad sysgetcwd
    .quad deprecated_syscall
    .quad deprecated_syscall
    .quad syssbrk
    .quad syspspawn
    .quad sysshutdown
    .quad sysreboot
syscall_table_end:

SYSCALL_TABLE_INFO:
    .quad (syscall_table_end - syscall_table) / 8
.section .text
.global syscall_base

syscall_base:
    cmp rax, [SYSCALL_TABLE_INFO]
    jge unsupported
    push rbx
    push rcx
    push rdx
    push rsi
    push rdi
    push rbp
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    call [syscall_table + rax * 8]
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rbp
    pop rdi
    pop rsi
    pop rdx
    pop rcx
    pop rbx
    iretq
unsupported:
    mov rax, {0}
    iretq
", const ErrorStatus::InvaildSyscall as u64
);

extern "x86-interrupt" {
    pub fn syscall_base();
}

#[no_mangle]
extern "C" fn deprecated_syscall(syscall: usize) -> ErrorStatus {
    debug!(
        SyscallContext,
        "deprecated syscall called with syscall {}", syscall
    );
    ErrorStatus::InvaildSyscall
}
