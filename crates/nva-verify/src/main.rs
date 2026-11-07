//! `nva-verify` CLI (`docs/ARCHITECTURE.md` §3.8).
//!
//! #42 implements `loadplan`; #44 implements `smoke`. `rewrite` / `shader` /
//! `hle` / `device` are known but not implemented yet: they answer `3` (oracle
//! unavailable) and say why.

use std::path::Path;

use nva_verify::{loadplan, smoke};

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
        [command, rest @ ..] if command.as_str() == "smoke" => run_smoke(rest),
        [command, ..] if is_unimplemented(command) => {
            eprintln!(
                "{command}: not implemented yet, so the oracle is unavailable \
                 (docs/ARCHITECTURE.md §3.8)"
            );
            EXIT_ORACLE_UNAVAILABLE
        }
        _ => {
            eprintln!("usage: nva-verify loadplan --fixture <pack-dir>");
            eprintln!("       nva-verify smoke [--headless] [--frames N] [--json [FILE]]");
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

fn run_smoke(args: &[String]) -> i32 {
    let mut frames = 1u32;
    let mut json_path: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--headless" => {}
            "--frames" => {
                index += 1;
                match args.get(index).and_then(|value| value.parse::<u32>().ok()) {
                    Some(value) => frames = value,
                    None => {
                        eprintln!("smoke: --frames needs a number");
                        return loadplan::EXIT_MISMATCH;
                    }
                }
            }
            "--json" => {
                // `--json` without a value prints to stdout; `--json FILE`
                // writes the same line to FILE (docs/GATES.md §3 uses the file
                // form).
                if let Some(next) = args.get(index + 1) {
                    if !next.starts_with("--") {
                        json_path = Some(next.clone());
                        index += 1;
                    }
                }
            }
            other => {
                eprintln!("smoke: unknown option {other}");
                return loadplan::EXIT_MISMATCH;
            }
        }
        index += 1;
    }

    let outcome = smoke::run(&smoke::Options { frames });
    match json_path {
        Some(path) => {
            if let Err(error) = std::fs::write(&path, format!("{}\n", outcome.json)) {
                eprintln!("smoke: cannot write {path}: {error}");
                return loadplan::EXIT_STALE_FIXTURE;
            }
        }
        None => println!("{}", outcome.json),
    }
    eprintln!("{}", outcome.detail);
    outcome.exit_code
}
