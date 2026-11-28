//! `nva-verify device` — the M0b device gate (#96).
//!
//! The contract is `docs/GATES.md` §4: run one `m0b` scenario on a phone and
//! judge only the four device criteria plus the HOME counter-proof. The PC
//! `smoke` verdict is deliberately not reused — its lavapipe and validation
//! layer expectations cannot hold on a real device.
//!
//! Every device call goes through [`Adb`], whose program is overridable with
//! `NVA_ADB` so tests can drive the CLI with a fake script. Exit codes are the
//! workspace contract: `0` pass, `2` mismatch, `3` device/tool unavailable,
//! `4` APK or evidence stale.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::json;
use crate::loadplan::{EXIT_MISMATCH, EXIT_STALE_FIXTURE};
use crate::smoke::{EXIT_OK, EXIT_ORACLE};
use crate::statslog::{self, Snapshot};

/// The subcommand name. `main.rs` matches on this instead of repeating the
/// literal, so the retired `is_unimplemented` pin cannot creep back.
pub const COMMAND: &str = "device";

const PACKAGE: &str = "com.bingnnvjn.nva";
const ACTIVITY: &str = "com.bingnnvjn.nva/.MainActivity";
const PIXEL_GREEN: u32 = 0xff00ff00;
/// `PixelFormat.RGBA_8888` / `ANDROID_BITMAP_FORMAT_RGBA_8888` in the raw
/// screencap header. Other formats are not decoded.
const ANDROID_FORMAT_RGBA_8888: u32 = 1;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(20);
const DETACH_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to keep reading after the detach snapshot before accepting that
/// no `detached_submit` signal will follow it (the runtime writes the snapshot
/// first and the signal second).
const SIGNAL_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A failed pre-step: the exit code the CLI returns and the reason on stderr.
struct Failure {
    exit: i32,
    detail: String,
}

fn fail(exit: i32, detail: impl Into<String>) -> Failure {
    Failure {
        exit,
        detail: detail.into(),
    }
}

/// Parsed command line for `nva-verify device`.
struct Options {
    json: Option<PathBuf>,
    private_root: PathBuf,
}

/// Entry point called by `main.rs` with the arguments after `device`.
pub fn run_cli(args: &[String]) -> i32 {
    let mut options = match parse_args(args) {
        Ok(options) => options,
        Err(failure) => {
            eprintln!("device: {}", failure.detail);
            return failure.exit;
        }
    };
    options.private_root = match ensure_private_root(&options.private_root) {
        Ok(root) => root,
        Err(failure) => {
            eprintln!("device: {}", failure.detail);
            return failure.exit;
        }
    };
    if let Some(json) = &options.json {
        if let Err(failure) = ensure_json_inside_root(json, &options.private_root) {
            eprintln!("device: {}", failure.detail);
            return failure.exit;
        }
    }

    let mut report = Report::new(options);
    let adb = Adb::new();
    if let Err(failure) = collect_identity(&adb, &mut report.identity) {
        report.detail = failure.detail;
        return report.finish(failure.exit);
    }
    report.capture = match capture_m0b(&adb) {
        Ok(capture) => Some(capture),
        Err(failure) => {
            report.detail = failure.detail;
            return report.finish(failure.exit);
        }
    };
    let code = verdict_m0b(&report.facts());
    report.detail = report.success_detail(code);
    report.finish(code)
}

/// Parses `--scenario m0b --json <out> --private-root <path> --m0a-machine-green`.
///
/// A missing or unknown scenario is a usage error (`2`). A missing private
/// root or the absent operator declaration is an unprepared environment (`3`):
/// the gate cannot even start.
fn parse_args(args: &[String]) -> Result<Options, Failure> {
    let mut scenario: Option<String> = None;
    let mut json: Option<PathBuf> = None;
    let mut private_root: Option<PathBuf> = None;
    let mut machine_green = false;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--scenario" => {
                index += 1;
                scenario = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| fail(EXIT_MISMATCH, "--scenario needs a value"))?,
                );
            }
            "--json" => {
                // Like `smoke`, `--json` alone means stdout; `--json FILE`
                // also writes the evidence file.
                if let Some(next) = args.get(index + 1) {
                    if !next.starts_with("--") {
                        json = Some(PathBuf::from(next));
                        index += 1;
                    }
                }
            }
            "--private-root" => {
                index += 1;
                private_root =
                    Some(PathBuf::from(args.get(index).cloned().ok_or_else(
                        || fail(EXIT_ORACLE, "--private-root needs a path"),
                    )?));
            }
            "--m0a-machine-green" => machine_green = true,
            other => {
                return Err(fail(EXIT_MISMATCH, format!("unknown option {other}")));
            }
        }
        index += 1;
    }

    let scenario = scenario.ok_or_else(|| fail(EXIT_MISMATCH, "--scenario is required"))?;
    if scenario != "m0b" {
        return Err(fail(
            EXIT_MISMATCH,
            format!("scenario {scenario:?} is not implemented"),
        ));
    }
    let private_root = private_root.ok_or_else(|| {
        fail(
            EXIT_ORACLE,
            "--private-root is required; the operator prepares it (docs/PRIVACY.md §2)",
        )
    })?;
    if !machine_green {
        return Err(fail(
            EXIT_ORACLE,
            "--m0a-machine-green is required; the operator declares that M0a is green",
        ));
    }
    Ok(Options { json, private_root })
}

/// Mounts, by path prefix, that PRIVACY §2 excludes from the private root.
const FORBIDDEN_ROOTS: [&str; 3] = ["/mnt", "/media", "/run/media"];

/// The private root must already exist and be writable; preparing it is an
/// operator precondition, not something the verifier creates from nothing.
///
/// Returns the canonical path: `--json` containment and the actual writes must
/// use the same resolved path, or a symlink could move the evidence outside.
fn ensure_private_root(root: &Path) -> Result<PathBuf, Failure> {
    if !root.is_dir() {
        return Err(fail(
            EXIT_ORACLE,
            format!(
                "private root {} is not an existing directory; prepare it first (docs/PRIVACY.md §2)",
                root.display()
            ),
        ));
    }
    let canonical = fs::canonicalize(root).map_err(|error| {
        fail(
            EXIT_ORACLE,
            format!("cannot resolve private root {}: {error}", root.display()),
        )
    })?;
    for prefix in FORBIDDEN_ROOTS {
        let prefix = Path::new(prefix);
        if canonical == prefix || canonical.starts_with(prefix) {
            return Err(fail(
                EXIT_ORACLE,
                format!(
                    "private root {} is on a Windows or removable mount, not a private root",
                    canonical.display()
                ),
            ));
        }
    }

    let work_tree = Command::new("git")
        .args(["-C"])
        .arg(&canonical)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map_err(|error| {
            fail(
                EXIT_ORACLE,
                format!("cannot run git to check the private root: {error}"),
            )
        })?;
    if String::from_utf8_lossy(&work_tree.stdout).trim() == "true" {
        return Err(fail(
            EXIT_ORACLE,
            format!(
                "private root {} is inside a git work tree; move it outside the repository",
                canonical.display()
            ),
        ));
    }

    let evidence = canonical.join("evidence").join("96");
    fs::create_dir_all(&evidence).map_err(|error| {
        fail(
            EXIT_ORACLE,
            format!(
                "private root {} is not writable: {error}",
                canonical.display()
            ),
        )
    })?;
    Ok(canonical)
}

/// `--json` must land inside the canonical private root. The parent may not
/// exist yet; the nearest existing ancestor is resolved, so a symlink inside
/// the root cannot redirect the evidence file elsewhere.
fn ensure_json_inside_root(json: &Path, root: &Path) -> Result<(), Failure> {
    let parent = json
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut probe = parent.to_path_buf();
    while !probe.exists() {
        let next = match probe.parent() {
            Some(next) if next != probe => next.to_path_buf(),
            _ => break,
        };
        probe = next;
    }
    let canonical = fs::canonicalize(&probe).map_err(|error| {
        fail(
            EXIT_ORACLE,
            format!(
                "cannot resolve the --json parent {}: {error}",
                json.display()
            ),
        )
    })?;
    if !canonical.starts_with(root) {
        return Err(fail(
            EXIT_ORACLE,
            format!(
                "--json {} is outside the private root; evidence stays private (docs/PRIVACY.md §2)",
                json.display()
            ),
        ));
    }
    Ok(())
}

/// Everything `docs/GATES.md` §1 takes from the machine.
struct Identity {
    commit: String,
    apk_sha256: String,
    device_model: String,
    driver_version: String,
    vulkan_api: String,
    page_size: String,
}

impl Identity {
    /// What the evidence JSON holds when a pre-step failed before the fields
    /// could be read; the `result` field says the run did not pass.
    fn unknown() -> Self {
        Identity {
            commit: String::new(),
            apk_sha256: String::new(),
            device_model: "unknown".to_string(),
            driver_version: "unknown".to_string(),
            vulkan_api: "unknown".to_string(),
            page_size: "unknown".to_string(),
        }
    }
}

/// Collects the identity fields on site. Device/tool failures are `3`; a
/// missing or unreadable installed APK is stale evidence (`4`).
///
/// Fields are written as they are read, so a later failure still leaves the
/// commit and any earlier value in the evidence JSON and the ledger.
fn collect_identity(adb: &Adb, identity: &mut Identity) -> Result<(), Failure> {
    identity.commit = git_commit()?;

    let model_output = adb.output(&["shell", "getprop", "ro.product.model"])?;
    let device_model = String::from_utf8_lossy(&model_output.stdout)
        .trim()
        .to_string();
    if !model_output.status.success() || device_model.is_empty() {
        return Err(fail(
            EXIT_ORACLE,
            "no device model from getprop; the device is unavailable",
        ));
    }
    identity.device_model = device_model;

    let apk_path = installed_apk_path(adb)?;
    identity.apk_sha256 = installed_apk_sha256(adb, &apk_path)?;

    let surface_flinger = adb
        .output(&["shell", "dumpsys", "SurfaceFlinger"])
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    if let Some(line) = line_containing(&surface_flinger, "driver") {
        identity.driver_version = line.to_string();
    }
    if let Some(line) = line_containing(&surface_flinger, "vulkan") {
        identity.vulkan_api = line.to_string();
    }
    if let Ok(output) = adb.output(&["shell", "getconf", "PAGE_SIZE"]) {
        let page_size = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !page_size.is_empty() {
            identity.page_size = page_size;
        }
    }
    Ok(())
}

/// `git rev-parse HEAD` with a clean tree, or `2`: the evidence would not name
/// the code that produced it.
fn git_commit() -> Result<String, Failure> {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| fail(EXIT_MISMATCH, format!("cannot run git: {error}")))?;
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .map_err(|error| fail(EXIT_MISMATCH, format!("cannot run git: {error}")))?;
    if !head.status.success() || !status.status.success() {
        return Err(fail(EXIT_MISMATCH, "cannot read the git identity"));
    }
    if !status.stdout.is_empty() {
        return Err(fail(
            EXIT_MISMATCH,
            "git status is not clean; the evidence commit would not match the tree",
        ));
    }
    let commit = String::from_utf8_lossy(&head.stdout).trim().to_string();
    if commit.is_empty() {
        return Err(fail(EXIT_MISMATCH, "git rev-parse HEAD returned nothing"));
    }
    Ok(commit)
}

/// `adb shell pm path <pkg>`, minus the `package:` prefix.
fn installed_apk_path(adb: &Adb) -> Result<String, Failure> {
    let output = adb.output(&["shell", "pm", "path", PACKAGE])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let path = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("package:"))
        .map(str::to_string);
    match path {
        Some(path) if output.status.success() && !path.is_empty() => Ok(path),
        _ => Err(fail(
            EXIT_STALE_FIXTURE,
            format!("package {PACKAGE} is not installed; the APK evidence is stale"),
        )),
    }
}

/// `adb shell sha256sum <device path>`; an unreadable APK is stale (`4`).
fn installed_apk_sha256(adb: &Adb, apk_path: &str) -> Result<String, Failure> {
    let output = adb.output(&["shell", "sha256sum", apk_path])?;
    let text = String::from_utf8_lossy(&output.stdout);
    let hash = text.split_whitespace().next().unwrap_or("");
    if !output.status.success() || !is_sha256(hash) {
        return Err(fail(
            EXIT_STALE_FIXTURE,
            format!("cannot hash the installed APK at {apk_path}; the evidence is stale"),
        ));
    }
    Ok(hash.to_string())
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// The first line containing `needle`, case-insensitively, trimmed.
fn line_containing<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    text.lines()
        .find(|line| line.to_ascii_lowercase().contains(needle))
        .map(str::trim)
}

/// The one place device commands are spawned. `NVA_ADB` overrides the program.
struct Adb {
    program: String,
}

impl Adb {
    fn new() -> Self {
        Adb {
            program: env::var("NVA_ADB").unwrap_or_else(|_| "adb".to_string()),
        }
    }

    fn output(&self, args: &[&str]) -> Result<Output, Failure> {
        Command::new(&self.program)
            .args(args)
            .output()
            .map_err(|error| {
                fail(
                    EXIT_ORACLE,
                    format!("cannot run {} {:?}: {error}", self.program, args),
                )
            })
    }

    fn text(&self, args: &[&str]) -> Result<String, Failure> {
        let output = self.output(args)?;
        if !output.status.success() {
            return Err(fail(
                EXIT_ORACLE,
                format!("{} {:?} failed", self.program, args),
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

/// The four M0b judgements of `docs/GATES.md` §4 plus the HOME counter-proof.
#[derive(Clone, Copy)]
struct M0bFacts {
    surface_generation: u32,
    presents: u32,
    pixel: u32,
    detached_submit: bool,
    presents_before: u32,
    presents_after: u32,
    crashed: bool,
}

impl M0bFacts {
    /// The all-zero facts a failed pre-step records in the evidence JSON.
    fn empty() -> Self {
        M0bFacts {
            surface_generation: 0,
            presents: 0,
            pixel: 0,
            detached_submit: false,
            presents_before: 0,
            presents_after: 0,
            crashed: false,
        }
    }
}

/// What one scenario run observed; the evidence writer turns it into JSON.
struct Capture {
    facts: M0bFacts,
    logcat: String,
    raw: Vec<u8>,
    validation_available: u32,
    validation_errors: u32,
}

/// Runs `m0b` once: clear, start, wait for attach, screenshot, HOME, wait for
/// detach. Missing snapshots are an environment timeout (`3`); a malformed
/// stream or a raw frame that does not match its header is evidence
/// disagreement (`2`).
fn capture_m0b(adb: &Adb) -> Result<Capture, Failure> {
    adb.text(&["logcat", "-c"])?;
    adb.text(&["shell", "am", "start", "-n", ACTIVITY])?;
    poll_snapshots(adb, ATTACH_TIMEOUT, "the attach snapshot", |snapshots| {
        ready_attach(snapshots).is_some()
    })?;

    let raw = screencap(adb)?;
    let pixel = centre_pixel(&raw).map_err(|error| match error {
        PixelError::UnsupportedFormat(format) => fail(
            EXIT_MISMATCH,
            format!("screencap format {format} is not RGBA_8888; the pixel cannot be sampled"),
        ),
        PixelError::Malformed => fail(
            EXIT_MISMATCH,
            "screencap is not a raw RGBA frame whose header matches its length",
        ),
    })?;

    adb.text(&["shell", "input", "keyevent", "KEYCODE_HOME"])?;
    let (logcat, _) = poll_snapshots(adb, DETACH_TIMEOUT, "the detach snapshot", |snapshots| {
        snapshots.iter().any(|snapshot| !snapshot.attached)
    })?;
    let logcat = drain_signal(adb, logcat)?;
    let snapshots = statslog::parse_stream(&logcat).map_err(|error| {
        fail(
            EXIT_MISMATCH,
            format!("cannot parse the nva-stats run: {error:?}"),
        )
    })?;

    let attached = ready_attach(&snapshots).ok_or_else(|| {
        fail(
            EXIT_MISMATCH,
            "the run has no attach snapshot with presents >= 1",
        )
    })?;
    let detached = snapshots
        .iter()
        .rev()
        .find(|snapshot| !snapshot.attached)
        .copied()
        .ok_or_else(|| fail(EXIT_MISMATCH, "the run has no detach snapshot"))?;

    Ok(Capture {
        facts: M0bFacts {
            surface_generation: attached.surface_generation,
            presents: attached.presents,
            pixel,
            detached_submit: has_detached_submit(&logcat),
            presents_before: attached.presents,
            presents_after: detached.presents,
            crashed: !process_alive(adb),
        },
        logcat,
        raw,
        validation_available: attached.validation_available,
        validation_errors: attached.validation_errors,
    })
}

/// Polls `logcat -d -s nva-stats` until `ready` accepts the snapshot sequence.
fn poll_snapshots<F>(
    adb: &Adb,
    timeout: Duration,
    what: &str,
    mut ready: F,
) -> Result<(String, Vec<Snapshot>), Failure>
where
    F: FnMut(&[Snapshot]) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let text = adb.text(&["logcat", "-d", "-s", "nva-stats"])?;
        let snapshots = statslog::parse_stream(&text).map_err(|error| {
            fail(
                EXIT_MISMATCH,
                format!("cannot parse the nva-stats run: {error:?}"),
            )
        })?;
        if ready(&snapshots) {
            return Ok((text, snapshots));
        }
        if Instant::now() >= deadline {
            return Err(fail(EXIT_ORACLE, format!("timed out waiting for {what}")));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// The attach snapshot that proves a measured surface: `attached:true` and at
/// least one present. A 0x0 `surfaceCreated` can attach with `presents:0`
/// (`docs/DEVICE-LOGS.md` §9); the next `surfaceChanged` attach is the one the
/// gate reads, so its own `presents:0` must not end the wait.
fn ready_attach(snapshots: &[Snapshot]) -> Option<Snapshot> {
    snapshots
        .iter()
        .rev()
        .find(|snapshot| snapshot.attached && snapshot.presents >= 1)
        .copied()
}

/// Reads once more after the detach snapshot, for up to
/// [`SIGNAL_DRAIN_TIMEOUT`], because the runtime writes the detach snapshot
/// before it reads and logs the post-stop present count. Accepting the first
/// read would turn "snapshot written, signal not yet written" into a false
/// pass.
fn drain_signal(adb: &Adb, mut logcat: String) -> Result<String, Failure> {
    let deadline = Instant::now() + SIGNAL_DRAIN_TIMEOUT;
    loop {
        if has_detached_submit(&logcat) || Instant::now() >= deadline {
            return Ok(logcat);
        }
        thread::sleep(POLL_INTERVAL);
        logcat = adb.text(&["logcat", "-d", "-s", "nva-stats"])?;
    }
}

fn screencap(adb: &Adb) -> Result<Vec<u8>, Failure> {
    let output = adb.output(&["exec-out", "screencap"])?;
    if !output.status.success() {
        return Err(fail(EXIT_ORACLE, "adb exec-out screencap failed"));
    }
    Ok(output.stdout)
}

/// The `detached_submit` signal is the fourth criterion's evidence, so it is
/// looked for separately: the stats parser skips it on purpose.
fn has_detached_submit(logcat: &str) -> bool {
    logcat.lines().any(|line| {
        let Some(start) = line.find('{') else {
            return false;
        };
        match json::parse(&line[start..]) {
            Ok(value) => {
                value.get("event").and_then(json::Value::as_str) == Some("detached_submit")
            }
            Err(_) => false,
        }
    })
}

/// `pidof` after the HOME detach. A missing `pidof` tool is not a crash; an
/// empty answer is.
fn process_alive(adb: &Adb) -> bool {
    match adb.output(&["shell", "pidof", PACKAGE]) {
        Ok(output) => {
            if output.status.success() {
                !String::from_utf8_lossy(&output.stdout).trim().is_empty()
            } else {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
                .to_ascii_lowercase();
                text.contains("not found")
            }
        }
        Err(_) => true,
    }
}

/// Why a raw screencap cannot be sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PixelError {
    /// The header or its declared length is not a raw frame this reader uses.
    Malformed,
    /// Only RGBA_8888 is decoded; the header names another format.
    UnsupportedFormat(u32),
}

/// Reads the centre pixel from `adb exec-out screencap`'s raw stream.
///
/// The header is `u32 width, u32 height, u32 format` and, on Android 10+, one
/// extra `u32 colorspace`; the header size is whatever remains after `w*h*4`
/// bytes, so both 12 and 16 byte headers are accepted. Only format `1`
/// (`PixelFormat.RGBA_8888`) is decoded — the 4-byte pixel packing and the
/// `0xff00ff00` expectation both depend on it. An unexpected format is
/// evidence disagreement (`2`), not a missing device (`3`). No PNG decoding.
fn centre_pixel(raw: &[u8]) -> Result<u32, PixelError> {
    if raw.len() < 12 {
        return Err(PixelError::Malformed);
    }
    let width = u32::from_le_bytes(raw[0..4].try_into().map_err(|_| PixelError::Malformed)?);
    let height = u32::from_le_bytes(raw[4..8].try_into().map_err(|_| PixelError::Malformed)?);
    let format = u32::from_le_bytes(raw[8..12].try_into().map_err(|_| PixelError::Malformed)?);
    if format != ANDROID_FORMAT_RGBA_8888 {
        return Err(PixelError::UnsupportedFormat(format));
    }
    if width == 0 || height == 0 {
        return Err(PixelError::Malformed);
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(PixelError::Malformed)?;
    let header = raw.len().checked_sub(pixels).ok_or(PixelError::Malformed)?;
    if header != 12 && header != 16 {
        return Err(PixelError::Malformed);
    }
    let x = width as usize / 2;
    let y = height as usize / 2;
    let offset = header + (y * width as usize + x) * 4;
    let bytes = raw.get(offset..offset + 4).ok_or(PixelError::Malformed)?;
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| PixelError::Malformed)?,
    ))
}

/// The independent M0b verdict. It checks only `docs/GATES.md` §4's criteria;
/// in particular, it never looks at llvmpipe or validation availability.
fn verdict_m0b(facts: &M0bFacts) -> i32 {
    if facts.surface_generation != 1 {
        return EXIT_MISMATCH;
    }
    if facts.presents == 0 {
        return EXIT_MISMATCH;
    }
    if facts.pixel != PIXEL_GREEN {
        return EXIT_MISMATCH;
    }
    // The HOME counter-proof is the `detached_submit` count: it is the number
    // of presents that entered after stop was set, so an absent signal means
    // `presents` did not grow after HOME. The attach and detach totals are
    // deliberately not compared: the render loop presents continuously between
    // attach and HOME, so a positive delta is normal, not a violation.
    if facts.detached_submit {
        return EXIT_MISMATCH;
    }
    if facts.crashed {
        return EXIT_MISMATCH;
    }
    EXIT_OK
}

fn result_word(exit: i32) -> &'static str {
    match exit {
        EXIT_OK => "pass",
        EXIT_MISMATCH => "mismatch",
        EXIT_ORACLE => "unavailable",
        _ => "stale",
    }
}

fn bool_word(flag: bool) -> &'static str {
    if flag {
        "true"
    } else {
        "false"
    }
}

/// One `device` invocation's evidence and its ledger entry.
struct Report {
    options: Options,
    run_id: String,
    ts_utc: String,
    identity: Identity,
    capture: Option<Capture>,
    detail: String,
}

impl Report {
    fn new(options: Options) -> Self {
        let (run_id, ts_utc) = utc_now();
        Report {
            options,
            run_id,
            ts_utc,
            identity: Identity::unknown(),
            capture: None,
            detail: String::new(),
        }
    }

    fn facts(&self) -> M0bFacts {
        self.capture
            .as_ref()
            .map_or_else(M0bFacts::empty, |capture| capture.facts)
    }

    fn success_detail(&self, code: i32) -> String {
        let facts = self.facts();
        format!(
            "m0b commit={} model={} surface_generation={} presents={} \
             pixel_rgba=0x{:08x} detached_submit={} presents_before={} \
             presents_after={} crashed={} result={}",
            self.identity.commit,
            self.identity.device_model,
            facts.surface_generation,
            facts.presents,
            facts.pixel,
            facts.detached_submit,
            facts.presents_before,
            facts.presents_after,
            facts.crashed,
            result_word(code),
        )
    }

    /// Writes the evidence files and the ledger entry, then returns `exit`.
    /// An unwritable private root turns into `4`: the run's evidence is stale.
    fn finish(&self, exit: i32) -> i32 {
        eprintln!("device: {}", self.detail);
        let result = result_word(exit);
        let json = self.evidence_json(result);
        match self.write_evidence(&json, result) {
            Ok(()) => exit,
            Err(error) => {
                eprintln!("device: cannot write evidence: {error}");
                EXIT_STALE_FIXTURE
            }
        }
    }

    fn evidence_json(&self, result: &str) -> String {
        let facts = self.facts();
        let (validation_available, validation_errors) =
            self.capture.as_ref().map_or((0, 0), |capture| {
                (capture.validation_available, capture.validation_errors)
            });
        format!(
            "{{\"scenario\":\"m0b\",\"commit\":{},\"apk_sha256\":{},\"pack_id\":\"none\",\
             \"build_id\":\"none\",\"device_model\":{},\"driver_version\":{},\
             \"vulkan_api\":{},\"page_size\":{},\"run_id\":{},\"ts_utc\":{},\
             \"m0a_machine_green\":true,\"surface_generation\":{},\"presents\":{},\
             \"pixel_rgba\":\"0x{:08x}\",\"surface_destroyed_still_submitting\":{},\
             \"presents_before\":{},\"presents_after\":{},\"crashed\":{},\
             \"validation_available\":{validation_available},\
             \"validation_errors\":{validation_errors},\"result\":{}}}",
            json::quote(&self.identity.commit),
            json::quote(&self.identity.apk_sha256),
            json::quote(&self.identity.device_model),
            json::quote(&self.identity.driver_version),
            json::quote(&self.identity.vulkan_api),
            json::quote(&self.identity.page_size),
            json::quote(&self.run_id),
            json::quote(&self.ts_utc),
            facts.surface_generation,
            facts.presents,
            facts.pixel,
            bool_word(facts.detached_submit),
            facts.presents_before,
            facts.presents_after,
            bool_word(facts.crashed),
            json::quote(result),
        )
    }

    fn write_evidence(&self, json: &str, result: &str) -> Result<(), String> {
        let evidence_dir = self.options.private_root.join("evidence").join("96");
        fs::create_dir_all(&evidence_dir)
            .map_err(|error| format!("cannot create {}: {error}", evidence_dir.display()))?;

        let mut files: Vec<PathBuf> = Vec::new();
        if let Some(capture) = &self.capture {
            if !capture.raw.is_empty() {
                let path = evidence_dir.join(format!("m0b-{}.raw", self.run_id));
                fs::write(&path, &capture.raw)
                    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                files.push(path);
            }
            if !capture.logcat.is_empty() {
                let path = evidence_dir.join(format!("m0b-{}.logcat", self.run_id));
                fs::write(&path, capture.logcat.as_bytes())
                    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                files.push(path);
            }
        }

        match &self.options.json {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
                }
                fs::write(path, format!("{json}\n"))
                    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
                files.push(path.clone());
            }
            None => println!("{json}"),
        }

        // `evidence_sha256[]` is ordered by evidence file name, so the entry is
        // stable regardless of which file was written first.
        let mut hashes: Vec<(String, String)> = Vec::with_capacity(files.len());
        for path in &files {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_string();
            hashes.push((name, sha256_file(path)?));
        }
        hashes.sort_by(|left, right| left.0.cmp(&right.0));
        let hashes = hashes
            .iter()
            .map(|(_, hash)| format!("\"{hash}\""))
            .collect::<Vec<_>>()
            .join(",");

        let entry = format!(
            "{{\"gate\":\"m0b\",\"commit\":{},\"apk_sha256\":{},\"pack_id\":\"none\",\
             \"run_id\":{},\"ts_utc\":{},\"result\":{},\"evidence_sha256\":[{hashes}]}}\n",
            json::quote(&self.identity.commit),
            json::quote(&self.identity.apk_sha256),
            json::quote(&self.run_id),
            json::quote(&self.ts_utc),
            json::quote(result),
        );
        let ledger = self.options.private_root.join("gate-ledger.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ledger)
            .map_err(|error| format!("cannot open {}: {error}", ledger.display()))?;
        file.write_all(entry.as_bytes())
            .map_err(|error| format!("cannot append {}: {error}", ledger.display()))?;
        Ok(())
    }
}

/// SHA-256 through the system `sha256sum`; the workspace has no hash crate.
fn sha256_file(path: &Path) -> Result<String, String> {
    let output = Command::new("sha256sum")
        .arg("--")
        .arg(path)
        .output()
        .map_err(|error| format!("cannot run sha256sum: {error}"))?;
    if !output.status.success() {
        return Err(format!("sha256sum failed for {}", path.display()));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let hash = text.split_whitespace().next().unwrap_or_default();
    if is_sha256(hash) {
        Ok(hash.to_string())
    } else {
        Err(format!("sha256sum gave no digest for {}", path.display()))
    }
}

fn utc_now() -> (String, String) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    let (timestamp, compact) = utc_from_seconds(seconds);
    (format!("{compact}-{}", std::process::id()), timestamp)
}

/// `(ISO8601, compact)` UTC for one Unix timestamp, without a date crate.
fn utc_from_seconds(seconds: i64) -> (String, String) {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = of_day / 3_600;
    let minute = (of_day % 3_600) / 60;
    let second = of_day % 60;
    (
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
        format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z"),
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to a civil date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{
        centre_pixel, is_sha256, line_containing, ready_attach, utc_from_seconds, verdict_m0b,
        M0bFacts, PixelError, Snapshot, PIXEL_GREEN,
    };

    fn snapshot(attached: bool, presents: u32) -> Snapshot {
        Snapshot {
            presents,
            surface_generation: 1,
            attached,
            generation: 1,
            submits: 1,
            validation_available: 0,
            validation_errors: 0,
        }
    }

    fn facts() -> M0bFacts {
        M0bFacts {
            surface_generation: 1,
            presents: 1,
            pixel: PIXEL_GREEN,
            detached_submit: false,
            presents_before: 1,
            presents_after: 1,
            crashed: false,
        }
    }

    #[test]
    fn utc_formatting_matches_iso8601() {
        let (timestamp, compact) = utc_from_seconds(1_700_000_000);
        assert_eq!(timestamp, "2023-11-14T22:13:20Z");
        assert_eq!(compact, "20231114T221320Z");
        assert_eq!(utc_from_seconds(0).0, "1970-01-01T00:00:00Z");
    }

    #[test]
    fn sha256_shape_is_strict() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
        assert!(!is_sha256(&"g".repeat(64)));
    }

    #[test]
    fn line_containing_is_case_insensitive() {
        let text = "GLES: vendor\nVulkan API level: 1.1.128\ndriver version: 615.50\n";
        assert_eq!(
            line_containing(text, "vulkan"),
            Some("Vulkan API level: 1.1.128")
        );
        assert_eq!(
            line_containing(text, "driver"),
            Some("driver version: 615.50")
        );
        assert_eq!(line_containing(text, "absent"), None);
    }

    #[test]
    fn centre_pixel_reads_12_and_16_byte_headers() {
        for header in [12usize, 16] {
            let width = 4u32;
            let height = 4u32;
            let mut raw = Vec::new();
            raw.extend_from_slice(&width.to_le_bytes());
            raw.extend_from_slice(&height.to_le_bytes());
            raw.extend_from_slice(&1u32.to_le_bytes());
            raw.resize(header, 0);
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            let offset = ((height / 2 * width + width / 2) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&PIXEL_GREEN.to_le_bytes());
            raw.extend_from_slice(&pixels);
            assert_eq!(centre_pixel(&raw), Ok(PIXEL_GREEN), "header {header}");
        }
    }

    #[test]
    fn centre_pixel_rejects_a_mismatched_length() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&1u32.to_le_bytes());
        raw.extend_from_slice(&[0u8; 8]); // far short of 4 * 4 * 4 data bytes
        assert_eq!(centre_pixel(&raw), Err(PixelError::Malformed));
    }

    #[test]
    fn centre_pixel_rejects_a_non_rgba_format() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&4u32.to_le_bytes());
        raw.extend_from_slice(&2u32.to_le_bytes()); // RGBX_8888, not RGBA_8888
        raw.extend_from_slice(&[0u8; 16]);
        assert_eq!(centre_pixel(&raw), Err(PixelError::UnsupportedFormat(2)));
    }

    #[test]
    fn zero_present_attach_is_not_ready() {
        assert_eq!(ready_attach(&[snapshot(true, 0)]), None);
        let measured = [snapshot(true, 0), snapshot(true, 1)];
        assert_eq!(
            ready_attach(&measured),
            Some(snapshot(true, 1)),
            "the next attach with a present is the one the gate reads"
        );
    }

    #[test]
    fn green_run_passes_without_validation_or_llvmpipe() {
        assert_eq!(verdict_m0b(&facts()), 0);
    }

    #[test]
    fn each_m0b_criterion_fails_alone() {
        let mut wrong_generation = facts();
        wrong_generation.surface_generation = 2;
        assert_eq!(verdict_m0b(&wrong_generation), 2);

        let mut no_presents = facts();
        no_presents.presents = 0;
        assert_eq!(verdict_m0b(&no_presents), 2);

        let mut wrong_pixel = facts();
        wrong_pixel.pixel = 0xff0000ff;
        assert_eq!(verdict_m0b(&wrong_pixel), 2);

        let mut detached_submit = facts();
        detached_submit.detached_submit = true;
        assert_eq!(verdict_m0b(&detached_submit), 2);

        let mut crashed = facts();
        crashed.crashed = true;
        assert_eq!(verdict_m0b(&crashed), 2);
    }

    #[test]
    fn rendering_between_attach_and_home_does_not_fail() {
        let mut facts = facts();
        facts.presents_after = 9;
        assert_eq!(
            verdict_m0b(&facts),
            0,
            "the loop presents between attach and HOME; only post-stop presents fail"
        );
    }
}
