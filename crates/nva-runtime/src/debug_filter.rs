//! Which `VK_EXT_debug_utils` messages count as validation errors (#44 / #31).
//!
//! The contract has two halves:
//! - only `ERROR` severity messages of type `VALIDATION` count (docs/ARCHITECTURE
//!   §3.6, #37 D11);
//! - the driver's own `VKDBGUTIL*` messages are driver noise, not layer
//!   findings, and never count. On the Adreno 725 (#31) those arrive as
//!   `ERROR`/`VALIDATION` with an empty `messageIdName`, so an empty id is
//!   treated as driver noise too.
//!
//! The decision is a pure function so the table can be pinned without a GPU.

/// Returns true when a debug-utils message must be counted in
/// `validation_errors`.
pub fn counts_as_validation_error(
    is_error: bool,
    is_validation: bool,
    message_id: &str,
    message: &str,
) -> bool {
    is_error && is_validation && !is_driver_message(message_id, message)
}

/// True for messages the driver emits itself through debug utils.
fn is_driver_message(message_id: &str, message: &str) -> bool {
    message_id.is_empty() || message_id.starts_with("VKDBGUTIL") || message.contains("VKDBGUTIL")
}

#[cfg(test)]
mod tests {
    use super::counts_as_validation_error;

    // Table from #44 §4 + #31: driver messages that arrive as ERROR/VALIDATION
    // with an empty messageIdName, or carry the VKDBGUTIL id, never count.
    #[test]
    fn empty_message_id_driver_message_does_not_count() {
        // Exact shape observed on the Adreno 725 (#31, RESULTS.md).
        assert!(!counts_as_validation_error(
            true,
            true,
            "",
            "VKDBGUTILWARN003: some driver hint",
        ));
    }

    #[test]
    fn vkdbgutil_id_does_not_count() {
        assert!(!counts_as_validation_error(
            true,
            true,
            "VKDBGUTILERR001",
            "driver hint",
        ));
    }

    #[test]
    fn vkdbgutil_in_text_does_not_count() {
        assert!(!counts_as_validation_error(
            true,
            true,
            "SomeId",
            "VKDBGUTILWARN003: driver hint",
        ));
    }

    #[test]
    fn layer_vuid_counts() {
        assert!(counts_as_validation_error(
            true,
            true,
            "VUID-vkCmdDraw-None-07831",
            "validation layer found a problem",
        ));
    }

    #[test]
    fn warnings_and_non_validation_types_do_not_count() {
        assert!(!counts_as_validation_error(
            false,
            true,
            "VUID-vkCmdDraw-None-07831",
            "warning",
        ));
        assert!(!counts_as_validation_error(
            true,
            false,
            "VUID-vkCmdDraw-None-07831",
            "performance",
        ));
    }
}
