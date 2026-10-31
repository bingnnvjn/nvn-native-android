# #30 ABI vertical slice — results

CI run: https://github.com/bingnnvjn/nvn-native-android/actions/runs/41067076837
Branch: `spike/abi-slice` (head `699725727f82ea37aa4cbec52f237f57a6a9569a`)
Artifacts: `abi-slice-pc`, `abi-slice-arm64`, `abi-slice-android`.

## Verdict

- **PC x86_64: 5/5 PASS** (Mesa lavapipe, Vulkan 1.4.318, validation layer clean).
- **AArch64 Linux: 5/5 PASS + all probes PASS** (lavapipe arm64, validation clean).
- **Android AArch64: cross-build PASS + APK PASS + real device PASS**.
  Redmi Note 12 Turbo (`23049RAD8C`, Android 15, Adreno 725, Vulkan 1.1.128):
  `all` 9/9 assertions, `gate` 10/10, `abort` child SIGABRT=6. The APK is the
  delivery form that was installed on the phone. Since 2026-10-31 the same APK
  also carries the Khronos validation layer: `validation_available:1` and both
  modes report 0 validation errors (see "Adreno 725: Vulkan validation layer").
- **C ABI v1 is directionally right, but needs four P0 additions** (TLS/thunk
  contract, signature metadata, fatal channel, bootstrap entry); see below.

## Five assertions

| assertion | x86_64 PC | AArch64 Linux | Adreno 725 |
|---|---|---|---|
| name_lookup | PASS | PASS | PASS |
| state_setter | PASS | PASS | PASS |
| draw_vulkan | PASS, validation clean | PASS, validation clean | PASS, validation clean (layer 1.4.350.0) |
| returns (float/pointer) | PASS | PASS | PASS |
| unimplemented_typed | PASS, abort=134 | PASS, abort=134 | PASS, abort=SIGABRT 6 |

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

## Adreno 725 real device (2026-10-30)

Device from the APK screen: `23049RAD8C / Android 15`, `arm64-v8a`.
Vulkan: `Adreno (TM) 725`, `vulkan 1.1.128`, `validation=false` (no layer).

- `all`: 9 assertions, 0 failed checks; the five required assertions are
  `name_lookup`, `state_setter`, `draw_vulkan` (pixel `0xff00ff00`),
  `returns`, `unimplemented_typed`.
- `gate`: 10 assertions, 0 failed checks, `tls_gate:1`.
  `tls_gate` probe = host TLS; guest TLS restored after return.
- `abort`: child process `SIGABRT=6`; the app process stayed alive.
- `tls_direct_hazard`: expected informational proof that direct thunk sees
  guest TPIDR_EL0 when it is installed.
- `x18` sentinel, `fpcr` RMode, `stack_args_direct` (sum 55) all pass.
- No app crash; only the intended child abort.

Screenshots: [all](evidence/device-all.png), [gate](evidence/device-gate.png),
[abort](evidence/device-abort.png).

## Adreno 725: Vulkan validation layer (2026-10-31, #31)

The phone image has no validation layer. The layer is loaded from the APK's own
native library directory:

- Layer: Khronos `Vulkan-ValidationLayers` **1.4.350.0**, Android `arm64-v8a`
  binary (`android-binaries-1.4.350.0.tar.gz`, NDK 27.3, API 26+), from
  <https://github.com/KhronosGroup/Vulkan-ValidationLayers/releases/tag/vulkan-sdk-1.4.350.0>.
  The `.so` sits in the APK at `lib/arm64-v8a/libVkLayer_khronos_validation.so`;
  it is not committed to the repository.
- Settings (per-app, persist across reboots):
  `enable_gpu_debug_layers=1`, `gpu_debug_app=com.bingnnvjn.abislice`,
  `gpu_debug_layer_app=com.bingnnvjn.abislice`,
  `gpu_debug_layers=VK_LAYER_KHRONOS_validation`.
- Loader log: `Loaded layer VK_LAYER_KHRONOS_validation`, then
  `Vulkan device: Adreno (TM) 725 (vulkan 1.1.128) validation=true`.

Result:

| mode | assertions | ok | validation_available | validation_errors |
|---|---|---|---|---|
| all | 9 | true | 1 | 0 |
| gate | 10 | true | 1 | 0 |

```
I vulkan   : Loaded layer VK_LAYER_KHRONOS_validation
I abi-slice: Vulkan device: Adreno (TM) 725 (vulkan 1.1.128) validation=true
I abi-slice: submit: serial=1 draw=(0,3) clear=[1.00,0.00,0.00,1.00] pixel=0xff00ff00 vk_errors=0
I abi-slice: SUMMARY {"mode":"all","ok":true,"assertions":9,"failed_assertions":0,"failed_checks":0,"tls_gate":0,"validation_available":1,"validation_errors":0}
I abi-slice: SUMMARY {"mode":"gate","ok":true,"assertions":10,"failed_assertions":0,"failed_checks":0,"tls_gate":1,"validation_available":1,"validation_errors":0}
```

`logcat` has no `VUID-` and no `Validation Error` line for either run. `SUMMARY`
now prints `validation_errors` directly (`guest.c`), so a gate can read it
instead of scraping the `draw_vulkan` detail string.

Driver message caveat: the Adreno driver also emits `VKDBGUTILWARN003`
("render pass is not qualified for multipass") through `VK_EXT_debug_utils`,
with an **empty** `messageIdName`, but severity `ERROR` and type `VALIDATION`.
It is a driver hint, not a validation-layer finding. `src/vk.rs` therefore
counts a message as a validation error only when it is not a `VKDBGUTIL*`
message; driver messages are logged as `[vk driver msg]` and never increment
`validation_errors`. Any real VUID from the layer still fails the assertion,
so this does not hide layer findings.

Reproduce: build the APK with the layer `.so` in `lib/arm64-v8a`, install it,
set the four `gpu_debug_*` settings above, then launch the app and press
`RUN GATE`.

## Phone-only APK

The `abi-slice-apk` artifact is a minimal Android shell around the same
`guest.c`:

- arm64-v8a, `minSdk 26`, `targetSdk 34`, debug-signed (APK signature v2/v3).
- `System.loadLibrary("nvn_host")` then `abi_slice_jni`; JNI redirects
  stdout/stderr to a cache file and returns the log to Java.
- On launch it runs `all` automatically. Buttons run `gate` and a child-process
  `unimplemented` abort test. The output is on the screen and in logcat
  (`tag=abi-slice`).
- A Linux JNI smoke (same `jni.c` + `guest.c`, x86_64 JDK + lavapipe) passed
  `all` 5/5, `gate` 5/5, and `abort` with child SIGABRT=6.

The APK is not a full app. It has no icon, no resources, and no UI beyond the
test. It does not change the C ABI.

## Not covered / next

- Android validation layer: loaded from the APK for #31 (above); the layer
  binary is downloaded from the Khronos release at build time, not committed.
- The phone run used the APK/JNI path; the `adb` executable path
  (`scripts/run_android.sh`) is built but was not used on this device.
- Full 523-slot table: the spike has 14 slots; the table shape is validated,
  not full coverage.
- Multi-thread TLS, LSE fallback, device-lost, and 8 MRT/present remain out of
  scope per #30.
