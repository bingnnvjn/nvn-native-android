# #30 ABI vertical slice — results

Status: x86_64 PC leg verified locally with Mesa lavapipe (Vulkan 1.3.255,
llvmpipe, validation layer clean). AArch64 Linux + Android legs are filled from
the GitHub Actions runs; Android real-device execution is the `adb` leg.

## Five assertions

| # | assertion | x86_64 lavapipe | AArch64 Linux | Adreno 725 |
|---:|---|---|---|---|
| 1 | name_lookup | PASS | pending CI | pending adb |
| 2 | state_setter | PASS | pending CI | pending adb |
| 3 | draw_vulkan | PASS (validation clean) | pending CI | pending adb |
| 4 | returns | PASS | pending CI | pending adb |
| 5 | unimplemented_typed + abort | PASS (134) | pending CI | pending adb |

x86_64 evidence (local, `VK_LAYER_PATH` + lavapipe + validation):

```
RESULT {"assertion":"name_lookup","ok":true,...}
RESULT {"assertion":"state_setter","ok":true,...}
RESULT {"assertion":"draw_vulkan","ok":true,"backend":"vulkan 1.3.255",
        "device":"llvmpipe (LLVM 15.0.7, 256 bits)","pixel_rgba":"0xff00ff00",
        "validation":"ok"}
RESULT {"assertion":"returns","ok":true,...}
RESULT {"assertion":"unimplemented_typed","ok":true,...}
SUMMARY {"mode":"all","ok":true,"assertions":5,"failed_assertions":0,...}
```

`unimplemented` mode aborts with SIGABRT (exit 134).

## C ABI v1 points (to be confirmed by the AArch64 leg)

See the issue #30 comment for the final numbered list. Candidate modifications
already exercised by the code:

- `nva_nvn_resolve_ex` (known/unknown/gate flags) — implemented and asserted.
- `NVA_NVN_TABLE_GATE_TLS` + 48 B gate thunk — implemented; gate shifts SP.
- typed stub metadata (return class / x8 / abort) — hard-coded for the 14 slots.
- `nva_host_services.fatal` — not in v1; spike uses `abort()`.
