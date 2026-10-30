# Coding standards for review

The review agent reads this file. The implementation agent does not need it in
context unless it is changing the C ABI.

## ABI slot change

A slot change touches all of these files. Check each one:

1. `spike/abi-slice/src/lib.rs` — `SLOT_NAMES`, `SLOT_COUNT`, implementation.
2. `spike/abi-slice/include/nva_abi.h` and `include/nvn_guest.h` — C prototype.
3. `spike/abi-slice/include/abi_layout_check.c` — C layout asserts.
4. `spike/abi-slice/nvn-table.json` — name, args, return class, stub class.
5. `spike/abi-slice/guest/guest.c` — resolve, call, assert.
6. `spike/abi-slice/DESIGN.md` and `RESULTS.md` — contract and evidence.

Mechanical checks live in `scripts/check.sh` and CI. Do not duplicate them in
review prose.

## Stub classes

- `impl`: a real implementation exists.
- `typed_zero`: default for an unimplemented query. Return `x0=0`; do not touch
  `x8` unless the signature is known.
- `abort`: only a command that must not silently succeed. Use the host fatal
  path; do not return a fake success.
- `unknown`: a slot with no prototype. `x0=0` only; never touch `x8`.

## Android / JNI

- Load `nvn_host` before `abi_slice_jni`.
- Do not use Java lambdas in the no-Gradle APK build; `android.jar` on the
  javac bootclasspath has no `LambdaMetafactory`.
- Do not link `-lpthread` on Android; pthread is in libc.
- Create and destroy the engine on every run. Do not leave `GLOBAL` set.

## Judgement calls

- Choose direct thunk only when guest TLS is not in `TPIDR_EL0`; otherwise use
  the gate.
- The gate moves SP. Reject `stack_bytes != 0` or copy stack arguments.
- Do not report "validation clean" unless the validation layer actually loaded.
