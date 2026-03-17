.altmacro
.macro SAVE reg, offset
    sd \reg, \offset*8(sp)
.endm

.macro LOAD reg, offset
    ld \reg, \offset*8(sp)
.endm

.macro SAVE_N n
    sd x\n, \n*8(sp)
.endm

.macro LOAD_N n
    ld x\n, \n*8(sp)
.endm

.macro SAVE_GENERAL_REGS
    SAVE x1, 1
    csrr x1, sscratch
    SAVE x1, 2
    .set n, 3
    .rept 29
        SAVE_N %n
        .set n, n + 1
    .endr

    csrr t0, sstatus
    csrr t1, sepc
    SAVE t0, 32
    SAVE t1, 33
.endm

.macro LOAD_GENERAL_REGS
    LOAD t0, 32
    LOAD t1, 33
    csrw sstatus, t0
    csrw sepc, t1

    LOAD x1, 1
    .set n, 3
    .rept 29
        LOAD_N %n
        .set n, n + 1
    .endr
    LOAD x2, 2
.endm

// -----------------------------------------------------------------------------
// Compatibility trampoline stub
// -----------------------------------------------------------------------------
.section .text.trampoline
.globl strampoline
.globl __alltraps
.globl __restore
.align 2
strampoline:
__alltraps:
    j kernelvec
__restore:
    j user_restore

// -----------------------------------------------------------------------------
// Unified trap vector: kernelvec/uservec + sscratch stack switch
// -----------------------------------------------------------------------------
.section .text
.globl kernelvec
.align 2
kernelvec:
    csrrw sp, sscratch, sp
    bnez sp, uservec
    csrr sp, sscratch

    addi sp, sp, -34*8
    SAVE_GENERAL_REGS
    csrw sscratch, x0

    mv a0, sp
    call trap_from_kernel

    LOAD_GENERAL_REGS
    sret

.globl user_restore
.align 2
user_restore:
    // Critical section: avoid taking S-mode interrupts before sret,
    // otherwise kernelvec may misclassify and clobber TrapContext.
    csrci sstatus, 2

    // Save kernel caller context on kernel stack (callee-saved + tp/gp/ra).
    addi sp, sp, -18*8
    csrr t0, satp
    sd t0, 8*0(sp)
    sd sp, 8*1(sp)
    sd gp, 8*2(sp)
    sd tp, 8*3(sp)
    sd s0, 8*4(sp)
    sd s1, 8*5(sp)
    sd s2, 8*6(sp)
    sd s3, 8*7(sp)
    sd s4, 8*8(sp)
    sd s5, 8*9(sp)
    sd s6, 8*10(sp)
    sd s7, 8*11(sp)
    sd s8, 8*12(sp)
    sd s9, 8*13(sp)
    sd s10, 8*14(sp)
    sd s11, 8*15(sp)
    sd a0, 8*16(sp)
    sd ra, 8*17(sp)

    // a0 = &TrapContext
    // Save kernel stack pointer as high-half alias so uservec can access it
    // before switching satp back to kernel.
    li t1, 0xffffffc000000000
    or t1, sp, t1
    sd t1, 8*0(a0)
    csrw sscratch, a0
    mv sp, a0

    // Switch to user page table right before sret.
    csrw satp, a1
    sfence.vma
    LOAD_GENERAL_REGS
    sret

.globl uservec
.align 2
uservec:
    SAVE_GENERAL_REGS
    csrw sscratch, x0

    mv a0, sp
    ld sp, 0*8(a0)
    sd x0, 0*8(a0)
    // Switch back to kernel page table before returning into kernel Rust code.
    ld t0, 8*0(sp)
    csrw satp, t0
    sfence.vma

    // Restore kernel caller context.
    ld gp, 8*2(sp)
    ld tp, 8*3(sp)
    ld s0, 8*4(sp)
    ld s1, 8*5(sp)
    ld s2, 8*6(sp)
    ld s3, 8*7(sp)
    ld s4, 8*8(sp)
    ld s5, 8*9(sp)
    ld s6, 8*10(sp)
    ld s7, 8*11(sp)
    ld s8, 8*12(sp)
    ld s9, 8*13(sp)
    ld s10, 8*14(sp)
    ld s11, 8*15(sp)
    ld ra, 8*17(sp)
    ld sp, 8*1(sp)

    addi sp, sp, 18*8
    ret
