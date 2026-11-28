//! Reader for the `nva-stats` lines the device shell writes (#94).
//!
//! The contract is `docs/DEVICE-LOGS.md` §9: one line per snapshot, tag
//! `nva-stats`, a single JSON object at the end of the line. This module
//! parses those objects strictly — a missing field, a wrong type or an
//! out-of-order sequence is an error, because "silently swallowed" is exactly
//! what the contract forbids. Lines that are not snapshots (the shell's own
//! events, a `detached_submit` signal) are skipped instead.

use crate::json::{self, Value};

/// One parsed `nva-stats` snapshot.
///
/// The field order here matches the JSON contract; the values are the ones
/// `docs/DEVICE-LOGS.md` §9 documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snapshot {
    /// Frames that reached the display.
    pub presents: u32,
    /// The generation of the last successful attach, sticky across detach.
    pub surface_generation: u32,
    /// Whether a window is attached at this moment.
    pub attached: bool,
    /// The same sticky generation, read beside `attached`.
    pub generation: u32,
    /// Queue submits the render loop made.
    pub submits: u64,
    /// Whether validation was available; constant 0 in the product package.
    pub validation_available: u32,
    /// Validation errors seen; constant 0 in the product package.
    pub validation_errors: u32,
}

/// Why a line that looked like a snapshot was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// A required field is absent.
    MissingField(&'static str),
    /// A required field has the wrong JSON type or does not fit its Rust type.
    WrongType(&'static str),
    /// The text from the first `{` to the end of the line is not a JSON object.
    NotJson(String),
    /// `presents` or `generation` went backwards in a stream.
    OutOfOrder(&'static str),
}

/// Parses one log line.
///
/// `Ok(None)` means the line is not a snapshot — it has no `{`, or it is the
/// `detached_submit` signal — and the caller should skip it. A line whose
/// object lacks a field, carries a wrong type, or is not JSON is an error:
/// the contract says the reader must not swallow it.
pub fn parse_line(line: &str) -> Result<Option<Snapshot>, ParseError> {
    let Some(start) = line.find('{') else {
        return Ok(None);
    };
    let value = json::parse(&line[start..]).map_err(ParseError::NotJson)?;
    if value.get("event").and_then(Value::as_str) == Some("detached_submit") {
        return Ok(None);
    }
    Ok(Some(Snapshot {
        presents: u32_field(&value, "presents")?,
        surface_generation: u32_field(&value, "surface_generation")?,
        attached: bool_field(&value, "attached")?,
        generation: u32_field(&value, "generation")?,
        submits: u64_field(&value, "submits")?,
        validation_available: u32_field(&value, "validation_available")?,
        validation_errors: u32_field(&value, "validation_errors")?,
    }))
}

/// Parses a whole captured log, in order.
///
/// Non-snapshot lines are skipped. A sequence whose `presents` or `generation`
/// goes backwards is refused with [`ParseError::OutOfOrder`]: two runs
/// interleaved, or a rotated file read out of order, must not look like a
/// counter that fell.
pub fn parse_stream(text: &str) -> Result<Vec<Snapshot>, ParseError> {
    let mut snapshots: Vec<Snapshot> = Vec::new();
    for line in text.lines() {
        let Some(snapshot) = parse_line(line)? else {
            continue;
        };
        if let Some(previous) = snapshots.last() {
            if snapshot.presents < previous.presents {
                return Err(ParseError::OutOfOrder("presents"));
            }
            if snapshot.generation < previous.generation {
                return Err(ParseError::OutOfOrder("generation"));
            }
        }
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

fn field<'a>(value: &'a Value, name: &'static str) -> Result<&'a Value, ParseError> {
    value.get(name).ok_or(ParseError::MissingField(name))
}

fn number(value: &Value, name: &'static str) -> Result<i64, ParseError> {
    field(value, name)?
        .as_i64()
        .ok_or(ParseError::WrongType(name))
}

fn u32_field(value: &Value, name: &'static str) -> Result<u32, ParseError> {
    u32::try_from(number(value, name)?).map_err(|_| ParseError::WrongType(name))
}

fn u64_field(value: &Value, name: &'static str) -> Result<u64, ParseError> {
    u64::try_from(number(value, name)?).map_err(|_| ParseError::WrongType(name))
}

fn bool_field(value: &Value, name: &'static str) -> Result<bool, ParseError> {
    match field(value, name)? {
        Value::Bool(flag) => Ok(*flag),
        _ => Err(ParseError::WrongType(name)),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_line, parse_stream, ParseError, Snapshot};

    /// One well-formed line as `logcat -d -s nva-stats` prints it.
    const VALID: &str = "13:40:12.418 [rt] info nva-stats: \
        {\"presents\":1,\"surface_generation\":2,\"attached\":true,\"generation\":2,\
        \"submits\":3,\"validation_available\":0,\"validation_errors\":0}";

    /// The well-formed line with `surface_generation` removed.
    const MISSING_SURFACE_GENERATION: &str =
        "13:40:12.418 [rt] info nva-stats: {\"presents\":1,\"attached\":true,\
        \"generation\":2,\"submits\":3,\"validation_available\":0,\"validation_errors\":0}";

    /// A runtime event line: no JSON object at all.
    const NON_JSON: &str = "13:40:12.408 [rt] info nva-runtime: start";

    /// Two snapshots whose `presents` goes backwards (5, then 3).
    const OUT_OF_ORDER: &str =
        "13:40:12.418 [rt] info nva-stats: {\"presents\":5,\"surface_generation\":2,\
        \"attached\":true,\"generation\":2,\"submits\":3,\"validation_available\":0,\
        \"validation_errors\":0}\n\
        13:40:12.419 [rt] info nva-stats: {\"presents\":3,\"surface_generation\":2,\
        \"attached\":true,\"generation\":2,\"submits\":4,\"validation_available\":0,\
        \"validation_errors\":0}";

    #[test]
    fn statslog_valid_snapshot_parses() {
        let snapshot = parse_line(VALID)
            .expect("a well-formed snapshot must parse")
            .expect("the line is a snapshot, not one to skip");
        assert_eq!(
            snapshot,
            Snapshot {
                presents: 1,
                surface_generation: 2,
                attached: true,
                generation: 2,
                submits: 3,
                validation_available: 0,
                validation_errors: 0,
            }
        );
    }

    #[test]
    fn statslog_missing_field_is_an_error() {
        assert_eq!(
            parse_line(MISSING_SURFACE_GENERATION),
            Err(ParseError::MissingField("surface_generation")),
            "a missing field must fail the parse, not take a default"
        );
    }

    #[test]
    fn statslog_out_of_order_is_an_error() {
        assert_eq!(
            parse_stream(OUT_OF_ORDER),
            Err(ParseError::OutOfOrder("presents")),
            "a stream whose counters go backwards is not a run"
        );
    }

    #[test]
    fn statslog_non_json_line_is_skipped() {
        assert_eq!(
            parse_line(NON_JSON),
            Ok(None),
            "a line with no JSON object is not a snapshot"
        );
        assert_eq!(
            parse_stream(NON_JSON),
            Ok(Vec::new()),
            "a stream of non-snapshot lines yields no snapshots"
        );
    }
}
