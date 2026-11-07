//! `nva-verify` — the cross-cutting verifier (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements the `loadplan` positive read path; #44 implements the
//! `smoke` frame path. `rewrite` / `shader` / `hle` / `device` are known but
//! unavailable and answer exit code 3 (#43).

#![deny(unsafe_code)]

pub mod json;
pub mod loadplan;
// The smoke path is the one place that crosses the C ABI (#37 D7); the rest of
// the verifier stays safe code.
#[allow(unsafe_code)]
pub mod smoke;
