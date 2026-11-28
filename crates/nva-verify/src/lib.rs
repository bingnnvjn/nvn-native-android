//! `nva-verify` — the cross-cutting verifier (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements the `loadplan` positive read path; #44 implements the
//! `smoke` frame path; #94 adds `statslog`, the reader for the device's
//! `nva-stats` snapshots; #96 adds `device`, the M0b gate. `rewrite` /
//! `shader` / `hle` are known but unavailable and answer exit code 3 (#43).

#![deny(unsafe_code)]

pub mod device;
pub mod json;
pub mod loadplan;
// The smoke path is the one place that crosses the C ABI (#37 D7); the rest of
// the verifier stays safe code.
#[allow(unsafe_code)]
pub mod smoke;
pub mod statslog;
