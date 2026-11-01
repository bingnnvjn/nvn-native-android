//! `nva-runtime` — the runtime behind the platform shell.
//!
//! The product C ABI export surface (`nva_app_` / `nva_gfx_` / `nva_host_`)
//! lives here. Ticket #44 adds exactly what the PC smoke path needs: create a
//! gfx object, drive one offscreen frame through the NVN entry points, wait for
//! the fence, and hand the result to `nva-verify`.
//!
//! Everything across the crate boundary is `extern "C"` (docs/ARCHITECTURE.md
//! §7.1). One verifier-only export, `nva_gfx_verify_probe`, is deliberately not
//! in `include/nva_abi.h`: the header is this ticket's input, not its output
//! (#37 D4), so the probe waits for a separate ABI append.

use core::ffi::{c_char, c_void, CStr};
use core::mem::size_of;
use core::ptr;

use nva_abi::{
    nva_app_desc, nva_gfx_caps, nva_gfx_desc, nva_host_desc, nva_nvn_table, nva_stats,
    NVA_APP_DESC_VERSION, NVA_ERR_DEVICE_LOST, NVA_ERR_INVALID_ARGUMENT, NVA_ERR_TIMEOUT,
    NVA_ERR_UNSUPPORTED, NVA_GFX_CAPS_VERSION, NVA_GFX_CAP_VULKAN_11, NVA_GFX_DESC_VERSION, NVA_OK,
    NVA_STATS_VERSION,
};

pub mod debug_filter;
mod vk;
pub mod verify;

use verify::nva_verify_state;
use vk::{CreateError, Vk, WaitError};

/// The one gfx object this ticket can create.
///
/// The NVN entry points have no gfx argument (real NVN hangs the device off the
/// object graph), so the current object is process-global. One process, one
/// object: the smoke verifier creates it, draws one frame, destroys it.
static CURRENT_GFX: std::sync::atomic::AtomicPtr<Gfx> =
    std::sync::atomic::AtomicPtr::new(ptr::null_mut());

/// Frame state recorded by the NVN command-buffer calls and consumed by
/// `nvnQueueSubmitCommands`.
struct PendingFrame {
    clear_rgba: [f32; 4],
    draw_vertices: u32,
}

static PENDING: std::sync::Mutex<PendingFrame> = std::sync::Mutex::new(PendingFrame {
    clear_rgba: [0.0; 4],
    draw_vertices: 0,
});

/// The engine object behind the opaque `nva_gfx *`.
pub struct Gfx {
    vk: Vk,
    frames: u64,
    draw_calls: u64,
    submits: u64,
    last_serial: u32,
    pending: bool,
}

impl Gfx {
    fn new(vk: Vk) -> Self {
        Self {
            vk,
            frames: 0,
            draw_calls: 0,
            submits: 0,
            last_serial: 0,
            pending: false,
        }
    }
}

// ---------------------------------------------------------------- gfx

/// Create the offscreen gfx object (`nva_gfx_create`).
///
/// # Safety
/// `desc` must point to a readable `nva_gfx_desc` and `out` to a writable
/// pointer slot.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_create(
    desc: *const nva_gfx_desc,
    out: *mut *mut Gfx,
) -> nva_abi::nva_status {
    if desc.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let desc = &*desc;
    if desc.size < size_of::<nva_gfx_desc>() as u32
        || desc.version != NVA_GFX_DESC_VERSION
        || desc.host.is_null()
    {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    *out = ptr::null_mut();
    // One process, one gfx object: the NVN entry points resolve through the
    // process-global `CURRENT_GFX`, so a second object would silently steal the
    // slot from the first.
    if !CURRENT_GFX
        .load(std::sync::atomic::Ordering::Acquire)
        .is_null()
    {
        return NVA_ERR_INVALID_ARGUMENT;
    }

    match Vk::new() {
        Ok(vk) => {
            let gfx = Box::new(Gfx::new(vk));
            let raw = Box::into_raw(gfx);
            *out = raw;
            CURRENT_GFX.store(raw, std::sync::atomic::Ordering::Release);
            NVA_OK
        }
        Err(CreateError::OracleUnavailable(reason)) => {
            eprintln!("[nva-runtime] gfx oracle unavailable: {reason}");
            NVA_ERR_UNSUPPORTED
        }
        Err(CreateError::Failed(reason)) => {
            eprintln!("[nva-runtime] gfx create failed: {reason}");
            NVA_ERR_DEVICE_LOST
        }
    }
}

/// # Safety
/// `gfx` must be a pointer returned by `nva_gfx_create`, or NULL.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_destroy(gfx: *mut Gfx) {
    if gfx.is_null() {
        return;
    }
    let _ = CURRENT_GFX.compare_exchange(
        gfx,
        ptr::null_mut(),
        std::sync::atomic::Ordering::AcqRel,
        std::sync::atomic::Ordering::Acquire,
    );
    drop(Box::from_raw(gfx));
}

/// # Safety
/// `gfx` must be valid and `out` must point to a writable `nva_gfx_caps`.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_query_caps(
    gfx: *mut Gfx,
    out: *mut nva_gfx_caps,
) -> nva_abi::nva_status {
    if gfx.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let out = &mut *out;
    if out.size < size_of::<nva_gfx_caps>() as u32 || out.version != NVA_GFX_CAPS_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let gfx = &*gfx;
    let props = gfx.vk.limits();
    out.max_color_attachments = props.limits.max_color_attachments;
    out.max_viewports = props.limits.max_viewports;
    out.max_push_constants = props.limits.max_push_constants_size;
    out.max_msaa_samples = max_sample_count(props.limits.framebuffer_color_sample_counts.as_raw());
    out.flags = if props.api_version >= ash::vk::make_api_version(0, 1, 1, 0) {
        NVA_GFX_CAP_VULKAN_11
    } else {
        0
    };
    out.reserved = 0;
    // Vulkan 1.1 properties that need a pNext chain are not queried here; the
    // smoke path does not consume them.
    out.max_memory_allocation_size = 0;
    out.max_memory_allocation_count = 0;
    out.reserved2 = 0;
    out.min_imported_host_pointer_alignment = 0;
    NVA_OK
}

/// # Safety
/// `gfx` must be valid.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_wait_idle(gfx: *mut Gfx) -> nva_abi::nva_status {
    if gfx.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    match (*gfx).vk.wait_idle() {
        Ok(()) => NVA_OK,
        Err(reason) => {
            eprintln!("[nva-runtime] wait_idle: {reason}");
            NVA_ERR_DEVICE_LOST
        }
    }
}

/// Wait for a submitted frame's fence, then read back the centre pixel.
///
/// # Safety
/// `gfx` must be valid and `fence` must point to a readable `nva_fence`.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_wait_fence(
    gfx: *mut Gfx,
    fence: *const nva_abi::nva_fence,
    timeout_ns: u64,
) -> nva_abi::nva_status {
    if gfx.is_null() || fence.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let gfx = &mut *gfx;
    let fence = &*fence;
    if fence.serial > u64::from(gfx.last_serial) {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if !gfx.pending {
        // Already waited (or nothing submitted): idempotent success.
        return NVA_OK;
    }
    match gfx.vk.wait_frame(timeout_ns) {
        Ok(()) => {
            gfx.pending = false;
            NVA_OK
        }
        Err(WaitError::Timeout) => NVA_ERR_TIMEOUT,
        Err(WaitError::Failed(reason)) => {
            eprintln!("[nva-runtime] wait_fence: {reason}");
            NVA_ERR_DEVICE_LOST
        }
    }
}

/// # Safety
/// `gfx` must be valid and `out` must point to a writable `nva_stats`.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_get_stats(
    gfx: *mut Gfx,
    out: *mut nva_stats,
) -> nva_abi::nva_status {
    if gfx.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let out = &mut *out;
    if out.size < size_of::<nva_stats>() as u32 || out.version != NVA_STATS_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let gfx = &*gfx;
    out.unsupported_calls = 0;
    out.unknown_resolves = 0;
    out.stub_aborts = 0;
    out.sticky_error = 0;
    out.last_serial = gfx.last_serial;
    out.flags = 0;
    out.frames = gfx.frames;
    out.draw_calls = gfx.draw_calls;
    out.submits = gfx.submits;
    NVA_OK
}

/// The NVN table is a later ticket's deliverable; the smoke path resolves its
/// six entry points through `nva_nvn_bootstrap`.
///
/// # Safety
/// `gfx` may be NULL.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_get_nvn_table(_gfx: *mut Gfx) -> *const nva_nvn_table {
    ptr::null()
}

/// Verifier-only probe: the four smoke JSON keys the frozen ABI does not
/// expose. Not part of `include/nva_abi.h` (#37 D4).
///
/// # Safety
/// `gfx` must be valid and `out` must point to a writable `nva_verify_state`.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_verify_probe(
    gfx: *mut Gfx,
    out: *mut nva_verify_state,
) -> nva_abi::nva_status {
    if gfx.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let out = &mut *out;
    if out.size < size_of::<nva_verify_state>() as u32
        || out.version != verify::NVA_VERIFY_STATE_VERSION
    {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let gfx = &*gfx;
    out.pixel_rgba = gfx.vk.pixel_rgba();
    out.validation_available = u32::from(gfx.vk.validation_available());
    out.validation_errors = gfx.vk.validation_errors();
    let name = gfx.vk.device_name().as_bytes();
    let length = name.len().min(out.device_name.len() - 1);
    for (slot, byte) in out.device_name.iter_mut().zip(name.iter()).take(length) {
        *slot = *byte as c_char;
    }
    out.device_name[length] = 0;
    NVA_OK
}

// ---------------------------------------------------------------- NVN

/// Resolve an NVN entry point by name for the current gfx object.
///
/// # Safety
/// `gfx` may be NULL; `name` must point to a NUL-terminated string.
#[no_mangle]
pub unsafe extern "C" fn nva_nvn_bootstrap(gfx: *mut Gfx, name: *const c_char) -> u64 {
    if gfx.is_null() || name.is_null() {
        return 0;
    }
    let name = CStr::from_ptr(name).to_str().unwrap_or("");
    nvn_entry(name).unwrap_or(nvn_zero_stub as *const () as u64)
}

unsafe extern "C" fn nvn_zero_stub() -> u64 {
    0
}

fn nvn_entry(name: &str) -> Option<u64> {
    macro_rules! address {
        ($function:expr) => {
            $function as *const () as u64
        };
    }
    let address = match name {
        "nvnDeviceBuilderSetDefaults" => address!(nvn_device_builder_set_defaults),
        "nvnDeviceInitialize" => address!(nvn_device_initialize),
        "nvnCommandBufferClearColor" => address!(nvn_command_buffer_clear_color),
        "nvnCommandBufferDrawArrays" => address!(nvn_command_buffer_draw_arrays),
        "nvnQueueSubmitCommands" => address!(nvn_queue_submit_commands),
        "nvnQueueFinish" => address!(nvn_queue_finish),
        _ => return None,
    };
    Some(address)
}

unsafe extern "C" fn nvn_device_builder_set_defaults(builder: *mut c_void) {
    if builder.is_null() {
        return;
    }
    // The builder is opaque to the runtime in this ticket; a real default set
    // lands with the NVN object model.
}

unsafe extern "C" fn nvn_device_initialize(
    device: *mut c_void,
    builder: *const c_void,
) -> u8 {
    if device.is_null() || builder.is_null() {
        return 0;
    }
    1
}

unsafe extern "C" fn nvn_command_buffer_clear_color(
    command_buffer: *mut c_void,
    _index: i32,
    color: *const f32,
    _mask: u32,
) {
    if command_buffer.is_null() || color.is_null() {
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.clear_rgba = [*color, *color.add(1), *color.add(2), *color.add(3)];
}

unsafe extern "C" fn nvn_command_buffer_draw_arrays(
    command_buffer: *mut c_void,
    _primitive: i32,
    _first: i32,
    count: i32,
) {
    if command_buffer.is_null() {
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    pending.draw_vertices = count.max(0) as u32;
}

unsafe extern "C" fn nvn_queue_submit_commands(
    queue: *mut c_void,
    count: i32,
    command_buffers: *const *mut c_void,
) {
    if queue.is_null() || count < 1 || command_buffers.is_null() {
        return;
    }
    let gfx = CURRENT_GFX.load(std::sync::atomic::Ordering::Acquire);
    if gfx.is_null() {
        return;
    }
    let (clear_rgba, draw_vertices) = {
        let pending = PENDING.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        (pending.clear_rgba, pending.draw_vertices)
    };
    let gfx = &mut *gfx;
    match gfx.vk.submit_frame(clear_rgba, draw_vertices) {
        Ok(serial) => {
            gfx.last_serial = serial;
            gfx.pending = true;
            gfx.frames = gfx.frames.saturating_add(1);
            gfx.submits = gfx.submits.saturating_add(1);
            if draw_vertices > 0 {
                gfx.draw_calls = gfx.draw_calls.saturating_add(1);
            }
        }
        Err(reason) => eprintln!("[nva-runtime] nvnQueueSubmitCommands: {reason}"),
    }
}

unsafe extern "C" fn nvn_queue_finish(_queue: *mut c_void) {
    // The fence wait is an explicit `nva_gfx_wait_fence` call in the smoke
    // path, so finish does not block here.
}

// ------------------------------------------------------- app / host
//
// Skeleton entry points: the shell lifecycle and the host surface are later
// tickets. They exist so the shared library carries all three ABI prefixes
// (#37 D6), and they fail honestly instead of pretending to succeed.

/// # Safety
/// `desc` and `out` must be readable/writable as declared.
#[no_mangle]
pub unsafe extern "C" fn nva_app_create(
    desc: *const nva_app_desc,
    out: *mut *mut c_void,
) -> nva_abi::nva_status {
    if desc.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let desc = &*desc;
    if desc.size < size_of::<nva_app_desc>() as u32 || desc.version != NVA_APP_DESC_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    *out = ptr::null_mut();
    NVA_ERR_UNSUPPORTED
}

/// # Safety
/// `app` must be NULL (no app object can be created in this ticket).
#[no_mangle]
pub unsafe extern "C" fn nva_app_destroy(_app: *mut c_void) {}

/// # Safety
/// `desc` and `out` must be readable/writable as declared.
#[no_mangle]
pub unsafe extern "C" fn nva_host_create(
    desc: *const nva_host_desc,
    out: *mut *mut c_void,
) -> nva_abi::nva_status {
    if desc.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let desc = &*desc;
    if desc.size < size_of::<nva_host_desc>() as u32
        || desc.version != nva_abi::NVA_HOST_DESC_VERSION
    {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    *out = ptr::null_mut();
    NVA_ERR_UNSUPPORTED
}

/// # Safety
/// `host` must be NULL (no host object can be created in this ticket).
#[no_mangle]
pub unsafe extern "C" fn nva_host_destroy(_host: *mut c_void) {}

fn max_sample_count(bits: u32) -> u32 {
    if bits == 0 {
        1
    } else {
        1 << (31 - bits.leading_zeros())
    }
}
