//! `nva-verify` CLI (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements `loadplan` only. The `rewrite` / `shader` / `hle` / `device` /
//! `smoke` subcommands land in later tickets.

use std::path::Path;

use nva_verify::loadplan;

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
        _ => {
            eprintln!("usage: nva-verify loadplan --fixture <pack-dir>");
            loadplan::EXIT_MISMATCH
        }
    }
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
