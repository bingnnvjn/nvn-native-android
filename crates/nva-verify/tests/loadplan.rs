//! `nva-verify loadplan` against the synthetic fixture (#42).

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

#[test]
fn reads_the_synthetic_pack() {
    let output = run(&["loadplan", "--fixture", "ci/fixtures/pack-min"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 stdout");
    assert_eq!(
        stdout.trim_end(),
        r#"{"entry":4096,"fixture":"ci/fixtures/pack-min","schema":1,"segments":1}"#
    );
}
