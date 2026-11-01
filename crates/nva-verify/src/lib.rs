//! `nva-verify` — the cross-cutting verifier (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements the `loadplan` positive read path. The `rewrite` / `shader` /
//! `hle` / `device` / `smoke` subcommands land in later tickets.

#![forbid(unsafe_code)]

pub mod json;
pub mod loadplan;
