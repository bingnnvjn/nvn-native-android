//! `nva-verify device --scenario m0b` through the CLI (#96).
//!
//! No real device is involved: a fake `adb` script stands in through `NVA_ADB`,
//! and a throwaway git repository gives the identity probes a clean tree. The
//! assertions stay on the external contract — exit code, stderr, evidence JSON
//! and the private ledger.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Fake `adb`. The state directory holds `attach.log`, `detach.log`,
/// `screen.raw`, and the `home` marker the `input keyevent` arm creates.
const FAKE_ADB: &str = r#"#!/bin/sh
set -u
state=${FAKE_STATE_DIR:?}
cmd=${1:-}
shift || true
case "$cmd" in
  logcat)
    sub=${1:-}
    case "$sub" in
      -c) rm -f "$state/home"; exit 0 ;;
      -d)
        cat "$state/attach.log"
        if [ -f "$state/home" ]; then cat "$state/detach.log"; fi
        exit 0 ;;
      *) exit 0 ;;
    esac ;;
  shell)
    sub=${1:-}
    shift || true
    case "$sub" in
      getprop)
        case "${1:-}" in
          ro.product.model) printf '%s\n' "${FAKE_MODEL:-Fake Model}" ;;
          ro.build.version.sdk) printf '34\n' ;;
          *) printf '\n' ;;
        esac
        exit 0 ;;
      pm)
        if [ "${FAKE_STALE_APK:-0}" = 1 ]; then
          echo "error: package com.bingnnvjn.nva not found" >&2
          exit 1
        fi
        printf 'package:/data/app/com.bingnnvjn.nva/base.apk\n'
        exit 0 ;;
      sha256sum)
        printf '%064d  %s\n' 0 "${1:-/data/app/base.apk}"
        exit 0 ;;
      dumpsys)
        printf '%s\n' "${FAKE_DUMPSYS:-Vulkan API level: 1.1.128}"
        exit 0 ;;
      getconf)
        printf '4096\n'
        exit 0 ;;
      am) exit 0 ;;
      input)
        touch "$state/home"
        exit 0 ;;
      pidof)
        printf '%s\n' "${FAKE_PID:-4242}"
        exit 0 ;;
      *) exit 0 ;;
    esac ;;
  exec-out)
    cat "$state/screen.raw"
    exit 0 ;;
esac
exit 0
"#;

const ATTACH_SNAPSHOT: &str = "13:40:12.418 [rt] info nva-stats: {\"presents\":1,\
    \"surface_generation\":1,\"attached\":true,\"generation\":1,\"submits\":1,\
    \"validation_available\":0,\"validation_errors\":0}\n";

const DETACH_SNAPSHOT: &str = "13:40:12.918 [rt] info nva-stats: {\"presents\":1,\
    \"surface_generation\":1,\"attached\":false,\"generation\":1,\"submits\":1,\
    \"validation_available\":0,\"validation_errors\":0}\n";

const DETACH_AFTER_HOME_GROWTH: &str = "13:40:12.918 [rt] info nva-stats: {\"presents\":2,\
    \"surface_generation\":1,\"attached\":false,\"generation\":1,\"submits\":2,\
    \"validation_available\":0,\"validation_errors\":0}\n";

const DETACHED_SUBMIT_SIGNAL: &str = "13:40:12.919 [rt] info nva-stats: \
    {\"event\":\"detached_submit\",\"count\":2,\"generation\":1}\n";

/// `(R,G,B,A) = (0,255,0,255)` read as a little-endian u32.
const GREEN_PIXEL: u32 = 0xff00ff00;
/// `(R,G,B,A) = (255,0,0,255)` read as a little-endian u32.
const RED_PIXEL: u32 = 0xff0000ff;

/// A scratch directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before the epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nva-device-{}-{tag}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("create the temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

/// A clean one-commit git repository: `git status --porcelain` is empty, which
/// the verifier's identity probe requires.
fn temp_git_repo(tag: &str) -> TempDir {
    let dir = TempDir::new(tag);
    git(dir.path(), &["init", "-q"]);
    fs::write(dir.path().join("README.md"), "device fixture\n").expect("write the repo file");
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.name=device-test",
            "-c",
            "user.email=device@example.invalid",
            "commit",
            "-qm",
            "init",
        ],
    );
    dir
}

fn write_script(path: &Path) {
    fs::write(path, FAKE_ADB).expect("write the fake adb");
    let mut permissions = fs::metadata(path).expect("stat the fake adb").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make the fake adb executable");
}

/// Writes a 16-byte-header raw screencap (Android 10+ shape) with `centre` at
/// the middle of a 4x4 RGBA_8888 image.
fn write_raw_screencap(path: &Path, centre: u32) {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&WIDTH.to_le_bytes());
    bytes.extend_from_slice(&HEIGHT.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes()); // RGBA_8888
    bytes.extend_from_slice(&0u32.to_le_bytes()); // colorspace
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let offset = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    pixels[offset..offset + 4].copy_from_slice(&centre.to_le_bytes());
    bytes.extend_from_slice(&pixels);
    fs::write(path, bytes).expect("write the raw screencap");
}

fn write_logs(state: &Path, attach: &str, detach: &str) {
    fs::write(state.join("attach.log"), attach).expect("write the attach log");
    fs::write(state.join("detach.log"), detach).expect("write the detach log");
}

/// The two environment variables every fake-adb test needs.
fn fake_env<'a>(adb: &'a Path, state: &'a Path) -> Vec<(&'a str, &'a str)> {
    vec![
        ("NVA_ADB", adb.to_str().expect("utf-8 adb")),
        ("FAKE_STATE_DIR", state.to_str().expect("utf-8 state")),
    ]
}

fn exit_code(output: &Output) -> i32 {
    output
        .status
        .code()
        .expect("nva-verify exited through a signal")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run_device(repo: &Path, envs: &[(&str, &str)], args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nva-verify"));
    command.args(args).current_dir(repo).env_remove("NVA_ADB");
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run nva-verify")
}

fn device_args<'a>(private_root: &'a str, json: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec![
        "device",
        "--scenario",
        "m0b",
        "--private-root",
        private_root,
        "--m0a-machine-green",
    ];
    if let Some(path) = json {
        args.push("--json");
        args.push(path);
    }
    args
}

#[test]
fn device_ledger_entry_is_eight_key_json() {
    use nva_verify::json::{parse, Value};

    let repo = temp_git_repo("ledger");
    let root = TempDir::new("ledger-root");
    let state = TempDir::new("ledger-state");
    let adb = state.path().join("adb");
    write_script(&adb);
    write_raw_screencap(&state.path().join("screen.raw"), GREEN_PIXEL);
    write_logs(state.path(), ATTACH_SNAPSHOT, DETACH_SNAPSHOT);

    let evidence_dir = root.path().join("evidence").join("96");
    fs::create_dir_all(&evidence_dir).expect("create the evidence dir");
    let json_path = evidence_dir.join("m0b.json");

    let output = run_device(
        repo.path(),
        &fake_env(&adb, state.path()),
        &device_args(
            root.path().to_str().expect("utf-8 root"),
            Some(json_path.to_str().expect("utf-8 json")),
        ),
    );
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 0, "a green fake run must pass; stderr: {stderr}");

    let evidence = fs::read_to_string(&json_path).expect("read the evidence JSON");
    let evidence = parse(&evidence).expect("the evidence file is JSON");
    assert_eq!(
        evidence.get("result").and_then(Value::as_str),
        Some("pass"),
        "the evidence result must pass"
    );
    for key in [
        "validation_available",
        "validation_errors",
        "m0a_machine_green",
        "surface_generation",
        "presents",
        "pixel_rgba",
    ] {
        assert!(evidence.get(key).is_some(), "evidence is missing {key}");
    }

    let ledger =
        fs::read_to_string(root.path().join("gate-ledger.jsonl")).expect("read the ledger");
    let lines: Vec<&str> = ledger.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "the first run appends exactly one entry");
    let entry = parse(lines[0]).expect("a ledger line is one JSON object");
    let entries = match &entry {
        Value::Object(entries) => entries,
        other => panic!("the ledger entry must be an object: {other:?}"),
    };
    let keys: Vec<&str> = entries.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "gate",
            "commit",
            "apk_sha256",
            "pack_id",
            "run_id",
            "ts_utc",
            "result",
            "evidence_sha256",
        ],
        "the ledger schema is eight keys in the documented order"
    );
    assert_eq!(
        entry.get("gate").and_then(Value::as_str),
        Some("m0b"),
        "the ledger names the gate"
    );
    assert_eq!(
        entry.get("result").and_then(Value::as_str),
        Some("pass"),
        "0 maps to pass"
    );
    assert_eq!(
        entry.get("pack_id").and_then(Value::as_str),
        Some("none"),
        "m0b has no pack precondition"
    );
    let commit = entry
        .get("commit")
        .and_then(Value::as_str)
        .expect("commit is a string");
    assert_eq!(commit.len(), 40, "commit is a full git object id");

    let hashes: Vec<String> = entry
        .get("evidence_sha256")
        .and_then(Value::as_array)
        .expect("evidence_sha256 is an array")
        .iter()
        .map(|value| {
            let hash = value.as_str().expect("each evidence hash is a string");
            assert_eq!(hash.len(), 64, "evidence hashes are SHA-256 hex");
            assert!(
                hash.chars().all(|ch| ch.is_ascii_hexdigit()),
                "evidence hashes are hex: {hash}"
            );
            hash.to_string()
        })
        .collect();
    assert_eq!(
        hashes.len(),
        3,
        "JSON, raw screenshot and logcat are the evidence files"
    );
    let mut sorted = hashes.clone();
    sorted.sort();
    assert_eq!(hashes, sorted, "evidence hashes sort by file name");
}

#[test]
fn device_pixel_mismatch_exits_2() {
    let repo = temp_git_repo("pixel");
    let root = TempDir::new("pixel-root");
    let state = TempDir::new("pixel-state");
    let adb = state.path().join("adb");
    write_script(&adb);
    write_raw_screencap(&state.path().join("screen.raw"), RED_PIXEL);
    write_logs(state.path(), ATTACH_SNAPSHOT, DETACH_SNAPSHOT);
    let output = run_device(
        repo.path(),
        &fake_env(&adb, state.path()),
        &device_args(root.path().to_str().expect("utf-8 root"), None),
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 2, "a red centre pixel must exit 2; stderr: {stderr}");
    assert!(
        stderr.contains("pixel"),
        "the reason must name the pixel mismatch; stderr: {stderr}"
    );
}

#[test]
fn device_rendering_between_attach_and_home_is_normal() {
    use nva_verify::json::{parse, Value};

    let repo = temp_git_repo("home-growth");
    let root = TempDir::new("home-growth-root");
    let state = TempDir::new("home-growth-state");
    let adb = state.path().join("adb");
    write_script(&adb);
    write_raw_screencap(&state.path().join("screen.raw"), GREEN_PIXEL);
    write_logs(state.path(), ATTACH_SNAPSHOT, DETACH_AFTER_HOME_GROWTH);

    let evidence_dir = root.path().join("evidence").join("96");
    fs::create_dir_all(&evidence_dir).expect("create the evidence dir");
    let json_path = evidence_dir.join("m0b.json");
    let output = run_device(
        repo.path(),
        &fake_env(&adb, state.path()),
        &device_args(
            root.path().to_str().expect("utf-8 root"),
            Some(json_path.to_str().expect("utf-8 json")),
        ),
    );
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 0,
        "frames presented between attach and HOME are normal; stderr: {stderr}"
    );

    let evidence = fs::read_to_string(&json_path).expect("read the evidence JSON");
    let evidence = parse(&evidence).expect("the evidence file is JSON");
    assert_eq!(
        evidence.get("presents_before").and_then(Value::as_i64),
        Some(1),
        "the attach snapshot records the baseline"
    );
    assert_eq!(
        evidence.get("presents_after").and_then(Value::as_i64),
        Some(2),
        "the detach snapshot includes frames presented before HOME"
    );
    assert_eq!(
        evidence.get("surface_destroyed_still_submitting"),
        Some(&Value::Bool(false)),
        "no post-stop present means HOME froze the counter"
    );
}

#[test]
fn device_detached_submit_exits_2() {
    let repo = temp_git_repo("detached-submit");
    let root = TempDir::new("detached-submit-root");
    let state = TempDir::new("detached-submit-state");
    let adb = state.path().join("adb");
    write_script(&adb);
    write_raw_screencap(&state.path().join("screen.raw"), GREEN_PIXEL);
    write_logs(
        state.path(),
        ATTACH_SNAPSHOT,
        &format!("{DETACH_SNAPSHOT}{DETACHED_SUBMIT_SIGNAL}"),
    );
    let output = run_device(
        repo.path(),
        &fake_env(&adb, state.path()),
        &device_args(root.path().to_str().expect("utf-8 root"), None),
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 2,
        "a detached_submit signal fails the fourth criterion; stderr: {stderr}"
    );
    assert!(
        stderr.contains("detached_submit=true"),
        "the reason must name the signal; stderr: {stderr}"
    );
}

#[test]
fn device_unknown_scenario_exits_2() {
    let repo = temp_git_repo("scenario");
    let root = TempDir::new("scenario-root");
    let output = run_device(
        repo.path(),
        &[],
        &[
            "device",
            "--scenario",
            "m9",
            "--private-root",
            root.path().to_str().expect("utf-8 root"),
            "--m0a-machine-green",
        ],
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 2,
        "an unknown scenario is a usage error; stderr: {stderr}"
    );
    assert!(
        stderr.contains("scenario"),
        "the reason must name the scenario; stderr: {stderr}"
    );
}

#[test]
fn device_missing_m0a_declaration_exits_3() {
    let repo = temp_git_repo("m0a");
    let root = TempDir::new("m0a-root");
    let output = run_device(
        repo.path(),
        &[],
        &[
            "device",
            "--scenario",
            "m0b",
            "--private-root",
            root.path().to_str().expect("utf-8 root"),
        ],
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 3,
        "the M0a declaration is an operator precondition; stderr: {stderr}"
    );
    assert!(
        stderr.contains("--m0a-machine-green"),
        "the reason must name the missing declaration; stderr: {stderr}"
    );
}

#[test]
fn device_stale_apk_exits_4() {
    let repo = temp_git_repo("stale-apk");
    let root = TempDir::new("stale-apk-root");
    let state = TempDir::new("stale-apk-state");
    let adb = state.path().join("adb");
    write_script(&adb);
    let output = run_device(
        repo.path(),
        &[
            ("NVA_ADB", adb.to_str().expect("utf-8 adb")),
            (
                "FAKE_STATE_DIR",
                state.path().to_str().expect("utf-8 state"),
            ),
            ("FAKE_STALE_APK", "1"),
        ],
        &device_args(root.path().to_str().expect("utf-8 root"), None),
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 4,
        "an uninstalled APK is stale evidence; stderr: {stderr}"
    );
    assert!(
        stderr.contains("APK") || stderr.contains("package"),
        "the reason must name the stale APK; stderr: {stderr}"
    );
}

#[test]
fn device_no_adb_exits_3() {
    let repo = temp_git_repo("no-adb");
    let root = TempDir::new("no-adb-root");
    let output = run_device(
        repo.path(),
        &[("NVA_ADB", "/nonexistent/nva-fake-adb")],
        &device_args(root.path().to_str().expect("utf-8 root"), None),
    );

    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 3, "missing adb must exit 3; stderr: {stderr}");
    assert!(
        stderr.contains("adb"),
        "the reason must name adb; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("not implemented"),
        "device is implemented now, not a pending oracle; stderr: {stderr}"
    );
}
