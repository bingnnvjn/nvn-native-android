//! `nva-verify smoke` — one offscreen frame through the product C ABI (#44).
//!
//! The contract (`#37` D7–D11) is narrow on purpose:
//! - create the gfx object, draw one frame (red clear + one triangle), wait for
//!   the fence and read the centre pixel **only through exported C symbols**;
//! - print four JSON keys: `device_name`, `pixel_rgba`, `validation_available`,
//!   `validation_errors`;
//! - exit `0` pass, `2` mismatch, `3` oracle unavailable (no Vulkan loader, no
//!   validation layer). "Validation unavailable" is always `3`, never a green
//!   run with a missing oracle.

use core::ffi::{c_char, c_void};
use core::mem::size_of;
use core::ptr;
use std::ffi::{CStr, CString};

use nva_abi::{
    nva_fence, nva_gfx_desc, nva_host_services, nva_mem_region, nva_stats,
    NVA_ERR_UNSUPPORTED, NVA_GFX_DESC_VERSION, NVA_HOST_SERVICES_VERSION, NVA_OK, NVA_STATS_VERSION,
};
use nva_runtime::verify::nva_verify_state;

/// `(R,G,B,A) = (0, 255, 0, 255)` read as a little-endian u32 (`#37` D10).
pub const PIXEL_GREEN: u32 = 0xff00ff00;

/// The lavapipe device name always contains this substring (`#30`).
pub const DEVICE_HINT: &str = "llvmpipe";

/// Exit code for one smoke result (`#37` D8).
///
/// Oracle availability is checked first: a missing layer must be `3` even when
/// the pixels also differ, otherwise a missing oracle could hide behind a
/// mismatch.
pub fn verdict(
    device_name: &str,
    pixel_rgba: u32,
    validation_available: bool,
    validation_errors: u32,
) -> i32 {
    if !validation_available {
        return 3;
    }
    if !device_name.contains(DEVICE_HINT) {
        return 2;
    }
    if pixel_rgba != PIXEL_GREEN {
        return 2;
    }
    if validation_errors != 0 {
        return 2;
    }
    0
}

/// The four-key JSON line. No `clean` anywhere: `#37` D8 / `#31`.
pub fn smoke_json(
    device_name: &str,
    pixel_rgba: u32,
    validation_available: bool,
    validation_errors: u32,
) -> String {
    format!(
        "{{\"device_name\":\"{}\",\"pixel_rgba\":\"0x{pixel_rgba:08x}\",\
         \"validation_available\":{},\"validation_errors\":{validation_errors}}}",
        json_escape(device_name),
        u8::from(validation_available),
    )
}

fn json_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                escaped.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => escaped.push(other),
        }
    }
    escaped
}

/// Exit codes from `#37` D8.
pub const EXIT_OK: i32 = 0;
pub const EXIT_MISMATCH: i32 = 2;
pub const EXIT_ORACLE: i32 = 3;

/// What a smoke run writes and returns.
pub struct Outcome {
    pub exit_code: i32,
    pub json: String,
    pub detail: String,
}

pub struct Options {
    pub frames: u32,
}

/// Run one headless smoke: build gfx, draw `frames` frames, wait the fence and
/// read the centre pixel. Every runtime call below is an exported C symbol.
pub fn run(options: &Options) -> Outcome {
    if options.frames == 0 {
        return failure(EXIT_MISMATCH, "smoke: --frames must be at least 1");
    }

    let host = nva_host_services {
        size: size_of::<nva_host_services>() as u32,
        version: NVA_HOST_SERVICES_VERSION,
        user: ptr::null_mut(),
        log: Some(host_log),
        mem_query: Some(host_mem_query),
        on_device_lost: Some(host_device_lost),
        fatal: Some(host_fatal),
    };
    let desc = nva_gfx_desc {
        size: size_of::<nva_gfx_desc>() as u32,
        version: NVA_GFX_DESC_VERSION,
        flags: 0,
        reserved: 0,
        host: &host,
        shader_cache: ptr::null(),
    };

    let mut gfx: *mut c_void = ptr::null_mut();
    let status = unsafe { nva_gfx_create(&desc, &mut gfx) };
    if status != NVA_OK {
        let exit_code = if status == NVA_ERR_UNSUPPORTED {
            EXIT_ORACLE
        } else {
            EXIT_MISMATCH
        };
        return failure(
            exit_code,
            &format!("smoke: nva_gfx_create returned {status} (no Vulkan oracle)"),
        );
    }
    let entry = |name: &str| -> u64 {
        let c_name = CString::new(name).expect("NVN entry point name has no NUL");
        unsafe { nva_nvn_bootstrap(gfx, c_name.as_ptr()) }
    };
    let set_defaults = entry("nvnDeviceBuilderSetDefaults");
    let device_initialize = entry("nvnDeviceInitialize");
    let clear_color = entry("nvnCommandBufferClearColor");
    let draw_arrays = entry("nvnCommandBufferDrawArrays");
    let submit = entry("nvnQueueSubmitCommands");
    let finish = entry("nvnQueueFinish");
    if [
        set_defaults,
        device_initialize,
        clear_color,
        draw_arrays,
        submit,
        finish,
    ]
    .iter()
    .any(|address| *address == 0)
    {
        unsafe { nva_gfx_destroy(gfx) };
        return failure(EXIT_MISMATCH, "smoke: an NVN entry point did not resolve");
    }

    // The NVN object model is opaque at this seam: the objects live in these
    // buffers and only the runtime interprets them.
    let mut builder = [0u64; 8];
    let mut device = [0u64; 8];
    let mut command_buffer = [0u64; 8];
    let mut queue = [0u64; 8];
    let red = [1.0f32, 0.0, 0.0, 1.0];

    let mut detail = String::new();
    let mut expected_serial = 0u32;
    for frame in 0..options.frames {
        unsafe {
            let set_defaults: unsafe extern "C" fn(*mut c_void) = function(set_defaults);
            let device_initialize: unsafe extern "C" fn(*mut c_void, *const c_void) -> u8 =
                function(device_initialize);
            let clear_color: unsafe extern "C" fn(*mut c_void, i32, *const f32, u32) =
                function(clear_color);
            let draw_arrays: unsafe extern "C" fn(*mut c_void, i32, i32, i32) =
                function(draw_arrays);
            let submit: unsafe extern "C" fn(*mut c_void, i32, *const *mut c_void) =
                function(submit);
            let finish: unsafe extern "C" fn(*mut c_void) = function(finish);

            let builder_ptr = builder.as_mut_ptr().cast::<c_void>();
            let device_ptr = device.as_mut_ptr().cast::<c_void>();
            let command_buffer_ptr = command_buffer.as_mut_ptr().cast::<c_void>();
            let queue_ptr = queue.as_mut_ptr().cast::<c_void>();

            set_defaults(builder_ptr);
            if device_initialize(device_ptr, builder_ptr as *const c_void) == 0 {
                nva_gfx_destroy(gfx);
                return failure(EXIT_MISMATCH, "smoke: nvnDeviceInitialize returned 0");
            }
            // NVN_DRAW_TRIANGLES is 0 in the NVN enum.
            clear_color(command_buffer_ptr, 0, red.as_ptr(), 0xf);
            draw_arrays(command_buffer_ptr, 0, 0, 3);
            let handles = [command_buffer_ptr];
            submit(queue_ptr, 1, handles.as_ptr());
            finish(queue_ptr);

            let mut stats = nva_stats {
                size: size_of::<nva_stats>() as u32,
                version: NVA_STATS_VERSION,
                unsupported_calls: 0,
                unknown_resolves: 0,
                stub_aborts: 0,
                sticky_error: 0,
                last_serial: 0,
                flags: 0,
                frames: 0,
                draw_calls: 0,
                submits: 0,
            };
            let status = nva_gfx_get_stats(gfx, &mut stats);
            if status != NVA_OK {
                nva_gfx_destroy(gfx);
                return failure(
                    EXIT_MISMATCH,
                    &format!("smoke: nva_gfx_get_stats returned {status}"),
                );
            }
            if stats.last_serial <= expected_serial {
                // `nvnQueueSubmitCommands` reports its own failure on stderr;
                // a serial that did not advance is how the verifier notices the
                // frame was never submitted.
                nva_gfx_destroy(gfx);
                return failure(
                    EXIT_MISMATCH,
                    &format!(
                        "smoke: frame {} was not submitted (last_serial={})",
                        frame + 1,
                        stats.last_serial
                    ),
                );
            }
            expected_serial = stats.last_serial;
            let fence = nva_fence {
                serial: u64::from(stats.last_serial),
                domain: 0,
                gen: 0,
            };
            let status = nva_gfx_wait_fence(gfx, &fence, 30_000_000_000);
            if status != NVA_OK {
                nva_gfx_destroy(gfx);
                return failure(
                    EXIT_MISMATCH,
                    &format!("smoke: nva_gfx_wait_fence returned {status}"),
                );
            }
            detail = format!(
                "smoke: frame {}/{} submitted, last_serial={}",
                frame + 1,
                options.frames,
                stats.last_serial
            );
        }
    }

    let mut probe = nva_verify_state::init();
    let status = unsafe { nva_gfx_verify_probe(gfx, &mut probe) };
    if status != NVA_OK {
        unsafe { nva_gfx_destroy(gfx) };
        return failure(
            EXIT_MISMATCH,
            &format!("smoke: nva_gfx_verify_probe returned {status}"),
        );
    }

    let device_name = probe.device_name_string();
    let pixel_rgba = probe.pixel_rgba;
    let validation_available = probe.validation_available != 0;
    let validation_errors = probe.validation_errors;
    let exit_code = verdict(
        &device_name,
        pixel_rgba,
        validation_available,
        validation_errors,
    );
    let json = smoke_json(
        &device_name,
        pixel_rgba,
        validation_available,
        validation_errors,
    );

    unsafe { nva_gfx_destroy(gfx) };

    let detail = format!(
        "{detail}; device=\"{device_name}\" pixel=0x{pixel_rgba:08x} \
         validation_available={} validation_errors={validation_errors}",
        u8::from(validation_available)
    );
    Outcome {
        exit_code,
        json,
        detail,
    }
}

fn failure(exit_code: i32, detail: &str) -> Outcome {
    Outcome {
        exit_code,
        // The four keys stay present even when the oracle is missing; the
        // values are the honest "we saw nothing" ones.
        json: smoke_json("", 0, false, 0),
        detail: detail.to_string(),
    }
}

unsafe fn function<F>(address: u64) -> F {
    debug_assert_eq!(size_of::<F>(), size_of::<u64>());
    std::mem::transmute_copy(&address)
}

fn c_string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(pointer).to_string_lossy().into_owned() }
}

unsafe extern "C" fn host_log(
    _user: *mut c_void,
    level: u32,
    tag: *const c_char,
    msg: *const c_char,
) {
    eprintln!(
        "[nva-runtime log {level}] {}: {}",
        c_string(tag),
        c_string(msg)
    );
}

unsafe extern "C" fn host_mem_query(
    _user: *mut c_void,
    _va: u64,
    _size: u64,
    _out: *mut nva_mem_region,
    _out_off: *mut u64,
    _out_covered: *mut u64,
) -> u32 {
    NVA_ERR_UNSUPPORTED
}

unsafe extern "C" fn host_device_lost(_user: *mut c_void, reason: u32, serial: u64) {
    eprintln!("[nva-runtime] device lost: reason={reason} serial={serial}");
}

unsafe extern "C" fn host_fatal(_user: *mut c_void, reason: u32, slot: u32, name: *const c_char) {
    eprintln!(
        "[nva-runtime] fatal: reason={reason} slot={slot} name={}",
        c_string(name)
    );
}

extern "C" {
    fn nva_gfx_create(desc: *const nva_gfx_desc, out: *mut *mut c_void) -> u32;
    fn nva_gfx_destroy(gfx: *mut c_void);
    fn nva_gfx_get_stats(gfx: *mut c_void, out: *mut nva_stats) -> u32;
    fn nva_gfx_wait_fence(gfx: *mut c_void, fence: *const nva_fence, timeout_ns: u64) -> u32;
    fn nva_gfx_verify_probe(gfx: *mut c_void, out: *mut nva_verify_state) -> u32;
    fn nva_nvn_bootstrap(gfx: *mut c_void, name: *const c_char) -> u64;
}

#[cfg(test)]
mod tests {
    use super::{smoke_json, verdict, PIXEL_GREEN};

    const LAVAPIPE: &str = "llvmpipe (LLVM 20.1.2, 256 bits)";

    #[test]
    fn green_frame_on_lavapipe_passes() {
        assert_eq!(verdict(LAVAPIPE, PIXEL_GREEN, true, 0), 0);
    }

    #[test]
    fn missing_validation_layer_exits_3() {
        assert_eq!(verdict(LAVAPIPE, PIXEL_GREEN, false, 0), 3);
    }

    #[test]
    fn oracle_unavailable_beats_pixel_mismatch() {
        assert_eq!(verdict(LAVAPIPE, 0, false, 0), 3);
    }

    #[test]
    fn wrong_device_exits_2() {
        assert_eq!(verdict("Adreno (TM) 725", PIXEL_GREEN, true, 0), 2);
    }

    #[test]
    fn wrong_pixel_exits_2() {
        assert_eq!(verdict(LAVAPIPE, 0xff0000ff, true, 0), 2);
    }

    #[test]
    fn validation_errors_exit_2() {
        assert_eq!(verdict(LAVAPIPE, PIXEL_GREEN, true, 2), 2);
    }

    #[test]
    fn json_has_the_four_keys_and_no_clean_word() {
        let json = smoke_json(LAVAPIPE, PIXEL_GREEN, true, 0);
        assert!(
            json.contains(r#""device_name":"llvmpipe (LLVM 20.1.2, 256 bits)""#),
            "{json}"
        );
        assert!(json.contains(r#""pixel_rgba":"0xff00ff00""#), "{json}");
        assert!(json.contains(r#""validation_available":1"#), "{json}");
        assert!(json.contains(r#""validation_errors":0"#), "{json}");
        assert!(!json.contains("clean"), "{json}");
    }

    #[test]
    fn json_escapes_the_device_name() {
        let json = smoke_json("a\"b\\c", PIXEL_GREEN, true, 0);
        assert!(json.contains(r#""device_name":"a\"b\\c""#), "{json}");
    }
}
