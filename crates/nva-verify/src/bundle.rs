//! `nva-verify device --from-bundle` — the offline M0b acceptance package (#109).
//!
//! The product APK's acceptance mode collects raw evidence (identity JSON, a
//! raw centre-pixel frame, the shell log and a sorted SHA-256 manifest) and
//! shares it as a zip. This module is the PC side that judges it: it reads the
//! zip through the system `unzip -p`, checks the manifest with the system
//! `sha256sum`, parses the evidence, and reuses [`crate::device`]'s
//! `verdict_m0b` and ledger writer so the app-carried and adb-carried gates
//! cannot drift.
//!
//! Exit codes are the workspace contract: `0` pass, `2` the evidence disagrees
//! (bad JSON, wrong pixel, missing snapshot), `3` a tool is unavailable, `4`
//! the bundle is stale (missing entry, SHA-256 mismatch, unreadable zip).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::device::{
    append_ledger, bool_word, centre_pixel, ensure_json_inside_root, ensure_private_root, fail,
    has_detached_submit, is_sha256, ready_attach, result_word, sha256_file, utc_now, verdict_m0b,
    Failure, LedgerEntry, M0bFacts, PixelError,
};
use crate::json::{self, Value};
use crate::loadplan::{EXIT_MISMATCH, EXIT_STALE_FIXTURE};
use crate::smoke::EXIT_ORACLE;
use crate::statslog;

/// The manifest entry name. The app writes `sha256.txt` beside the evidence.
const MANIFEST: &str = "sha256.txt";
const EVIDENCE_JSON: &str = "m0b.json";
const EVIDENCE_RAW: &str = "screen.raw";
const EVIDENCE_LOG: &str = "nva-shell.log";
/// The evidence files the manifest must list, in ledger order (name ascending).
const REQUIRED_EVIDENCE: [&str; 3] = [EVIDENCE_JSON, EVIDENCE_LOG, EVIDENCE_RAW];

/// Parsed `device --from-bundle` command line.
struct Options {
    bundle: PathBuf,
    private_root: PathBuf,
    json: Option<PathBuf>,
}

/// Entry point called from `device::run_cli` when `--from-bundle` is present.
pub fn run_cli(args: &[String]) -> i32 {
    match parse_args(args) {
        Ok(options) => run(options),
        Err(failure) => {
            eprintln!("device: {}", failure.detail);
            failure.exit
        }
    }
}

/// Parses `--from-bundle <zip> --private-root <path> [--json <out>]`.
///
/// The bundle carrier does not take `--scenario` or `--m0a-machine-green`: the
/// scenario is fixed by the evidence and the operator declaration travels in
/// `m0b.json`, where the app recorded it.
fn parse_args(args: &[String]) -> Result<Options, Failure> {
    let mut bundle: Option<PathBuf> = None;
    let mut private_root: Option<PathBuf> = None;
    let mut json: Option<PathBuf> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--from-bundle" => {
                index += 1;
                bundle = Some(PathBuf::from(args.get(index).cloned().ok_or_else(
                    || fail(EXIT_MISMATCH, "--from-bundle needs a zip path"),
                )?));
            }
            "--private-root" => {
                index += 1;
                private_root =
                    Some(PathBuf::from(args.get(index).cloned().ok_or_else(
                        || fail(EXIT_ORACLE, "--private-root needs a path"),
                    )?));
            }
            "--json" => {
                // Like `device`, `--json` alone means stdout; a following
                // non-option value also writes the evidence file.
                if let Some(next) = args.get(index + 1) {
                    if !next.starts_with("--") {
                        json = Some(PathBuf::from(next));
                        index += 1;
                    }
                }
            }
            other => {
                return Err(fail(EXIT_MISMATCH, format!("unknown option {other}")));
            }
        }
        index += 1;
    }

    let bundle = bundle.ok_or_else(|| fail(EXIT_MISMATCH, "--from-bundle is required"))?;
    let private_root = private_root.ok_or_else(|| {
        fail(
            EXIT_ORACLE,
            "--private-root is required; the operator prepares it (docs/PRIVACY.md §2)",
        )
    })?;
    Ok(Options {
        bundle,
        private_root,
        json,
    })
}

fn run(mut options: Options) -> i32 {
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
    match report.evaluate() {
        Ok(code) => {
            report.detail = report.success_detail(code);
            report.finish(code)
        }
        Err(failure) => {
            report.detail = failure.detail;
            report.finish(failure.exit)
        }
    }
}

/// The identity fields `docs/GATES.md` §1 reads. The bundle carries them from
/// the phone; unknowns stay `unknown` (driver/Vulkan are not exposed by the app
/// yet, and a new ABI export is out of scope).
struct Identity {
    commit: String,
    apk_sha256: String,
    device_model: String,
    driver_version: String,
    vulkan_api: String,
    page_size: String,
}

impl Identity {
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

/// One `--from-bundle` invocation's evidence and ledger entry.
struct Report {
    options: Options,
    run_id: String,
    ts_utc: String,
    identity: Identity,
    facts: M0bFacts,
    /// Evidence digests in ledger order (name ascending), computed from the
    /// copies retained under `<private-root>/evidence/109/<run_id>/`.
    evidence_sha256: Vec<String>,
    m0a_machine_green: bool,
    /// `m0b.json`'s own pixel claim, cross-checked against `screen.raw`.
    pixel_sampled: bool,
    recorded_pixel: u32,
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
            facts: M0bFacts::empty(),
            evidence_sha256: Vec::new(),
            m0a_machine_green: false,
            pixel_sampled: false,
            recorded_pixel: 0,
            detail: String::new(),
        }
    }

    /// Reads and judges the bundle, leaving the parsed identity and facts in
    /// `self` even when a later step fails, so a failure still reaches the
    /// evidence JSON and the ledger.
    fn evaluate(&mut self) -> Result<i32, Failure> {
        // An absolute path cannot be read as an option by `unzip`, and the
        // entry names are validated before they reach the command line.
        let zip = fs::canonicalize(&self.options.bundle).map_err(|error| {
            fail(
                EXIT_STALE_FIXTURE,
                format!(
                    "cannot resolve bundle {}; the evidence is unreadable or stale: {error}",
                    self.options.bundle.display()
                ),
            )
        })?;
        let manifest = read_manifest(&zip)?;

        let mut contents: Vec<(String, Vec<u8>)> = Vec::with_capacity(manifest.len());
        for (name, expected) in &manifest {
            let bytes = read_entry(&zip, name)?;
            let actual = sha256_bytes(&bytes)?;
            if &actual != expected {
                return Err(fail(
                    EXIT_STALE_FIXTURE,
                    format!("{name} does not match sha256.txt; the bundle is stale"),
                ));
            }
            contents.push((name.clone(), bytes));
        }

        let evidence = take(&mut contents, EVIDENCE_JSON)?;
        let raw = take(&mut contents, EVIDENCE_RAW)?;
        let log = take(&mut contents, EVIDENCE_LOG)?;

        // Retain the bytes before judging (P1): even a mismatch-led run leaves
        // the exact evidence the ledger hashes point at. Retaining under the
        // bundle's run id happens after `parse_evidence` sets it.
        let parsed = self.parse_evidence(&evidence);
        self.retain_evidence(&evidence, &raw, &log)?;
        parsed?;

        // The raw frame is the only pixel source. `pixel_sampled` must be true
        // and `m0b.json`'s claim must match the frame, so a stale raw file from
        // an earlier run cannot make a new, unsampled run look green.
        if !self.pixel_sampled {
            return Err(fail(
                EXIT_MISMATCH,
                "m0b.json pixel_sampled is false; the frame was not sampled and cannot be \
                 called green",
            ));
        }
        let pixel = centre_pixel(&raw).map_err(pixel_failure)?;
        if self.recorded_pixel != pixel {
            return Err(fail(
                EXIT_MISMATCH,
                format!(
                    "m0b.json pixel_rgba 0x{:08x} disagrees with screen.raw 0x{pixel:08x}",
                    self.recorded_pixel
                ),
            ));
        }
        self.parse_log(&log, pixel)?;
        Ok(verdict_m0b(&self.facts))
    }

    /// Copies the three evidence files into `<private-root>/evidence/109/
    /// <run_id>/` and computes the ledger digests from those copies, so the
    /// retained bytes can be re-hashed later and still match the ledger.
    fn retain_evidence(&mut self, evidence: &[u8], raw: &[u8], log: &[u8]) -> Result<(), Failure> {
        if !safe_run_id(&self.run_id) {
            return Err(fail(
                EXIT_MISMATCH,
                format!(
                    "m0b.json run_id {:?} is not a safe evidence directory name",
                    self.run_id
                ),
            ));
        }
        let directory = self
            .options
            .private_root
            .join("evidence")
            .join("109")
            .join(&self.run_id);
        fs::create_dir_all(&directory).map_err(|error| {
            fail(
                EXIT_STALE_FIXTURE,
                format!(
                    "cannot create evidence directory {}: {error}",
                    directory.display()
                ),
            )
        })?;

        let files: [(&str, &[u8]); 3] = [
            (EVIDENCE_JSON, evidence),
            (EVIDENCE_LOG, log),
            (EVIDENCE_RAW, raw),
        ];
        let mut hashes: Vec<(String, String)> = Vec::with_capacity(files.len());
        for (name, bytes) in files {
            let path = directory.join(name);
            fs::write(&path, bytes).map_err(|error| {
                fail(
                    EXIT_STALE_FIXTURE,
                    format!("cannot retain {}: {error}", path.display()),
                )
            })?;
            let digest = sha256_file(&path).map_err(|error| {
                fail(
                    EXIT_STALE_FIXTURE,
                    format!("cannot hash retained {}: {error}", path.display()),
                )
            })?;
            hashes.push((name.to_string(), digest));
        }
        hashes.sort_by(|left, right| left.0.cmp(&right.0));
        self.evidence_sha256 = hashes.into_iter().map(|(_, hash)| hash).collect();
        Ok(())
    }

    /// Fills the identity, `crashed` and `m0a_machine_green` from `m0b.json`.
    /// A missing required field or malformed JSON is evidence disagreement
    /// (`2`), the same class as a wrong pixel.
    fn parse_evidence(&mut self, bytes: &[u8]) -> Result<(), Failure> {
        let text =
            std::str::from_utf8(bytes).map_err(|_| fail(EXIT_MISMATCH, "m0b.json is not UTF-8"))?;
        let value = json::parse(text)
            .map_err(|why| fail(EXIT_MISMATCH, format!("m0b.json is not JSON: {why}")))?;

        self.identity.commit = required_string(&value, "commit")?;
        let apk_sha256 = required_string(&value, "apk_sha256")?;
        if !is_sha256(&apk_sha256) {
            return Err(fail(
                EXIT_MISMATCH,
                "m0b.json apk_sha256 is not a SHA-256 hex digest",
            ));
        }
        self.identity.apk_sha256 = apk_sha256;
        self.run_id = required_string(&value, "run_id")?;
        self.ts_utc = required_string(&value, "ts_utc")?;
        self.facts.crashed = required_bool(&value, "crashed")?;
        // Only a run the app itself finished is evidence. A partial run (HOME
        // before the pixel, a missing log, or a crash the app survived) cannot
        // be judged as a pass, even if the counters happen to look right.
        let phase = required_string(&value, "evidence")?;
        if phase != "complete" {
            return Err(fail(
                EXIT_MISMATCH,
                format!("m0b.json evidence is {phase:?}, not \"complete\"; the run did not finish"),
            ));
        }
        if let Some(message) = value.get("log_error").and_then(Value::as_str) {
            if !message.is_empty() {
                return Err(fail(
                    EXIT_MISMATCH,
                    format!("m0b.json records a log problem: {message}"),
                ));
            }
        }
        self.pixel_sampled = required_bool(&value, "pixel_sampled")?;
        let pixel_rgba = required_string(&value, "pixel_rgba")?;
        self.recorded_pixel = parse_pixel_rgba(&pixel_rgba).ok_or_else(|| {
            fail(
                EXIT_MISMATCH,
                format!("m0b.json pixel_rgba {pixel_rgba:?} is not a 0xAARRGGBB hex value"),
            )
        })?;
        self.m0a_machine_green = optional_bool(&value, "m0a_machine_green").unwrap_or(false);
        if let Some(model) = optional_string(&value, "device_model") {
            self.identity.device_model = model;
        }
        if let Some(value) = optional_string(&value, "driver_version") {
            self.identity.driver_version = value;
        }
        if let Some(value) = optional_string(&value, "vulkan_api") {
            self.identity.vulkan_api = value;
        }
        if let Some(value) = optional_string(&value, "page_size") {
            self.identity.page_size = value;
        }
        Ok(())
    }

    /// Parses the shell log the app shared. The log may hold more than one run
    /// (the normal shell wrote its own run before acceptance started), so the
    /// slice after the last `=== run` header is what `statslog` reads; a
    /// counter reset across runs would otherwise read as `OutOfOrder`
    /// (`docs/DEVICE-LOGS.md` §9).
    fn parse_log(&mut self, bytes: &[u8], pixel: u32) -> Result<(), Failure> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| fail(EXIT_MISMATCH, "nva-shell.log is not UTF-8"))?;
        let run = last_run(text);
        let snapshots = statslog::parse_stream(run).map_err(|error| {
            fail(
                EXIT_MISMATCH,
                format!("cannot parse the nva-stats run: {error:?}"),
            )
        })?;
        let attached = ready_attach(&snapshots).ok_or_else(|| {
            fail(
                EXIT_MISMATCH,
                "the bundle has no attach snapshot with presents >= 1",
            )
        })?;
        let detached = snapshots
            .iter()
            .rev()
            .find(|snapshot| !snapshot.attached)
            .copied()
            .ok_or_else(|| fail(EXIT_MISMATCH, "the bundle has no detach snapshot"))?;

        self.facts.surface_generation = attached.surface_generation;
        self.facts.presents = attached.presents;
        self.facts.pixel = pixel;
        self.facts.detached_submit = has_detached_submit(run);
        self.facts.presents_before = attached.presents;
        self.facts.presents_after = detached.presents;
        Ok(())
    }

    fn success_detail(&self, code: i32) -> String {
        // The device model stays in the private evidence JSON only; stderr can
        // be pasted into a ticket or a log, so it must not carry it.
        format!(
            "m0b source=bundle commit={} surface_generation={} presents={} \
             pixel_rgba=0x{:08x} detached_submit={} presents_before={} \
             presents_after={} crashed={} result={}",
            self.identity.commit,
            self.facts.surface_generation,
            self.facts.presents,
            self.facts.pixel,
            bool_word(self.facts.detached_submit),
            self.facts.presents_before,
            self.facts.presents_after,
            bool_word(self.facts.crashed),
            result_word(code),
        )
    }

    /// Writes the evidence JSON and the ledger entry, then returns `exit`.
    fn finish(&self, exit: i32) -> i32 {
        eprintln!("device: {}", self.detail);
        let result = result_word(exit);
        let json = self.evidence_json(result);
        match self.write(&json, result) {
            Ok(()) => exit,
            Err(error) => {
                eprintln!("device: cannot write evidence: {error}");
                EXIT_STALE_FIXTURE
            }
        }
    }

    fn evidence_json(&self, result: &str) -> String {
        format!(
            "{{\"scenario\":\"m0b\",\"source\":\"bundle\",\"commit\":{},\"apk_sha256\":{},\
             \"pack_id\":\"none\",\"build_id\":\"none\",\"device_model\":{},\
             \"driver_version\":{},\"vulkan_api\":{},\"page_size\":{},\"run_id\":{},\
             \"ts_utc\":{},\"m0a_machine_green\":{},\"surface_generation\":{},\
             \"presents\":{},\"pixel_rgba\":\"0x{:08x}\",\
             \"surface_destroyed_still_submitting\":{},\"presents_before\":{},\
             \"presents_after\":{},\"crashed\":{},\"result\":{}}}",
            json::quote(&self.identity.commit),
            json::quote(&self.identity.apk_sha256),
            json::quote(&self.identity.device_model),
            json::quote(&self.identity.driver_version),
            json::quote(&self.identity.vulkan_api),
            json::quote(&self.identity.page_size),
            json::quote(&self.run_id),
            json::quote(&self.ts_utc),
            bool_word(self.m0a_machine_green),
            self.facts.surface_generation,
            self.facts.presents,
            self.facts.pixel,
            bool_word(self.facts.detached_submit),
            self.facts.presents_before,
            self.facts.presents_after,
            bool_word(self.facts.crashed),
            json::quote(result),
        )
    }

    fn write(&self, json: &str, result: &str) -> Result<(), String> {
        match &self.options.json {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
                }
                fs::write(path, format!("{json}\n"))
                    .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
            }
            None => println!("{json}"),
        }
        append_ledger(
            &self.options.private_root,
            &LedgerEntry {
                commit: &self.identity.commit,
                apk_sha256: &self.identity.apk_sha256,
                run_id: &self.run_id,
                ts_utc: &self.ts_utc,
                result,
                evidence_sha256: &self.evidence_sha256,
            },
        )
    }
}

/// Reads `sha256.txt` and checks it lists exactly the required evidence files.
fn read_manifest(zip: &Path) -> Result<Vec<(String, String)>, Failure> {
    let bytes = read_entry(zip, MANIFEST)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| fail(EXIT_STALE_FIXTURE, "sha256.txt is not UTF-8"))?;
    let mut entries: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else {
            return Err(malformed_manifest(line));
        };
        let Some(name) = parts.next() else {
            return Err(malformed_manifest(line));
        };
        if parts.next().is_some() || !is_sha256(hash) || !safe_entry_name(name) {
            return Err(malformed_manifest(line));
        }
        entries.push((name.to_string(), hash.to_string()));
    }

    let mut names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    if names != REQUIRED_EVIDENCE {
        return Err(fail(
            EXIT_STALE_FIXTURE,
            format!("sha256.txt must list exactly {REQUIRED_EVIDENCE:?}; got {names:?}"),
        ));
    }
    Ok(entries)
}

fn malformed_manifest(line: &str) -> Failure {
    fail(
        EXIT_STALE_FIXTURE,
        format!("sha256.txt has a malformed line: {line:?}"),
    )
}

/// An entry this reader will pass to `unzip -p`: no path separators, no
/// `..`, no leading dash that could be read as an option.
fn safe_entry_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('-')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// Reads one zip entry through the system `unzip -p`. The archive path is
/// already absolute and every entry name is validated, so neither can be read
/// as an option.
///
/// Info-ZIP accepts `--`; busybox's applet treats it as a file name. The first
/// attempt uses the separator; a failed attempt is retried without it. The
/// absolute path and [`safe_entry_name`] already make option injection
/// impossible, so the retry is a compatibility path, not a weaker check.
///
/// A non-zero result after both means the zip or the entry is unreadable, which
/// is stale evidence (`4`). Some `unzip` builds return `0` with empty output for
/// a missing entry; the SHA-256 comparison catches that later.
fn read_entry(zip: &Path, name: &str) -> Result<Vec<u8>, Failure> {
    let with_separator = Command::new("unzip")
        .arg("-p")
        .arg("--")
        .arg(zip)
        .arg(name)
        .output()
        .map_err(|error| fail(EXIT_ORACLE, format!("cannot run unzip: {error}")))?;
    if with_separator.status.success() {
        return Ok(with_separator.stdout);
    }
    let without_separator = Command::new("unzip")
        .arg("-p")
        .arg(zip)
        .arg(name)
        .output()
        .map_err(|error| fail(EXIT_ORACLE, format!("cannot run unzip: {error}")))?;
    if !without_separator.status.success() {
        return Err(fail(
            EXIT_STALE_FIXTURE,
            format!(
                "cannot read {name} from {}; the bundle is unreadable or stale",
                zip.display()
            ),
        ));
    }
    Ok(without_separator.stdout)
}

/// Parses `m0b.json`'s pixel claim: the `0xAARRGGBB` form the app writes with
/// `String.format("%08x", argb)`. Anything else cannot be compared, so it is
/// evidence disagreement.
fn parse_pixel_rgba(text: &str) -> Option<u32> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))?;
    if digits.len() != 8 || !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok()
}

/// A run id becomes a directory name under the private root, so it must not
/// escape it.
fn safe_run_id(run_id: &str) -> bool {
    safe_entry_name(run_id) && run_id.len() <= 128
}

/// SHA-256 of in-memory evidence through the system `sha256sum`, which reads
/// standard input when given no file.
fn sha256_bytes(bytes: &[u8]) -> Result<String, Failure> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| fail(EXIT_ORACLE, format!("cannot run sha256sum: {error}")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| fail(EXIT_ORACLE, "cannot write to sha256sum"))?;
    stdin
        .write_all(bytes)
        .map_err(|error| fail(EXIT_ORACLE, format!("cannot write to sha256sum: {error}")))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| fail(EXIT_ORACLE, format!("sha256sum failed: {error}")))?;
    if !output.status.success() {
        return Err(fail(
            EXIT_STALE_FIXTURE,
            "sha256sum failed on extracted evidence",
        ));
    }
    let hash = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    if !is_sha256(&hash) {
        return Err(fail(
            EXIT_STALE_FIXTURE,
            "sha256sum gave no digest for extracted evidence",
        ));
    }
    Ok(hash)
}

fn take(contents: &mut Vec<(String, Vec<u8>)>, name: &str) -> Result<Vec<u8>, Failure> {
    match contents.iter().position(|(entry, _)| entry == name) {
        Some(index) => Ok(contents.remove(index).1),
        None => Err(fail(
            EXIT_STALE_FIXTURE,
            format!("sha256.txt does not list {name}"),
        )),
    }
}

fn pixel_failure(error: PixelError) -> Failure {
    match error {
        PixelError::UnsupportedFormat(format) => fail(
            EXIT_MISMATCH,
            format!("screen.raw format {format} is not RGBA_8888; the pixel cannot be sampled"),
        ),
        PixelError::Malformed => fail(
            EXIT_MISMATCH,
            "screen.raw is not a raw RGBA frame whose header matches its length",
        ),
    }
}

/// The slice after the last `=== run ` header, or the whole text when there is
/// none. One run's counters restart at zero, so mixing runs would read as an
/// out-of-order stream.
fn last_run(text: &str) -> &str {
    let mut start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with("=== run ") {
            start = offset;
        }
        offset += line.len();
    }
    &text[start..]
}

fn required_string(value: &Value, name: &'static str) -> Result<String, Failure> {
    match value.get(name).and_then(Value::as_str) {
        Some(text) if !text.is_empty() => Ok(text.to_string()),
        Some(_) => Err(fail(
            EXIT_MISMATCH,
            format!("m0b.json field {name} is empty"),
        )),
        None => Err(fail(
            EXIT_MISMATCH,
            format!("m0b.json is missing string {name}"),
        )),
    }
}

fn required_bool(value: &Value, name: &'static str) -> Result<bool, Failure> {
    match value.get(name) {
        Some(Value::Bool(flag)) => Ok(*flag),
        _ => Err(fail(
            EXIT_MISMATCH,
            format!("m0b.json is missing boolean {name}"),
        )),
    }
}

fn optional_string(value: &Value, name: &str) -> Option<String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn optional_bool(value: &Value, name: &str) -> Option<bool> {
    match value.get(name) {
        Some(Value::Bool(flag)) => Some(*flag),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{last_run, parse_pixel_rgba, safe_entry_name, safe_run_id};

    #[test]
    fn safe_entry_names_reject_paths_and_options() {
        assert!(safe_entry_name("m0b.json"));
        assert!(safe_entry_name("screen.raw"));
        assert!(!safe_entry_name("../m0b.json"));
        assert!(!safe_entry_name("dir/m0b.json"));
        assert!(!safe_entry_name("-p"));
        assert!(!safe_entry_name(""));
    }

    #[test]
    fn pixel_rgba_must_be_eight_hex_digits() {
        assert_eq!(parse_pixel_rgba("0xff00ff00"), Some(0xff00_ff00));
        assert_eq!(parse_pixel_rgba("0XFF00FF00"), Some(0xff00_ff00));
        assert_eq!(parse_pixel_rgba("ff00ff00"), None);
        assert_eq!(parse_pixel_rgba("0xff00ff0"), None);
        assert_eq!(parse_pixel_rgba("0xzzzzzzzz"), None);
    }

    #[test]
    fn run_ids_must_be_safe_directory_names() {
        assert!(safe_run_id("20261128T120000Z-1234"));
        assert!(!safe_run_id("../96"));
        assert!(!safe_run_id("a/b"));
        assert!(!safe_run_id(""));
    }

    #[test]
    fn last_run_slices_after_the_final_header() {
        let text = "=== run 1 x ===\na\n=== run 2 y ===\nb\n";
        assert_eq!(last_run(text), "=== run 2 y ===\nb\n");
        assert_eq!(last_run("a\nb\n"), "a\nb\n");
    }
}
