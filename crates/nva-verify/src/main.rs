//! `nva-verify` CLI (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements `loadplan` only. `rewrite` / `shader` / `hle` / `device` are
//! known but not implemented yet: they answer `3` (oracle unavailable) and say
//! why. `smoke` lands with #44.

use std::path::Path;

use nva_verify::loadplan;

/// `3` oracle unavailable (#37 D8, #37 D12).
const EXIT_ORACLE_UNAVAILABLE: i32 = 3;

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, flag, fixture]
            if command.as_str() == "loadplan" && flag.as_str() == "--fixture" =>
        {
            run_loadplan(fixture)
        }
        [command, ..] if is_unimplemented(command) => {
            eprintln!(
                "{command}: not implemented yet, so the oracle is unavailable \
                 (docs/ARCHITECTURE.md §3.8)"
            );
            EXIT_ORACLE_UNAVAILABLE
        }
        _ => {
            eprintln!("usage: nva-verify loadplan --fixture <pack-dir>");
            loadplan::EXIT_MISMATCH
        }
    }
}

/// The four subcommands whose implementation is still pending (#37 D12).
///
/// They are *known but unavailable*, which is exit `3`, not the usage error `2`
/// and never the success `0`.
fn is_unimplemented(command: &str) -> bool {
    matches!(command, "rewrite" | "shader" | "hle" | "device")
}

fn run_loadplan(fixture: &str) -> i32 {
    match loadplan::load(Path::new(fixture)) {
        Ok(plan) => {
            println!("{}", loadplan::summary_line(fixture, &plan));
            0
        }
        Err(error) => {
            eprintln!("loadplan: {error}");
            error.exit_code()
        }
    }
}
