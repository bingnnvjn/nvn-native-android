//! The `nva-stats` parser contract through the public API (#94).
//!
//! The unit tests in `src/statslog.rs` pin the four ticket fixtures. These add
//! the shapes the contract names but does not pin there: malformed JSON that
//! does contain a `{`, wrong field types, and the `detached_submit` signal.

use nva_verify::statslog::{parse_line, parse_stream, ParseError};

/// A complete, well-formed snapshot. Tests replace one value at a time.
const VALID: &str = "13:40:12.418 [rt] info nva-stats: {\"presents\":1,\
    \"surface_generation\":2,\"attached\":true,\"generation\":2,\"submits\":3,\
    \"validation_available\":0,\"validation_errors\":0}";

#[test]
fn statslog_contract_bad_json_after_brace_is_not_json() {
    for line in [
        "13:40:12.418 [rt] info nva-stats: {\"presents\":1",
        "13:40:12.418 [rt] info nva-stats: {not json",
    ] {
        let parsed = parse_line(line);
        assert!(
            matches!(parsed, Err(ParseError::NotJson(_))),
            "a line with `{{` but no JSON object must be an error: {parsed:?}"
        );
    }
}

#[test]
fn statslog_contract_wrong_type_is_an_error() {
    assert_eq!(
        parse_line(&VALID.replace("\"attached\":true", "\"attached\":1")),
        Err(ParseError::WrongType("attached")),
        "a number where the contract says bool"
    );
    assert_eq!(
        parse_line(&VALID.replace("\"presents\":1", "\"presents\":\"1\"")),
        Err(ParseError::WrongType("presents")),
        "a string where the contract says integer"
    );
}

#[test]
fn statslog_contract_signal_is_skipped() {
    let signal = "13:40:12.418 [rt] info nva-stats: \
        {\"event\":\"detached_submit\",\"count\":1,\"generation\":2}";
    assert_eq!(parse_line(signal), Ok(None), "the signal is not a snapshot");
    let stream = format!("{VALID}\n{signal}\n{VALID}");
    assert_eq!(
        parse_stream(&stream).map(|snapshots| snapshots.len()),
        Ok(2),
        "the signal is skipped and does not join the snapshot sequence"
    );
}
