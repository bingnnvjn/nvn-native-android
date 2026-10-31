# #30 ABI vertical slice (spike)

Guest calls NVN-shaped functions by name; the implementation is Rust in the same
process; a `{count,target}` thunk is the only code between them. The point is to
find where the C ABI v1 contract breaks, not to build an engine.

- Design: [DESIGN.md](DESIGN.md)
- C ABI subset: `include/nva_abi.h`
- Spike-local NVN object layouts: `include/nvn_guest.h`
- Results: [RESULTS.md](RESULTS.md) (filled after the CI runs)
- Issue: https://github.com/bingnnvjn/nvn-native-android/issues/30

## What runs where

| leg | runner | Vulkan | proves |
|---|---|---|---|
| PC x86_64 | `ubuntu-24.04` | Mesa lavapipe | table/name lookup, state setter, clear+draw+fence+readback, float/pointer returns, typed stub, abort |
| AArch64 Linux | `ubuntu-24.04-arm` | lavapipe arm64 | same five + LSE thunk, TLS direct/gate, x18 (`+reserve-x18`), FPCR, stack args, struct returns |
| Android AArch64 | CI cross-build + APK | Adreno 725 | APK installed on the phone: all 9/9, gate 10/10, abort SIGABRT |

No 8 MRT, no block-linear, no present, no BotW. Builds are all in CI; the
device leg is only `adb`.

## Checks

```sh
scripts/check.sh       # C layout, slot table vs nvn-table.json, JSON, bash -n
scripts/jni_smoke.sh   # Linux JNI runtime: all, gate, abort (needs JDK + lavapipe)
```

CI runs both in the PC job. `scripts/check.sh` is cheap and needs no Vulkan.
The C layout check compiles `include/abi_layout_check.c`; do not remove it.

## Run

CI: push to `spike/abi-slice`, then download artifacts:

```sh
gh run download -R bingnnvjn/nvn-native-android --name abi-slice-pc
gh run download -R bingnnvjn/nvn-native-android --name abi-slice-arm64
gh run download -R bingnnvjn/nvn-native-android --name abi-slice-android
```

Real device (Redmi Note 12 Turbo, arm64-v8a):

```sh
tar xzf abi-slice-android.tar.gz -C dist/android
spike/abi-slice/scripts/run_android.sh        # uses adb; no root
```

Phone-only APK (no adb, no Termux):

- Artifact `abi-slice-apk` (`abi-slice.apk`, arm64-v8a, debug-signed v2/v3).
- Install it, enable "install unknown apps" for the file manager/browser.
- Open **ABI Slice**. It runs `all` automatically.
- Buttons: `Run all`, `Run gate`, `Abort test`.
- The result is on the screen and in logcat with tag `abi-slice`.
- The APK is a JNI shell around the same `guest.c`; it does not change the C ABI.
- Verified on Redmi Note 12 Turbo (Android 15, Adreno 725,
  Vulkan 1.1.128): `all` 9/9, `gate` 10/10, `abort` SIGABRT=6.

Local Linux smoke (needs glslang-tools + lavapipe):

```sh
spike/abi-slice/scripts/run_pc.sh
```

## Assertions

The guest prints one `RESULT {...}` JSON line per assertion and a final
`SUMMARY {...}`; the exit code is authoritative.

1. `name_lookup` — bootstrap / `nva_nvn_resolve` / `nvnDeviceGetProcAddress`
   agree; unknown names return a callable zero stub.
2. `state_setter` — `nvnCommandBufferSetDepthRange(0.25f, 0.75f)` arrives with
   exact float bits.
3. `draw_vulkan` — clear red, draw a fullscreen triangle, fence wait, center
   pixel green; validation errors 0 when the layer is present.
4. `returns` — `nvnSamplerGetLodBias` in s0 and `nvnBufferGetAddress` in x0.
5. `unimplemented_typed` — timestamp returns 0 **and** bumps the sticky
   counter; `unimplemented` mode calls compute and must SIGABRT.

AArch64-only evidence: `tls_direct_hazard`, `tls_gate`, `x18`, `fpcr`,
`stack_args`, `struct_returns`.

## C ABI v1 candidates this spike checks

- table shape (`nva_nvn_table`, `nva_nvn_slot`, names/hash/flags);
- `nva_nvn_resolve` vs `nva_nvn_resolve_ex` (unknown/gate reporting);
- typed stub vs abort vs sticky error, and who owns the fatal path;
- 32 B direct thunk vs 48 B TLS gate, stack/arg/return contract;
- `nva_host_services` ordering and whether a `fatal` callback is needed.
