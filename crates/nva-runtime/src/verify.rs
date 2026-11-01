//! Verifier-only probe for the PC smoke path (#44).
//!
//! The product header has no entry point that returns the physical device name,
//! the validation counters or the last read-back pixel, but `nva-verify smoke`
//! has to put all four JSON keys somewhere. This module is that place: a single
//! `extern "C"` export, `nva_gfx_verify_probe`, that the verifier calls after
//! `nva_gfx_wait_fence`.
//!
//! It is **not** in `include/nva_abi.h` on purpose (#37 D4: the header is this
//! ticket's input, not its output). When an ABI append ticket adopts it, this
//! type moves next to the other C structs and gains the usual `NVA_*_VERSION`
//! macro; until then the verifier and the runtime share this Rust definition.

use core::ffi::c_char;

/// State the smoke path needs that the frozen ABI does not expose.
#[repr(C)]
pub struct nva_verify_state {
    pub size: u32,
    pub version: u32,
    /// Last centre pixel, `R8G8B8A8_UNORM` bytes read as a little-endian u32.
    pub pixel_rgba: u32,
    pub validation_available: u32,
    pub validation_errors: u32,
    /// `VkPhysicalDeviceProperties.deviceName`, NUL-terminated.
    pub device_name: [c_char; 128],
}

pub const NVA_VERIFY_STATE_VERSION: u32 = 1;

impl nva_verify_state {
    /// Zeroed value with `size`/`version` filled in.
    pub fn init() -> Self {
        Self {
            size: core::mem::size_of::<Self>() as u32,
            version: NVA_VERIFY_STATE_VERSION,
            pixel_rgba: 0,
            validation_available: 0,
            validation_errors: 0,
            device_name: [0; 128],
        }
    }

    pub fn device_name_string(&self) -> String {
        let bytes: Vec<u8> = self
            .device_name
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
