//! Exit-code semantics of the `nva-verify` CLI (#43).
//!
//! Only externally observable behavior is asserted: the process exit code and
//! the reason printed on stderr. `0` pass / `2` mismatch / `3` oracle
//! unavailable / `4` fixture or hash is stale (`docs/ARCHITECTURE.md` §3.8).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nva-verify"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("run nva-verify")
}

fn code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("nva-verify exited through a signal")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Run the CLI and require `want` as the exit code. The failure message carries
/// the arguments and stderr, so a wrong exit code says why.
fn expect_exit(args: &[&str], want: i32) -> Output {
    let output = run(args);
    assert_eq!(
        code(&output),
        want,
        "{args:?} must exit {want}; stderr: {}",
        stderr(&output)
    );
    output
}

/// A scratch directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before the epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nva-verify-{}-{tag}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create the temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Copy the committed synthetic pack into `into`, so a test can break the copy
/// without touching `ci/fixtures/pack-min`.
fn copy_min_pack(into: &Path) {
    std::fs::create_dir_all(into).expect("create the pack dir");
    for name in ["manifest.json", "load-plan.json"] {
        std::fs::copy(
            repo_root().join("ci/fixtures/pack-min").join(name),
            into.join(name),
        )
        .expect("copy a fixture file");
    }
}

#[test]
fn synthetic_pack_exits_0() {
    expect_exit(&["loadplan", "--fixture", "ci/fixtures/pack-min"], 0);
}

#[test]
fn missing_fixture_exits_4() {
    let dir = TempDir::new("missing");
    let absent = dir.path().join("no-such-pack");
    expect_exit(&["loadplan", "--fixture", absent.to_str().unwrap()], 4);
}

#[test]
fn tampered_load_plan_exits_2() {
    let dir = TempDir::new("tampered");
    let pack = dir.path().join("pack");
    copy_min_pack(&pack);
    let plan = pack.join("load-plan.json");
    let text = std::fs::read_to_string(&plan).expect("read the load plan");
    let tampered = text.replace("\"version\":1", "\"version\":2");
    assert_ne!(
        tampered, text,
        "the fixture is no longer version 1; update the tampering"
    );
    std::fs::write(&plan, tampered).expect("write the tampered load plan");

    expect_exit(&["loadplan", "--fixture", pack.to_str().unwrap()], 2);
}

#[test]
fn malformed_json_exits_2() {
    let dir = TempDir::new("malformed");
    let pack = dir.path().join("pack");
    copy_min_pack(&pack);
    std::fs::write(pack.join("load-plan.json"), "{not json").expect("break the load plan");

    expect_exit(&["loadplan", "--fixture", pack.to_str().unwrap()], 2);
}

#[test]
fn unreadable_manifest_exits_4() {
    let dir = TempDir::new("unreadable");
    let pack = dir.path().join("pack");
    copy_min_pack(&pack);
    // A directory named `manifest.json` exists but cannot be read, which is a
    // stale/unusable fixture (4), not a schema mismatch (2).
    std::fs::remove_file(pack.join("manifest.json")).expect("remove the manifest");
    std::fs::create_dir(pack.join("manifest.json")).expect("create the directory");

    expect_exit(&["loadplan", "--fixture", pack.to_str().unwrap()], 4);
}

#[test]
fn unimplemented_subcommands_exit_3_and_say_why() {
    let cases: &[&[&str]] = &[
        &["rewrite", "--fixture", "ci/fixtures/rewrite-min"],
        &["shader"],
        &["hle"],
        &["device"],
    ];
    for args in cases {
        let command = args[0];
        let reason = stderr(&expect_exit(args, 3));
        assert!(
            reason.contains(command),
            "{command}: the reason must name the command, stderr: {reason}"
        );
        // "known but not implemented", not the usage error: naming the command
        // alone is not a reason (the usage line also names it).
        assert!(
            reason.contains("not implemented"),
            "{command}: the reason must say it is not implemented, stderr: {reason}"
        );
    }
}

#[test]
fn unknown_subcommand_stays_a_usage_error() {
    let reason = stderr(&expect_exit(&["bogus"], 2));
    assert!(
        reason.contains("usage:"),
        "an unknown command must print the usage, stderr: {reason}"
    );
}
