# #30 ABI vertical slice — design

Status: spike, throw-away quality, but the C ABI structs are v1-shaped.

The question: does a guest (C, AAPCS64 / SysV x86-64) get a thunk by name, call
NVN-shaped functions through it, and come back with the right registers, when
the implementation is Rust in the same address space?

Two builds of the *same* code:

1. `x86_64-unknown-linux-gnu` on a GitHub-hosted runner, offscreen Vulkan via
   Mesa lavapipe. Proves table/name/state/draw/returns/stub semantics.
2. `aarch64-unknown-linux-gnu` on a GitHub-hosted arm64 runner, same tests.
   Proves the AAPCS64 / LSE / TLS / x18 / FPCR / stack contract. This is the
   stand-in for the Adreno 725 run when no device is attached.
3. `aarch64-linux-android` cross-build artifact: `libnvn_host.so`, `abi_guest`,
   `run_android.sh`. That is what gets pushed to the Redmi Note 12 Turbo. The
   Android build is not executed by GitHub-hosted CI; the local `adb` run is
   the real-device leg.

## Layout

```
spike/abi-slice/
  Cargo.toml
  build.rs                 # glslangValidator -> OUT_DIR/*.spv
  include/nva_abi.h        # C ABI subset + spike hooks
  include/nvn_guest.h      # spike-local NVN object layouts
  src/lib.rs               # engine, table, NVN impls, C ABI exports
  src/vk.rs                # ash offscreen clear/draw/fence/readback (owner: vk subagent)
  src/thunks_aarch64.S     # direct 32B thunks + gate 48B thunks
  src/thunks_x86_64.S      # lock incq + jmp thunks
  shaders/tri.vert/.frag
  guest/guest.c            # the five assertions + TLS/x18/FPCR/stack probes
  scripts/run_pc.sh
  scripts/run_android.sh
  .github/workflows/ci.yml
```

## Slot table (spike subset; v1 says 523)

`nva_nvn_slot { u64 count; u64 target; }`, 16 B, engine-owned. The thunk
increments `count` and branches to `target`; it touches only x16/x17 (AArch64)
or rax (x86-64), never arg/return registers.

| slot | name | signature | purpose |
|---:|---|---|---|
| 0 | `nvnDeviceBuilderSetDefaults` | `void(Builder*)` | lifecycle |
| 1 | `nvnDeviceInitialize` | `NVNboolean(Device*, const Builder*)` | creates the Vulkan device, returns 1 in x0 |
| 2 | `nvnDeviceGetProcAddress` | `void*(const Device*, const char*)` | name lookup second path |
| 3 | `nvnCommandBufferSetDepthRange` | `void(CB*, float, float)` | **state setter**: f32 in s0/s1 |
| 4 | `nvnCommandBufferClearColor` | `void(CB*, int, const float*, int)` | draw record |
| 5 | `nvnCommandBufferDrawArrays` | `void(CB*, int, int, int)` | draw record |
| 6 | `nvnQueueSubmitCommands` | `void(Queue*, int, const Handle*)` | replay to Vulkan + fence |
| 7 | `nvnQueueFinish` | `void(Queue*)` | fence wait |
| 8 | `nvnSamplerGetLodBias` | `float(const Sampler*)` | **float return**: s0 |
| 9 | `nvnBufferGetAddress` | `u64(const Buffer*)` | **pointer return**: x0 |
| 10 | `nvnCommandBufferDispatchCompute` | `void(CB*, int, int, int)` | **unimplemented -> abort** |
| 11 | `nvnDeviceGetCurrentTimestampInNanoseconds` | `u64(const Device*)` | **unimplemented -> typed 0 + sticky**, not silent |
| 12 | `nvaSpikeTlsProbe` | `u64(void)` | returns TPIDR_EL0 at host entry |
| 13 | `nvaSpikeStackProbe` | `u64(10 x u64)` | stack args (9th/10th) |

`nva_nvn_resolve` returns `nvn_thunk_<slot>` (or `nvn_gate_<slot>` when the table
has `GATE_TLS`). Unknown names return `nvn_unknown_stub` (a real function
returning x0=0) and bump `unknown_resolves`; never NULL.

## TLS gate (AArch64)

Direct 32 B thunk is only safe while guest TLS is **not** in `TPIDR_EL0`.
The gate variant saves guest `TPIDR_EL0` on a 48 B frame, loads the host TLS
captured in `nva_gfx_create`, `blr target`, restores guest TLS, returns.
It deliberately does not touch x0-x7/v0-v7/x8. The gate costs 12 instructions
and 48 B of stack, and it shifts SP, so **stack arguments break** under the
gate. The spike measures both.

Known limitation: `g_host_tls` is a single value captured on the thread that
calls `nva_gfx_create`. The spike is single-threaded; a real runtime needs a
per-thread host TLS value (see #19 §4.1).

## Guest assertions

1. **name lookup**: bootstrap / GetProcAddress / `nva_nvn_resolve` return the
   same non-NULL thunk for the same name; different names differ; unknown name
   is non-NULL and callable (returns 0).
2. **state setter**: `SetDepthRange(0.25f, 0.75f)`; engine sees the exact bits.
3. **draw**: clear red + draw 3-vertex triangle -> lavapipe/Adreno; center
   pixel is green; fence waited; validation errors == 0 (where the layer is
   available).
4. **float/pointer returns**: sampler LOD bias bits (`1.25f`) in s0; buffer
   address (`0x123456789abcdef0`) in x0.
5. **unimplemented**: compute slot aborts (child process checks 134); timestamp
   slot returns 0 and increments `unsupported_calls`, so it is not silent.

AArch64-only, logged separately:

- `TPIDR_EL0` probe: direct + fake guest TLS -> probe sees fake (hazard proven);
  gate + fake guest TLS -> probe sees host TLS and guest TLS is restored.
- x18 sentinel survives a thunk call when the host is built `+reserve-x18`.
- FPCR bits survive a float-return call.
- stack probe: direct passes; gate fails (expected, documented).
- 16 B / 24 B struct returns (x0/x1 vs x8) survive the thunk.

## C ABI v1 points this is meant to settle

- Is `nva_nvn_table` (slots + names + hash + flags) sufficient? Missing
  per-slot signature/stub metadata is a candidate finding.
- Is `nva_nvn_resolve` enough, or is `resolve_ex` needed to report unknown/gate?
- Is a 32 B zero-touch thunk enough, or must v1 specify a TLS gate? What is
  the gate's stack/arg contract?
- Does `nva_host_services` need a `fatal` callback instead of libc `abort`?
- Where does sticky error live (queue vs gfx)? v1 says per-queue, but the spike
  has a gfx-level counter.
