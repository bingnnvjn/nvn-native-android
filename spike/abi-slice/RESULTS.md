# #30 ABI vertical slice — results

CI run: https://github.com/bingnnvjn/nvn-native-android/actions/runs/40169777912
Branch: `spike/abi-slice` (head `cf3ba662a8b14c24f4870911fdc29e7ed82514cb`)
Artifacts: `abi-slice-pc`, `abi-slice-arm64`, `abi-slice-android`.

## Verdict

- **PC x86_64: 5/5 PASS** (Mesa lavapipe, Vulkan 1.4.318, validation layer clean).
- **AArch64 Linux: 5/5 PASS + all probes PASS** (lavapipe arm64, validation clean).
- **Android AArch64: cross-build PASS**; the device leg is not executed here:
  this session has no usable device (`adb` starts, `adb devices` is empty; no
  `/dev/bus/usb`). The artifact and `scripts/run_android.sh` are ready.
- **C ABI v1 is directionally right, but needs four P0 additions** (TLS/thunk
  contract, signature metadata, fatal channel, bootstrap entry); see below.

## Five assertions

| assertion | x86_64 PC | AArch64 Linux | Android build |
|---|---|---|---|
| name_lookup | PASS | PASS | built |
| state_setter | PASS | PASS | built |
| draw_vulkan | PASS, validation clean | PASS, validation clean | built, not run |
| returns (float/pointer) | PASS | PASS | built |
| unimplemented_typed | PASS, abort=134 | PASS, abort=134 | built |

Exact log lines (trimmed):

```
PC:
RESULT {"assertion":"name_lookup","ok":true,...}
RESULT {"assertion":"state_setter","ok":true,"depth_bits_near":"0x3e800000",...}
RESULT {"assertion":"draw_vulkan","ok":true,"backend":"vulkan 1.4.318",
        "device":"llvmpipe (LLVM 20.1.2, 256 bits)","pixel_rgba":"0xff00ff00",
        "validation":"ok"}
RESULT {"assertion":"returns","ok":true,...}
RESULT {"assertion":"unimplemented_typed","ok":true,...}
SUMMARY {"mode":"all","ok":true,"assertions":5,...}

arm64:
RESULT {"assertion":"tls_direct_hazard","ok":true,"probe":"0x...aabc8bf87000",
        "fake":"0x...aabc8bf87000","host_tls":"0x...ff404ee99760"}
RESULT {"assertion":"tls_gate","ok":true,"probe":"0x...ff404ee99760",
        "host_tls":"0x...ff404ee99760","tls_after":"0x...aabc8bf87000"}
RESULT {"assertion":"x18_preserved","ok":true,"required":true}
RESULT {"assertion":"fpcr_preserved","ok":true,"toggled":"0x...c00000"}
RESULT {"assertion":"stack_args_direct","ok":true,"sum":55}
RESULT {"assertion":"stack_args_gate","ok":true,"expected_broken":true,"sum":36}
SUMMARY {"mode":"gate","ok":true,"assertions":10,...}
```

`objdump` of the AArch64/Android `.so` shows `stadd x17, [x16]` at every
`nvn_thunk_N`; the direct thunk is 32 B, the gate is 48 B.

## AArch64 / TLS findings

1. **Direct 32 B thunk is only safe if guest TLS is not in TPIDR_EL0.**
   With a fake guest TPIDR_EL0, the Rust impl reads the fake pointer
   (`probe == fake`, `host_tls != fake`): a direct thunk would make Rust/bionic
   TLS use guest memory. This is the P0 that #19 §4.1 predicted.
2. **48 B gate works**: save guest TPIDR_EL0, load the host TLS captured at
   `nva_gfx_create`, `blr` the impl, restore guest TLS. Probe returns host TLS;
   guest TLS is restored after return. Full five assertions also pass in gate
   mode.
3. **Gate breaks stack arguments**: `sub sp, sp, #48` shifts the caller's
   stack; the 10-arg probe returns 36 instead of 55. Current NVN prototypes have
   at most 8 GPR args (research: 3 functions at 8, no HFA, no stack args), so
   the gate is viable for the current NVN surface **only if v1 states
   `stack_bytes == 0` or the gate copies stack args**.
4. **Per-thread TLS is not solved by the spike.** `g_host_tls` is one global
   captured on the `nva_gfx_create` thread. A real runtime needs a per-thread
   host TLS value; the gate shape is otherwise right.
5. **x18**: with host `+reserve-x18` and guest `-ffixed-x18`, the sentinel
   survives both direct and gate calls. This must be a build contract, not an
   accident.
6. **FPCR**: toggling RMode (FPCR bits 22–23) survives a float-return call.
   Host must not change FPCR/FPSR; if an impl can, the gate saves/restores.

## Research facts used for the stub rules

From `work/research/refs/d2r/nvn_protos.h` (477 typedefs) and
`nvn_slots.tsv` (523 rows):

- 316 void, 44 `NVNboolean` (8-bit in the SDK header), 21 int, 18 size_t,
  12 ptrdiff_t, 5 float, 10 pointer returns, 5 struct-by-value returns
  (2 × 16 B, 3 × `NVNmemoryPool` >16 B via x8).
- 46 slots have no prototype (`523 - 477`); they must be `sig:"unknown"` and
  must not touch x8.
- max GPR args 8 (3 functions); max FPR args 3; no >8 GPR, no HFA, no stack
  args in the current NVN surface.
- `nvnDeviceGetTimestampInNanoseconds` has 2 args; the 1-arg variant is
  `nvnDeviceGetCurrentTimestampInNanoseconds` (spike uses the latter).

## C ABI v1 modification points

### P0-1 thunk/TLS contract

v1 currently says "32 B thunk, zero-touch; TLS handled by rewriter/gate" but
does not define which one is mandatory or the gate's stack/thread contract.

Proposal:

- Add `NVA_NVN_TABLE_GATE_TLS` and `NVA_NVN_TABLE_DIRECT` flags (already
  prototyped in `include/nva_abi.h`).
- Choose one of:
  - **A (recommended for v1)**: rewriter relocates guest TLS off TPIDR_EL0;
    32 B direct thunk; `stack_bytes` can be non-zero. Gate remains a fallback.
  - **B**: gate thunk is the default; v1 must state `stack_bytes == 0` (true
    for today's 477 prototypes) and add per-thread host TLS.
- Add per-slot `stack_bytes` and `arg_gpr`/`arg_fpr` metadata (P0-2) so the
  gate can reject unsupported signatures.
- Add `nva_nvn_set_thread_tls(nva_gfx*, uint64_t host_tls)` or a host service
  returning the current thread's host TLS; one global is not enough.

### P0-2 signature / typed-stub metadata

`nva_nvn_table` has only `{count,target}` + names/hash/flags. v1 promises typed
stubs but provides no type information. Add a versioned `nva_nvn_slot_meta`:

```c
typedef struct nva_nvn_slot_meta {
    uint32_t slot;
    uint16_t ret_class;   /* void/bool32/int32/uint64/float/pointer/struct16/struct_gt16/unknown */
    uint16_t arg_gpr;
    uint16_t arg_fpr;
    uint16_t stack_bytes;
    uint32_t stub_class;  /* IMPL / TYPED_ZERO / ABORT / UNKNOWN */
    uint32_t flags;
} nva_nvn_slot_meta;
```

Append `uint64_t meta` to `nva_nvn_table` under version 2 (do not change the v1
prefix). The 46 unknown slots are `stub_class=UNKNOWN`, `ret_class=unknown`;
the stub writes x0=0 only and never touches x8.

### P0-3 fatal / sticky channel

`abort()` produced SIGABRT 134, but v1 has no host-controlled fatal path and
does not define where sticky error lives (v1 says per-queue; this spike uses a
gfx counter). Add:

```c
void (*fatal)(void *user, uint32_t reason, uint32_t slot, const char *name);
/* in nva_host_services; engine calls it for must-fail slots (compute/indirect) */
```

and a stats accessor with `unsupported_calls`, `unknown_resolves`,
`stub_aborts`, `sticky_error`, `last_serial`. Document that `abort()` is only a
spike fallback; the runtime chooses terminate/tombstone/guest-stop.

### P0-4 bootstrap entry

v1 has no C declaration for `nvnBootstrapLoader` / `nvnDeviceGetProcAddress`;
the spike uses a process-global engine. Add an explicit entry:

```c
uint64_t nva_nvn_bootstrap(nva_gfx *gfx, const char *name);
```

or make the loader own the `nva_gfx*` and pass it to `resolve`. The ambient
singleton is acceptable for one app but should not be the frozen ABI.

### P1-1 `nva_nvn_resolve_ex`

Add `resolve_ex` with `{target, slot, flags}` (`KNOWN/UNKNOWN/GATE`). It is
already implemented and asserted in the spike; `resolve` alone cannot tell the
loader whether a name was found or which thunk shape it got.

### P1-2 build / ABI requirements

- AArch64 host: `+reserve-x18`; guest: `-ffixed-x18`. Add to v1 build contract.
- FPCR/FPSR must not change across NVN calls.
- `NVNboolean` is 8-bit; typed stubs zero w0, callers ignore upper bits.
- `stadd` needs ARMv8.1-LSE; the Redmi 725 supports it. Add a `LSE/LLSC` table
  flag and an `ldxr/stxr` fallback for older devices.

## Not covered / next

- Real Adreno 725: run `scripts/run_android.sh` from a machine with adb and the
  device attached. This session could not (no device).
- Android validation layer: not packaged; only the two Linux lavapipe runs have
  clean validation.
- Full 523-slot table: the spike has 14 slots; the table shape is validated,
  not full coverage.
- Multi-thread TLS, LSE fallback, device-lost, and 8 MRT/present remain out of
  scope per #30.
