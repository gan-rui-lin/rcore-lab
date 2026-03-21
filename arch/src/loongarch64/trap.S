/*
 * LoongArch64 trap entry/return assembly
 *
 * This file centralizes low-level trap/user-transition assembly to keep
 * Rust trap logic focused on classification/policy.
 *
 * Symbol contract with Rust (`trap.rs`):
 *   - user_vec
 *   - user_restore
 *   - trap_vector_base
 *   - tlb_fill
 *   - user_rw_exception_entry
 *   - try_write_user
 *   - try_read_user
 */

    .altmacro

/* KScratch slots used by trap transition */
    .equ KSAVE_KSP,  0x30
    .equ KSAVE_CTX,  0x31
    .equ KSAVE_USP,  0x32

/* Save user GPRs/CSR fields into TrapFrame at current $sp */
    .macro SAVE_REGS
        st.d    $ra, $sp,  1*8
        st.d    $tp, $sp,  2*8
        st.d    $a0, $sp,  4*8
        st.d    $a1, $sp,  5*8
        st.d    $a2, $sp,  6*8
        st.d    $a3, $sp,  7*8
        st.d    $a4, $sp,  8*8
        st.d    $a5, $sp,  9*8
        st.d    $a6, $sp, 10*8
        st.d    $a7, $sp, 11*8
        st.d    $t0, $sp, 12*8
        st.d    $t1, $sp, 13*8
        st.d    $t2, $sp, 14*8
        st.d    $t3, $sp, 15*8
        st.d    $t4, $sp, 16*8
        st.d    $t5, $sp, 17*8
        st.d    $t6, $sp, 18*8
        st.d    $t7, $sp, 19*8
        st.d    $t8, $sp, 20*8
        st.d    $r21,$sp, 21*8
        st.d    $fp, $sp, 22*8
        st.d    $s0, $sp, 23*8
        st.d    $s1, $sp, 24*8
        st.d    $s2, $sp, 25*8
        st.d    $s3, $sp, 26*8
        st.d    $s4, $sp, 27*8
        st.d    $s5, $sp, 28*8
        st.d    $s6, $sp, 29*8
        st.d    $s7, $sp, 30*8
        st.d    $s8, $sp, 31*8

        csrrd   $t0, KSAVE_USP
        st.d    $t0, $sp,  3*8

        csrrd   $t0, 0x1
        st.d    $t0, $sp, 8*32      /* prmd */

        csrrd   $t0, 0x6
        st.d    $t0, $sp, 8*33      /* era */
    .endm

/* Restore TrapFrame -> registers and switch back to user stack */
    .macro LOAD_REGS
        ld.d    $t0, $sp, 32*8
        csrwr   $t0, 0x1            /* PRMD */

        ld.d    $t0, $sp, 33*8
        csrwr   $t0, 0x6            /* ERA */

        ld.d    $ra, $sp, 1*8
        ld.d    $tp, $sp, 2*8
        ld.d    $a0, $sp, 4*8
        ld.d    $a1, $sp, 5*8
        ld.d    $a2, $sp, 6*8
        ld.d    $a3, $sp, 7*8
        ld.d    $a4, $sp, 8*8
        ld.d    $a5, $sp, 9*8
        ld.d    $a6, $sp, 10*8
        ld.d    $a7, $sp, 11*8
        ld.d    $t0, $sp, 12*8
        ld.d    $t1, $sp, 13*8
        ld.d    $t2, $sp, 14*8
        ld.d    $t3, $sp, 15*8
        ld.d    $t4, $sp, 16*8
        ld.d    $t5, $sp, 17*8
        ld.d    $t6, $sp, 18*8
        ld.d    $t7, $sp, 19*8
        ld.d    $t8, $sp, 20*8
        ld.d    $r21,$sp, 21*8
        ld.d    $fp, $sp, 22*8
        ld.d    $s0, $sp, 23*8
        ld.d    $s1, $sp, 24*8
        ld.d    $s2, $sp, 25*8
        ld.d    $s3, $sp, 26*8
        ld.d    $s4, $sp, 27*8
        ld.d    $s5, $sp, 28*8
        ld.d    $s6, $sp, 29*8
        ld.d    $s7, $sp, 30*8
        ld.d    $s8, $sp, 31*8

        ld.d    $sp, $sp, 3*8
    .endm

    .section .text

/* User trap vector: save user regs then return to Rust kernel context */
    .global user_vec
    .type user_vec,@function
user_vec:
    csrrd   $sp,  KSAVE_CTX
    SAVE_REGS

    csrrd   $sp,  KSAVE_KSP
    ld.d    $ra,  $sp, 0*8
    ld.d    $tp,  $sp, 1*8
    ld.d    $r21, $sp, 2*8
    ld.d    $s9,  $sp, 3*8
    ld.d    $s0,  $sp, 4*8
    ld.d    $s1,  $sp, 5*8
    ld.d    $s2,  $sp, 6*8
    ld.d    $s3,  $sp, 7*8
    ld.d    $s4,  $sp, 8*8
    ld.d    $s5,  $sp, 9*8
    ld.d    $s6,  $sp, 10*8
    ld.d    $s7,  $sp, 11*8
    ld.d    $s8,  $sp, 12*8
    addi.d  $sp,  $sp, 13*8
    ret

/* Restore user context then return to user mode via ERTN */
    .global user_restore
    .type user_restore,@function
user_restore:
    addi.d  $sp,  $sp, -13*8
    st.d    $ra,  $sp, 0*8
    st.d    $tp,  $sp, 1*8
    st.d    $r21, $sp, 2*8
    st.d    $s9,  $sp, 3*8
    st.d    $s0,  $sp, 4*8
    st.d    $s1,  $sp, 5*8
    st.d    $s2,  $sp, 6*8
    st.d    $s3,  $sp, 7*8
    st.d    $s4,  $sp, 8*8
    st.d    $s5,  $sp, 9*8
    st.d    $s6,  $sp, 10*8
    st.d    $s7,  $sp, 11*8
    st.d    $s8,  $sp, 12*8

    csrwr   $sp, KSAVE_KSP
    move    $sp, $a0
    csrwr   $a0, KSAVE_CTX

    LOAD_REGS
    ertn

/*
 * Main exception entry:
 *   - user-originated trap  -> jump user_vec
 *   - kernel-originated trap -> save TrapFrame, call Rust classifier, restore, ertn
 */
    .global trap_vector_base
    .type trap_vector_base,@function
    .balign 4096
trap_vector_base:
    csrwr   $sp, KSAVE_USP
    csrrd   $sp, 0x1
    andi    $sp, $sp, 0x3
    bnez    $sp, user_vec

    /* Fast path for PageModifyFault (ecode=4): set D bit and return */
    csrrd   $sp, 0x5
    srli.d  $sp, $sp, 16
    andi    $sp, $sp, 0x3f
    ori     $t0, $zero, 4
    bne     $sp, $t0, 1f

    tlbsrch
    tlbrd
    ori     $t0, $zero, 0x2
    csrrd   $sp, 0x0c
    or      $sp, $sp, $t0
    csrwr   $sp, 0x0c
    csrrd   $sp, 0x0d
    or      $sp, $sp, $t0
    csrwr   $sp, 0x0d
    tlbwr
    csrrd   $sp, KSAVE_USP
    ertn

1:
    csrrd   $sp, KSAVE_USP
    addi.d  $sp, $sp, -{trapframe_size}
    SAVE_REGS

    move    $a0, $sp
    bl      {trap_handler}

    LOAD_REGS
    ertn

/* TLB refill exception entry */
    .global tlb_fill
    .type tlb_fill,@function
    .balign 4096
tlb_fill:
    .equ LA_CSR_PGD,           0x1b
    .equ LA_CSR_TLBRSAVE,      0x8b

    csrwr   $t0, LA_CSR_TLBRSAVE
    csrrd   $t0, LA_CSR_PGD
    lddir   $t0, $t0, 3
    lddir   $t0, $t0, 1
    ldpte   $t0, 0
    ldpte   $t0, 1
    tlbfill
    csrrd   $t0, LA_CSR_TLBRSAVE
    ertn

/* User RW probe exception entry (for try_read_user/try_write_user) */
    .global user_rw_exception_entry
    .type user_rw_exception_entry,@function
    .balign 4096
user_rw_exception_entry:
    csrrd   $a0, 0x6
    addi.d  $a0, $a0, 4
    csrwr   $a0, 0x6
    ori     $a0, $zero, 1
    csrrd   $a1, 0x5
    ertn

    .global try_write_user
    .type try_write_user,@function
    .balign 4096
try_write_user:
    move $a2, $a0
    move $a0, $zero
    ld.b $a1, $a2, 0
    st.b $a1, $a2, 0
    jr $ra

    .global try_read_user
    .type try_read_user,@function
    .balign 4096
try_read_user:
    move $a1, $a0
    move $a0, $zero
    ld.b $a1, $a1, 0
    jr $ra
