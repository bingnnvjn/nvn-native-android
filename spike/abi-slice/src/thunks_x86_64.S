/*
 * x86_64 thunks for the #30 ABI spike. Same count/target table shape as
 * AArch64; `lock incq` is the x86 stand-in for stadd. Touches only rax/flags.
 */
    .text
    .hidden g_nvn_slots

    .macro THUNK n
    .globl  nvn_thunk_\n
    .hidden nvn_thunk_\n
    .type   nvn_thunk_\n, @function
nvn_thunk_\n:
    lock inc qword ptr [rip + g_nvn_slots + (\n * 16)]
    jmp qword ptr [rip + g_nvn_slots + (\n * 16) + 8]
    .size   nvn_thunk_\n, .-nvn_thunk_\n
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

    .section .note.GNU-stack,"",%progbits
