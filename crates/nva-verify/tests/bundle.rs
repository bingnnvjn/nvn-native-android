//! `nva-verify device --from-bundle` through the CLI (#109).
//!
//! The seam is the same external contract #96 tests: exit code, evidence JSON
//! and the private ledger. A bundle is a zip of raw evidence the product APK
//! produced; these tests build synthetic fixtures in a throwaway directory, so
//! no device is involved. The zip is written here with stored entries (no
//! compression, no external crate); the verifier reads it through the system
//! `unzip -p`, the same tool the operator path uses.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// `(R,G,B,A) = (0,255,0,255)` read as a little-endian u32.
const GREEN_PIXEL: u32 = 0xff00ff00;
/// `(R,G,B,A) = (255,0,0,255)` read as a little-endian u32.
const RED_PIXEL: u32 = 0xff0000ff;

const ATTACH_SNAPSHOT: &str = "12:00:00.100 [rt] info nva-stats: {\"presents\":1,\
    \"surface_generation\":1,\"attached\":true,\"generation\":1,\"submits\":1,\
    \"validation_available\":0,\"validation_errors\":0}\n";

const DETACH_SNAPSHOT: &str = "12:00:00.300 [rt] info nva-stats: {\"presents\":1,\
    \"surface_generation\":1,\"attached\":false,\"generation\":1,\"submits\":1,\
    \"validation_available\":0,\"validation_errors\":0}\n";

const RUN_HEADER: &str = "=== run 1 2026-11-28T12:00:00.000+00:00 ===\n";

/// A scratch directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock before the epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("nva-bundle-{}-{tag}-{nanos}", std::process::id()));
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

/// CRC-32 (IEEE 802.3), the polynomial a stored zip entry carries.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Writes a zip whose entries are stored (method 0), in the given order.
///
/// Stored entries need no deflate implementation, which keeps the fixture
/// builder dependency-free; `unzip -p` reads them the same way it reads a
/// deflated bundle from the phone.
fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut body: Vec<u8> = Vec::new();
    let mut directory: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let offset = body.len() as u32;
        let crc = crc32(data);
        let size = data.len() as u32;
        let name_bytes = name.as_bytes();

        push_u32(&mut body, 0x0403_4b50);
        push_u16(&mut body, 20);
        push_u16(&mut body, 0);
        push_u16(&mut body, 0); // stored
        push_u16(&mut body, 0); // time
        push_u16(&mut body, 0); // date
        push_u32(&mut body, crc);
        push_u32(&mut body, size);
        push_u32(&mut body, size);
        push_u16(&mut body, name_bytes.len() as u16);
        push_u16(&mut body, 0);
        body.extend_from_slice(name_bytes);
        body.extend_from_slice(data);

        push_u32(&mut directory, 0x0201_4b50);
        push_u16(&mut directory, 20);
        push_u16(&mut directory, 20);
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0); // stored
        push_u16(&mut directory, 0);
        push_u16(&mut directory, 0);
        push_u32(&mut directory, crc);
        push_u32(&mut directory, size);
        push_u32(&mut directory, size);
        push_u16(&mut directory, name_bytes.len() as u16);
        push_u16(&mut directory, 0); // extra
        push_u16(&mut directory, 0); // comment
        push_u16(&mut directory, 0); // disk
        push_u16(&mut directory, 0); // internal attrs
        push_u32(&mut directory, 0); // external attrs
        push_u32(&mut directory, offset);
        directory.extend_from_slice(name_bytes);
    }

    let directory_offset = body.len() as u32;
    let directory_size = directory.len() as u32;
    let mut zip = body;
    zip.extend_from_slice(&directory);
    push_u32(&mut zip, 0x0605_4b50);
    push_u16(&mut zip, 0);
    push_u16(&mut zip, 0);
    push_u16(&mut zip, entries.len() as u16);
    push_u16(&mut zip, entries.len() as u16);
    push_u32(&mut zip, directory_size);
    push_u32(&mut zip, directory_offset);
    push_u16(&mut zip, 0);
    fs::write(path, zip).expect("write the bundle zip");
}

/// A 12-byte-header raw frame (the app's `u32 width, u32 height, u32 format`
/// shape) with `centre` at the middle.
fn raw_frame(centre: u32) -> Vec<u8> {
    raw_frame_with_format(centre, 1)
}

fn raw_frame_with_format(centre: u32, format: u32) -> Vec<u8> {
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&WIDTH.to_le_bytes());
    bytes.extend_from_slice(&HEIGHT.to_le_bytes());
    bytes.extend_from_slice(&format.to_le_bytes());
    let mut pixels = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    let offset = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 4) as usize;
    pixels[offset..offset + 4].copy_from_slice(&centre.to_le_bytes());
    bytes.extend_from_slice(&pixels);
    bytes
}

/// The SHA-256 of one byte string, through the system tool the verifier uses.
fn sha256_hex(bytes: &[u8]) -> String {
    let dir = TempDir::new("hash");
    let path = dir.path().join("blob");
    fs::write(&path, bytes).expect("write the hash input");
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(&path)
        .output()
        .expect("run sha256sum");
    assert!(output.status.success(), "sha256sum {}", path.display());
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("sha256sum prints a digest")
        .to_string()
}

/// The evidence JSON the app writes. `commit` / `run_id` are fixed so the
/// ledger assertions have a known value.
fn evidence_json(model: &str) -> Vec<u8> {
    format!(
        "{{\"scenario\":\"m0b\",\"source\":\"app\",\
         \"commit\":\"{}\",\"apk_sha256\":\"{}\",\"pack_id\":\"none\",\
         \"build_id\":\"none\",\"device_model\":{},\"driver_version\":\"unknown\",\
         \"vulkan_api\":\"unknown\",\"page_size\":\"4096\",\
         \"run_id\":\"20261128T120000Z-1234\",\"ts_utc\":\"2026-11-28T12:00:00Z\",\
         \"m0a_machine_green\":true,\"surface_generation\":1,\"presents\":1,\
         \"pixel_rgba\":\"0xff00ff00\",\"surface_destroyed_still_submitting\":false,\
         \"crashed\":false}}",
        "0123456789abcdef0123456789abcdef01234567",
        "a".repeat(64),
        quote(model),
    )
    .into_bytes()
}

/// A small JSON string literal encoder for the fixture (the fixture never
/// contains a quote or backslash in the model name).
fn quote(text: &str) -> String {
    format!("\"{text}\"")
}

/// The `sha256.txt` manifest, sorted by file name ascending, as the app writes
/// it.
fn manifest(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut files: Vec<(&str, String)> = entries
        .iter()
        .map(|(name, data)| (*name, sha256_hex(data)))
        .collect();
    files.sort_by(|left, right| left.0.cmp(right.0));
    let mut text = String::new();
    for (name, hash) in files {
        text.push_str(&format!("{hash}  {name}\n"));
    }
    text.into_bytes()
}

/// Builds a bundle from explicit evidence bytes and writes it to `dir`.
fn write_bundle(
    dir: &Path,
    evidence: &[u8],
    raw: &[u8],
    log: &[u8],
    manifest_files: &[(&str, &[u8])],
) -> PathBuf {
    let manifest_bytes = manifest(manifest_files);
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("m0b.json", evidence),
        ("screen.raw", raw),
        ("nva-shell.log", log),
        ("sha256.txt", &manifest_bytes),
    ];
    // Keep a stable order that is not the manifest order on purpose: the
    // verifier must sort by name, not by zip order.
    entries.sort_by(|left, right| right.0.cmp(left.0));
    let zip = dir.join("bundle.zip");
    write_zip(&zip, &entries);
    zip
}

/// The standard green bundle's parts.
fn green_parts(centre: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let evidence = evidence_json("Fake Model");
    let raw = raw_frame(centre);
    let log = format!("{RUN_HEADER}{ATTACH_SNAPSHOT}{DETACH_SNAPSHOT}").into_bytes();
    (evidence, raw, log)
}

fn run_bundle(root: &Path, bundle: &Path, json: Option<&Path>) -> Output {
    let mut args: Vec<String> = vec![
        "device".to_string(),
        "--from-bundle".to_string(),
        bundle.to_str().expect("utf-8 bundle").to_string(),
        "--private-root".to_string(),
        root.to_str().expect("utf-8 root").to_string(),
    ];
    if let Some(path) = json {
        args.push("--json".to_string());
        args.push(path.to_str().expect("utf-8 json").to_string());
    }
    Command::new(env!("CARGO_BIN_EXE_nva-verify"))
        .args(&args)
        .output()
        .expect("run nva-verify")
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

/// The single `result` value a fresh private root's ledger holds.
fn ledger_result(root: &Path) -> String {
    use nva_verify::json::{parse, Value};

    let ledger = fs::read_to_string(root.join("gate-ledger.jsonl")).expect("read the ledger");
    let lines: Vec<&str> = ledger.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "one run appends exactly one entry");
    let entry = parse(lines[0]).expect("a ledger line is one JSON object");
    entry
        .get("result")
        .and_then(Value::as_str)
        .expect("the ledger has a string result")
        .to_string()
}

#[test]
fn bundle_green_exits_0() {
    let root = TempDir::new("green-root");
    let work = TempDir::new("green-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 0, "a green bundle must pass; stderr: {stderr}");
    assert!(
        stderr.contains("result=pass"),
        "the verdict must say pass; stderr: {stderr}"
    );
    assert!(
        !stderr.contains("Fake Model"),
        "stderr must not carry the device model; stderr: {stderr}"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\"result\":\"pass\""),
        "--json omitted writes the evidence JSON to stdout; stdout: {stdout}"
    );
}

#[test]
fn bundle_pixel_mismatch_exits_2() {
    let root = TempDir::new("pixel-root");
    let work = TempDir::new("pixel-work");
    let (evidence, _green, log) = green_parts(GREEN_PIXEL);
    let raw = raw_frame(RED_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 2, "a red centre pixel must exit 2; stderr: {stderr}");
    assert!(
        stderr.contains("pixel") || stderr.contains("screen.raw"),
        "the reason must name the pixel mismatch; stderr: {stderr}"
    );
    assert_eq!(
        ledger_result(root.path()),
        "mismatch",
        "exit 2 maps to the mismatch result"
    );
}

#[test]
fn bundle_bad_json_exits_2() {
    let root = TempDir::new("bad-json-root");
    let work = TempDir::new("bad-json-work");
    let (_evidence, raw, log) = green_parts(GREEN_PIXEL);
    let broken = b"{\"scenario\":\"m0b\",\"commit\":".to_vec();
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &broken),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &broken, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 2, "malformed m0b.json must exit 2; stderr: {stderr}");
    assert!(
        stderr.contains("m0b.json"),
        "the reason must name m0b.json; stderr: {stderr}"
    );
}

#[test]
fn bundle_missing_field_exits_2() {
    let root = TempDir::new("missing-field-root");
    let work = TempDir::new("missing-field-work");
    let (_evidence, raw, log) = green_parts(GREEN_PIXEL);
    // Valid JSON with every field but `run_id`.
    let incomplete = format!(
        "{{\"commit\":\"{}\",\"apk_sha256\":\"{}\",\"ts_utc\":\"2026-11-28T12:00:00Z\",\
         \"m0a_machine_green\":true,\"crashed\":false}}",
        "0123456789abcdef0123456789abcdef01234567",
        "a".repeat(64),
    )
    .into_bytes();
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &incomplete),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &incomplete, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 2, "a missing field must exit 2; stderr: {stderr}");
    assert!(
        stderr.contains("run_id"),
        "the reason must name the missing field; stderr: {stderr}"
    );
}

#[test]
fn bundle_stale_exits_4() {
    let root = TempDir::new("stale-root");
    let work = TempDir::new("stale-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    // The manifest is computed from the original evidence, but the zip carries
    // a tampered copy, so the digest no longer matches.
    let tampered = b"{\"scenario\":\"m0b\",\"tampered\":true}".to_vec();
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &tampered, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 4,
        "a SHA-256 mismatch is stale evidence; stderr: {stderr}"
    );
    assert!(
        stderr.contains("sha256") || stderr.contains("stale"),
        "the reason must name the stale digest; stderr: {stderr}"
    );
    assert_eq!(
        ledger_result(root.path()),
        "stale",
        "exit 4 maps to the stale result"
    );
}

#[test]
fn bundle_missing_entry_exits_4() {
    let root = TempDir::new("missing-entry-root");
    let work = TempDir::new("missing-entry-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    // The manifest lists all three, but the zip omits m0b.json. Some `unzip`
    // builds return success with empty output here; the digest is what fails.
    let manifest_bytes = manifest(&[
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ]);
    let zip = work.path().join("bundle.zip");
    write_zip(
        &zip,
        &[
            ("screen.raw", &raw),
            ("nva-shell.log", &log),
            ("sha256.txt", &manifest_bytes),
        ],
    );

    let output = run_bundle(root.path(), &zip, None);
    assert_eq!(
        exit_code(&output),
        4,
        "a manifest entry missing from the zip is stale; stderr: {}",
        stderr(&output)
    );
}

#[test]
fn bundle_unreadable_zip_exits_4() {
    let root = TempDir::new("unreadable-root");
    let work = TempDir::new("unreadable-work");
    let missing = work.path().join("not-a-zip.zip");
    let output = run_bundle(root.path(), &missing, None);
    assert_eq!(
        exit_code(&output),
        4,
        "an unreadable zip is stale evidence; stderr: {}",
        stderr(&output)
    );
}

/// The single parsed ledger entry in a fresh private root.
fn ledger_entry(root: &Path) -> nva_verify::json::Value {
    use nva_verify::json::parse;

    let ledger = fs::read_to_string(root.join("gate-ledger.jsonl")).expect("read the ledger");
    let lines: Vec<&str> = ledger.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 1, "one run appends exactly one entry");
    parse(lines[0]).expect("a ledger line is one JSON object")
}

/// The evidence digests ordered by file name, from the fixture bytes.
fn expected_hashes(files: &[(&str, &[u8])]) -> Vec<String> {
    let mut pairs: Vec<(&str, String)> = files
        .iter()
        .map(|(name, data)| (*name, sha256_hex(data)))
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(right.0));
    pairs.into_iter().map(|(_, hash)| hash).collect()
}

#[test]
fn bundle_ledger_is_eight_key_json() {
    use nva_verify::json::Value;

    let root = TempDir::new("ledger-root");
    let work = TempDir::new("ledger-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);
    let json_path = root.path().join("bundle-evidence.json");

    let output = run_bundle(root.path(), &bundle, Some(&json_path));
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(code, 0, "a green bundle must pass; stderr: {stderr}");
    assert!(
        !stderr.contains("Fake Model"),
        "stderr must not carry the device model; stderr: {stderr}"
    );

    let entry = ledger_entry(root.path());
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
    assert_eq!(entry.get("gate").and_then(Value::as_str), Some("m0b"));
    assert_eq!(entry.get("pack_id").and_then(Value::as_str), Some("none"));
    assert_eq!(
        entry.get("commit").and_then(Value::as_str),
        Some("0123456789abcdef0123456789abcdef01234567"),
        "the ledger names the build commit the bundle carried"
    );
    assert_eq!(
        entry.get("run_id").and_then(Value::as_str),
        Some("20261128T120000Z-1234"),
        "the ledger uses the run the device produced"
    );
    assert_eq!(
        entry.get("result").and_then(Value::as_str),
        Some("pass"),
        "0 maps to pass"
    );
    let hashes: Vec<String> = entry
        .get("evidence_sha256")
        .and_then(Value::as_array)
        .expect("evidence_sha256 is an array")
        .iter()
        .map(|value| {
            let hash = value.as_str().expect("each evidence hash is a string");
            assert_eq!(hash.len(), 64, "evidence hashes are SHA-256 hex");
            hash.to_string()
        })
        .collect();
    assert_eq!(hashes.len(), 3, "three evidence files are hashed");
    assert_eq!(
        hashes,
        expected_hashes(&files),
        "evidence_sha256[] holds the digests ordered by file name"
    );

    // The verifier's own evidence JSON is written where `--json` asked, inside
    // the private root.
    let written = fs::read_to_string(&json_path).expect("the --json evidence file was written");
    let written = nva_verify::json::parse(&written).expect("the evidence file is JSON");
    assert_eq!(written.get("result").and_then(Value::as_str), Some("pass"));
    assert_eq!(
        written.get("source").and_then(Value::as_str),
        Some("bundle"),
        "the evidence names its carrier"
    );
    assert_eq!(
        written.get("device_model").and_then(Value::as_str),
        Some("Fake Model"),
        "the model stays in the private evidence JSON"
    );
}

#[test]
fn bundle_contents_match_manifest() {
    use nva_verify::json::Value;

    let root = TempDir::new("contents-root");
    let work = TempDir::new("contents-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);
    let json_path = root.path().join("contents-evidence.json");

    let output = run_bundle(root.path(), &bundle, Some(&json_path));
    assert_eq!(exit_code(&output), 0, "stderr: {}", stderr(&output));

    // The manifest the app wrote lists exactly the three evidence files, each
    // digest matching the bytes the verifier read back out of the zip.
    let entry = ledger_entry(root.path());
    let hashes: Vec<String> = entry
        .get("evidence_sha256")
        .and_then(Value::as_array)
        .expect("evidence_sha256 is an array")
        .iter()
        .map(|value| value.as_str().expect("a hash string").to_string())
        .collect();
    let expected = expected_hashes(&files);
    assert_eq!(
        hashes, expected,
        "the ledger digests match the manifest's three files"
    );
    let manifest_text = String::from_utf8(manifest(&files)).expect("utf-8 manifest");
    assert_eq!(
        manifest_text.lines().count(),
        3,
        "sha256.txt lists JSON, raw pixels and the log"
    );
    assert!(
        !manifest_text.contains("sha256.txt"),
        "the manifest does not hash itself"
    );
}

#[test]
fn bundle_unsupported_pixel_format_exits_2() {
    let root = TempDir::new("format-root");
    let work = TempDir::new("format-work");
    let (evidence, _green, log) = green_parts(GREEN_PIXEL);
    // RGBX_8888 (2), not RGBA_8888 (1): the 4-byte packing is not the one the
    // gate decodes, so this is evidence disagreement, not a stale bundle.
    let raw = raw_frame_with_format(GREEN_PIXEL, 2);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 2,
        "an unsupported format must exit 2; stderr: {stderr}"
    );
    assert!(
        stderr.contains("format 2"),
        "the reason must name the format; stderr: {stderr}"
    );
}

#[test]
fn bundle_detached_submit_exits_2() {
    let root = TempDir::new("submit-root");
    let work = TempDir::new("submit-work");
    let (evidence, raw, _log) = green_parts(GREEN_PIXEL);
    let signal = "12:00:00.350 [rt] info nva-stats: \
        {\"event\":\"detached_submit\",\"count\":1,\"generation\":1}\n";
    let log = format!("{RUN_HEADER}{ATTACH_SNAPSHOT}{DETACH_SNAPSHOT}{signal}").into_bytes();
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 2,
        "a post-HOME submit fails the HOME counter-proof; stderr: {stderr}"
    );
    assert!(
        stderr.contains("detached_submit=true"),
        "the reason must name the signal; stderr: {stderr}"
    );
}

#[test]
fn bundle_uses_the_last_run() {
    let root = TempDir::new("runs-root");
    let work = TempDir::new("runs-work");
    let (evidence, raw, _log) = green_parts(GREEN_PIXEL);
    // The normal shell wrote a run with higher counters first; acceptance
    // started a second run whose counters restart at zero. Parsing both would
    // read as OutOfOrder, so the verifier slices at the last header.
    let old_attach = "12:00:00.010 [rt] info nva-stats: {\"presents\":5,\
        \"surface_generation\":1,\"attached\":true,\"generation\":1,\"submits\":5,\
        \"validation_available\":0,\"validation_errors\":0}\n";
    let log = format!(
        "=== run 1 2026-11-28T11:00:00.000+00:00 ===\n{old_attach}\
         {RUN_HEADER}{ATTACH_SNAPSHOT}{DETACH_SNAPSHOT}"
    )
    .into_bytes();
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);

    let output = run_bundle(root.path(), &bundle, None);
    assert_eq!(
        exit_code(&output),
        0,
        "the last run is the acceptance run; stderr: {}",
        stderr(&output)
    );
}

#[test]
fn bundle_json_outside_private_root_exits_3() {
    let root = TempDir::new("outside-root");
    let outside = TempDir::new("outside-target");
    let work = TempDir::new("outside-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);
    let json_path = outside.path().join("bundle-evidence.json");

    let output = run_bundle(root.path(), &bundle, Some(&json_path));
    let code = exit_code(&output);
    let stderr = stderr(&output);
    assert_eq!(
        code, 3,
        "--json outside the root is unprepared; stderr: {stderr}"
    );
    assert!(
        stderr.contains("outside the private root"),
        "the reason must name the boundary; stderr: {stderr}"
    );
}

#[test]
fn bundle_missing_private_root_exits_3() {
    let root = TempDir::new("nope-root");
    let work = TempDir::new("nope-work");
    let (evidence, raw, log) = green_parts(GREEN_PIXEL);
    let files: Vec<(&str, &[u8])> = vec![
        ("m0b.json", &evidence),
        ("screen.raw", &raw),
        ("nva-shell.log", &log),
    ];
    let bundle = write_bundle(work.path(), &evidence, &raw, &log, &files);
    let missing_root = root.path().join("not-prepared");

    let output = run_bundle(&missing_root, &bundle, None);
    assert_eq!(
        exit_code(&output),
        3,
        "a missing private root is unprepared; stderr: {}",
        stderr(&output)
    );
}
