.equ CONTEXT_SIZE, 16 * 50
.macro EXCEPTION_VECTOR handler, save_eregs=0

    sub sp, sp, #CONTEXT_SIZE
# store general purpose registers
    stp x0, x1, [sp, #16 * 0]
    stp x2, x3, [sp, #16 * 1]
    stp x4, x5, [sp, #16 * 2]
    stp x6, x7, [sp, #16 * 3]
    stp x8, x9, [sp, #16 * 4]
    stp x10, x11, [sp, #16 * 5]
    stp x12, x13, [sp, #16 * 6]
    stp x14, x15, [sp, #16 * 7]
    stp x16, x17, [sp, #16 * 8]
    stp x18, x19, [sp, #16 * 9]
    stp x20, x21, [sp, #16 * 10]
    stp x22, x23, [sp, #16 * 11]
    stp x24, x25, [sp, #16 * 12]
    stp x26, x27, [sp, #16 * 13]
    stp x28, x29, [sp, #16 * 14]

    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x0, x1, [sp, #16 * 15]

    .if \save_eregs
        mrs x0, esr_el1
        mrs x1, far_el1
        stp x0, x1, [sp, #16 * 16]
    .else
        stp xzr, xzr, [sp, #16 * 16]
    .endif

    mov x0, sp
    mov x1, #CONTEXT_SIZE
    add x1, x1, x0
    # store link register which is x30 and the stack
    stp x30, x1, [sp, #16 * 17]

    # store FPU registers
    stp q0, q1, [sp, #16 * 18]
    stp q2, q3, [sp, #16 * 20]
    stp q4, q5, [sp, #16 * 22]
    stp q6, q7, [sp, #16 * 24]
    stp q8, q9, [sp, #16 * 26]
    stp q10, q11, [sp, #16 * 28]
    stp q12, q13, [sp, #16 * 30]
    stp q14, q15, [sp, #16 * 32]
    stp q16, q17, [sp, #16 * 34]
    stp q18, q19, [sp, #16 * 36]
    stp q20, q21, [sp, #16 * 38]
    stp q22, q23, [sp, #16 * 40]
    stp q24, q25, [sp, #16 * 42]
    stp q26, q27, [sp, #16 * 44]
    stp q28, q29, [sp, #16 * 46]
    stp q30, q31, [sp, #16 * 48]

# call exception handler
    bl \handler
# avoid the 128 byte limit
    b exit_exception
.endm

.text
# restores an interrupt frame at x0 without ereting, and therefore doesn't restore the lr netheir does it restore x0, and x1
.global restore_frame_partial
restore_frame_partial:
# load elr and spsr, these might be modified for example by context switching
    ldp x1, x2, [x0, #16 * 15]
    msr elr_el1, x1
    msr spsr_el1, x2

    ldp x2, x3, [x0, #16 * 1]
    ldp x4, x5, [x0, #16 * 2]
    ldp x6, x7, [x0, #16 * 3]
    ldp x8, x9, [x0, #16 * 4]
    ldp x10, x11, [x0, #16 * 5]
    ldp x12, x13, [x0, #16 * 6]
    ldp x14, x15, [x0, #16 * 7]
    ldp x16, x17, [x0, #16 * 8]
    ldp x18, x19, [x0, #16 * 9]
    ldp x20, x21, [x0, #16 * 10]
    ldp x22, x23, [x0, #16 * 11]
    ldp x24, x25, [x0, #16 * 12]
    ldp x26, x27, [x0, #16 * 13]
    ldp x28, x29, [x0, #16 * 14]
    # load FPU registers
    ldp q0, q1, [x0, #16 * 18]
    ldp q2, q3, [x0, #16 * 20]
    ldp q4, q5, [x0, #16 * 22]
    ldp q6, q7, [x0, #16 * 24]
    ldp q8, q9, [x0, #16 * 26]
    ldp q10, q11, [x0, #16 * 28]
    ldp q12, q13, [x0, #16 * 30]
    ldp q14, q15, [x0, #16 * 32]
    ldp q16, q17, [x0, #16 * 34]
    ldp q18, q19, [x0, #16 * 36]
    ldp q20, q21, [x0, #16 * 38]
    ldp q22, q23, [x0, #16 * 40]
    ldp q24, q25, [x0, #16 * 42]
    ldp q26, q27, [x0, #16 * 44]
    ldp q28, q29, [x0, #16 * 46]
    ldp q30, q31, [x0, #16 * 48]
    ret
.global restore_frame
# restores an interrupt frame at x0 and then erets
restore_frame:
    bl restore_frame_partial
# esr and far doesn't have to be restored
    ldp x30, x1, [x0, #16 * 17]
    mov sp, x1
    ldp x0, x1, [x0, #16 * 0]
    eret

exit_exception:
    mov x0, sp
    b restore_frame

handle_sync_exception_inner:
    EXCEPTION_VECTOR handle_sync_exception, 1

handle_irq_inner:
    EXCEPTION_VECTOR handle_irq, 0

handle_fiq_inner:
    EXCEPTION_VECTOR handle_fiq, 0

handle_serror_inner:
    EXCEPTION_VECTOR handle_serror, 1

.global exc_vector_table
.balign 2048
exc_vector_table:
# the first 4 entries will never be reached
    b .
.balign 0x80
    b .
.balign 0x80
    b .
.balign 0x80
    b .
# Below exceptions happens inside the kernel spaces
# Synchronous Exception
.balign 0x80
    b handle_sync_exception_inner
# IRQ
.balign 0x80
    b handle_irq_inner
# FIQ
.balign 0x80
    b handle_fiq_inner
# SError
.balign 0x80
    b handle_serror_inner
# EL0 Synchorus exceptions
# FIXME: this should terminate the process instead of panicking, fix when signals are added
.balign 0x80
    b handle_sync_exception_inner
# EL0 IRQ
.balign 0x80
    b handle_irq_inner
# EL0 FIQ
.balign 0x80
    b handle_fiq_inner
# EL0 SError
.balign 0x80
    b handle_serror_inner
