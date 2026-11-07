//! `nva-verify loadplan` — read an import pack's load plan (#42).
//!
//! This ticket implements the positive read path: parse the manifest and the
//! load plan, accept schema version 1, and return the parsed plan. Overlap,
//! page alignment and entry-in-`rx` checks belong to the L3 loader and the
//! negative-path ticket (#43); this module does not do that business
//! validation.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};

pub const MANIFEST_VERSION: i64 = 1;
pub const LOAD_PLAN_VERSION: i64 = 1;

/// `0` pass / `2` mismatch (`docs/ARCHITECTURE.md` §3.8).
pub const EXIT_MISMATCH: i32 = 2;
/// `4` fixture or hash is stale (`docs/ARCHITECTURE.md` §3.8).
pub const EXIT_STALE_FIXTURE: i32 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub module: String,
    pub name: String,
    pub vaddr: i64,
    pub size: i64,
    pub perm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadPlan {
    pub entry: i64,
    pub segments: Vec<Segment>,
}

#[derive(Debug)]
pub enum Error {
    /// The fixture directory or a required file does not exist.
    Missing(PathBuf),
    /// The file exists but cannot be read.
    CannotRead(PathBuf, String),
    /// The file is not JSON.
    Malformed(PathBuf, String),
    /// The JSON is valid but not schema version 1.
    Schema(String),
}

impl Error {
    /// Exit code for the CLI. The mapping lives here so `main.rs` cannot drift
    /// from the documented `0 / 2 / 4` semantics.
    pub fn exit_code(&self) -> i32 {
        match self {
            Error::Missing(_) | Error::CannotRead(..) => EXIT_STALE_FIXTURE,
            Error::Malformed(..) | Error::Schema(_) => EXIT_MISMATCH,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Missing(path) => write!(f, "missing {}", path.display()),
            Error::CannotRead(path, why) => write!(f, "cannot read {}: {why}", path.display()),
            Error::Malformed(path, why) => write!(f, "invalid JSON in {}: {why}", path.display()),
            Error::Schema(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Error {}

/// Read `manifest.json` and `load-plan.json` from one fixture directory.
pub fn load(fixture: &Path) -> Result<LoadPlan, Error> {
    let manifest = read_json(&fixture.join("manifest.json"))?;
    let plan = read_json(&fixture.join("load-plan.json"))?;
    check_manifest(&manifest)?;
    parse_plan(&plan)
}

/// The one-line JSON summary printed by the `loadplan` subcommand.
pub fn summary_line(fixture: &str, plan: &LoadPlan) -> String {
    format!(
        "{{\"entry\":{},\"fixture\":{},\"schema\":{},\"segments\":{}}}",
        plan.entry,
        json::quote(fixture),
        LOAD_PLAN_VERSION,
        plan.segments.len()
    )
}

fn read_json(path: &Path) -> Result<Value, Error> {
    let bytes = std::fs::read(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => Error::Missing(path.to_path_buf()),
        _ => Error::CannotRead(path.to_path_buf(), error.to_string()),
    })?;
    // A file that exists but is not UTF-8 is malformed JSON, not a stale
    // fixture: exit 2, not 4.
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Malformed(path.to_path_buf(), "invalid UTF-8".into()))?;
    json::parse(&text).map_err(|why| Error::Malformed(path.to_path_buf(), why))
}

fn check_manifest(manifest: &Value) -> Result<(), Error> {
    match manifest.get("version").and_then(Value::as_i64) {
        Some(MANIFEST_VERSION) => Ok(()),
        Some(other) => Err(Error::Schema(format!(
            "manifest version {other} is not supported"
        ))),
        None => Err(Error::Schema(
            "manifest is missing integer 'version'".into(),
        )),
    }
}

fn parse_plan(plan: &Value) -> Result<LoadPlan, Error> {
    let version = plan
        .get("version")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Schema("load plan is missing integer 'version'".into()))?;
    if version != LOAD_PLAN_VERSION {
        return Err(Error::Schema(format!(
            "load plan version {version} is not supported"
        )));
    }
    let entry = plan
        .get("entry")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Schema("load plan is missing integer 'entry'".into()))?;
    let raw_segments = plan
        .get("segments")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Schema("load plan is missing array 'segments'".into()))?;

    let mut segments = Vec::with_capacity(raw_segments.len());
    for (index, raw) in raw_segments.iter().enumerate() {
        let module = string_field(raw, index, "module")?;
        let name = string_field(raw, index, "name")?;
        let perm = string_field(raw, index, "perm")?;
        if !matches!(perm.as_str(), "r" | "rw" | "rx") {
            return Err(Error::Schema(format!(
                "segment {index} has invalid 'perm' {perm:?}"
            )));
        }
        let vaddr = integer_field(raw, index, "vaddr")?;
        let size = integer_field(raw, index, "size")?;
        segments.push(Segment {
            module,
            name,
            vaddr,
            size,
            perm,
        });
    }
    Ok(LoadPlan { entry, segments })
}

fn string_field(value: &Value, index: usize, key: &str) -> Result<String, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error::Schema(format!("segment {index} is missing string '{key}'")))
}

fn integer_field(value: &Value, index: usize, key: &str) -> Result<i64, Error> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::Schema(format!("segment {index} is missing integer '{key}'")))
}
