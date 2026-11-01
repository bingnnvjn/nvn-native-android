# Glossary

- **guest**: the program that calls NVN. In this repo, the C guest.
- **host**: the runtime layer that owns memory, nvdrv handles, SVCs, and the
  platform. It is not the graphics engine.
- **engine**: the Rust graphics layer that implements the NVN slots and records
  and replays Vulkan commands.
- **thunk**: the small code returned by `nva_nvn_resolve` / `nvnBootstrapLoader`.
  It increments a slot counter and branches to the implementation.
- **slot**: one 16-byte `{count, target}` entry in the engine-owned NVN table.
- **C ABI v1**: the frozen cross-language interface in `include/nva_abi.h`.
- **TLS**: thread-local storage. On AArch64 the hardware pointer is `TPIDR_EL0`.
- **gate**: a 48-byte thunk that saves guest `TPIDR_EL0`, installs host TLS,
  calls the implementation, and restores guest TLS.
- **assertion**: one guest check, printed as a `RESULT {...}` JSON line.
- **APK shell**: the small Android app that runs the same C guest through JNI.
- **import pack**: the per-game data bundle that the PC tooling produces and the
  phone imports at run time. It holds the manifest, the load plan, the code and
  the RomFS blob. It is game data, so it never enters this repository.
- **load plan**: the segment table of an import pack, stored as
  `load-plan.json`. It gives the guest VA, size and permissions of each
  segment, and the entry point. The field schema is owned by L3 and is still
  open (`docs/PIPELINE.md` §10).
- **loadplan**: the `nva-verify` subcommand that reads a load plan.
- **fixture**: synthetic test data committed under `ci/fixtures/`. It has the
  shape of a real artifact but no game bytes; the title IDs are all zero.
