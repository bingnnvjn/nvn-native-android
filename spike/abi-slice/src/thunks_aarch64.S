/*
 * AArch64 thunks for the #30 ABI spike.
 *
 * Direct (d2r shape, 32 B): count++ then br target. Touches only x16/x17.
 * Gate (48 B): saves guest TPIDR_EL0, installs the host TLS captured at
 * nva_gfx_create(), calls target, restores guest TLS. Touches x16/x17 and
 * uses a 48-byte stack frame (SP stays 16-byte aligned).
 *
 * NOTE: the gate shifts SP, so stack arguments (>8 GPR args) are *not*
 * visible to the target at their original offsets. NVN's current prototypes
 * max out at 8 GPR args, but C ABI v1 must state this.
 */
    .text
    .hidden g_nvn_slots
    .hidden g_host_tls

    .macro THUNK n
    .globl  nvn_thunk_\n
    .hidden nvn_thunk_\n
    .type   nvn_thunk_\n, %function
nvn_thunk_\n:
    adrp    x16, g_nvn_slots + (\n * 16)
    add     x16, x16, :lo12:g_nvn_slots + (\n * 16)
    mov     x17, #1
    .inst   0xf831021f          // stadd x17, [x16] (ARMv8.1-LSE)
    ldr     x17, [x16, #8]
    br      x17
    .size   nvn_thunk_\n, .-nvn_thunk_\n
    .endm

    .macro GATE n
    .globl  nvn_gate_\n
    .hidden nvn_gate_\n
    .type   nvn_gate_\n, %function
nvn_gate_\n:
    sub     sp, sp, #48
    stp     x29, x30, [sp, #32]
    mrs     x16, tpidr_el0
    str     x16, [sp, #16]
    adrp    x16, g_host_tls
    ldr     x16, [x16, :lo12:g_host_tls]
    msr     tpidr_el0, x16
    adrp    x16, g_nvn_slots + (\n * 16)
    add     x16, x16, :lo12:g_nvn_slots + (\n * 16)
    mov     x17, #1
    .inst   0xf831021f          // stadd x17, [x16] (ARMv8.1-LSE)
    ldr     x17, [x16, #8]
    blr     x17
    ldr     x16, [sp, #16]
    msr     tpidr_el0, x16
    ldp     x29, x30, [sp, #32]
    add     sp, sp, #48
    ret
    .size   nvn_gate_\n, .-nvn_gate_\n
    .endm

    THUNK 0
    THUNK 1
    THUNK 2
    THUNK 3
    THUNK 4
    THUNK 5
    THUNK 6
    THUNK 7
    THUNK 8
    THUNK 9
    THUNK 10
    THUNK 11
    THUNK 12
    THUNK 13

    GATE 0
    GATE 1
    GATE 2
    GATE 3
    GATE 4
    GATE 5
    GATE 6
    GATE 7
    GATE 8
    GATE 9
    GATE 10
    GATE 11
    GATE 12
    GATE 13

    .section .note.GNU-stack,"",%progbits
