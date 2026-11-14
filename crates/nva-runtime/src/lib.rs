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

use std::sync::{Mutex, MutexGuard};

use nva_abi::{
    nva_app_desc, nva_gfx_caps, nva_gfx_desc, nva_host_desc, nva_nvn_table, nva_stats,
    NVA_APP_DESC_VERSION, NVA_ERR_DEVICE_LOST, NVA_ERR_INVALID_ARGUMENT, NVA_ERR_TIMEOUT,
    NVA_ERR_UNSUPPORTED, NVA_GFX_CAPS_VERSION, NVA_GFX_CAP_VULKAN_11, NVA_GFX_DESC_VERSION, NVA_OK,
    NVA_STATS_VERSION,
};

pub mod debug_filter;
pub mod verify;
mod vk;

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

static PENDING: Mutex<PendingFrame> = Mutex::new(PendingFrame {
    clear_rgba: [0.0; 4],
    draw_vertices: 0,
});

/// Locks `PENDING`, taking the guard back out of a poisoned mutex rather than
/// failing.
///
/// Recovering is safe here for three reasons:
///
/// 1. `PendingFrame` is plain data: a `[f32; 4]` and a `u32`, with no invariant
///    tying one field to the other. No panic can leave it in a state a later
///    reader would mistake for a valid frame.
/// 2. The engine's three critical sections only read or write the frame's
///    fields. They allocate nothing, index nothing, and call nothing that can
///    panic, so a panic inside one of them cannot come from this crate's own
///    work and cannot interrupt a write halfway. The only panic raised while
///    the guard is held is in `pending_recovers_a_poisoned_lock` below, which
///    takes the lock on purpose to create the poison and writes nothing while
///    it holds it.
/// 3. Release builds set `panic = "abort"` in the workspace `Cargo.toml`
///    (`[profile.release]`), so a poisoned `PENDING` cannot exist there at all:
///    it is reachable only under the unwind semantics of the `test` profile,
///    where an unintended panic has already failed the test that caused it.
///    Failing the lock here would instead turn that one failure into a
///    cascade across every later submission in the same test binary.
fn pending() -> MutexGuard<'static, PendingFrame> {
    PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
// SAFETY: `desc` and `out` are guest-owned and must stay readable/writable for
// the whole call. Both are null-checked, and `size` / `version` are validated
// before any other field is read, so a malformed struct fails with
// `NVA_ERR_INVALID_ARGUMENT` instead of being dereferenced. On success `*out`
// owns the boxed `Gfx`, and only `nva_gfx_destroy` may free it; the host table
// behind `desc.host` must outlive that object. The guest calls this from one
// thread, before any other export has a reason to touch `CURRENT_GFX`.
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
// SAFETY: `gfx` must be the pointer `nva_gfx_create` wrote and must not have
// been destroyed. When `gfx` is the current object the slot in `CURRENT_GFX` is
// cleared before `Box::from_raw`, so the object is dropped exactly once and
// `nvn_queue_submit_commands` afterwards finds an empty slot and returns.
// NULL is accepted and does nothing. No other thread may be inside an export
// that uses the same `gfx` — destroy is the caller's last use.
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
// SAFETY: `gfx` must still be alive and `out` must point to a writable
// `nva_gfx_caps` that the caller keeps exclusively borrowed for the call. The
// write happens only after `out.size` / `out.version` pass, so a shorter struct
// is refused with `NVA_ERR_INVALID_ARGUMENT` rather than overrun. Nothing is
// retained: the caps are copied out and the borrow ends with the call.
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
// SAFETY: `gfx` must be a live object and the caller must hold it exclusively
// for the call — `device_wait_idle` needs external synchronization on the
// device, which the caller's single-threaded ownership provides. NULL returns
// `NVA_ERR_INVALID_ARGUMENT`; a failed wait is reported as `NVA_ERR_DEVICE_LOST`
// and never as success.
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
// SAFETY: `gfx` must be live and held exclusively, and `fence` must point to a
// readable `nva_fence` owned by the guest for the call. The serial is compared
// before the object state is touched, so a fence from a later frame is an
// `NVA_ERR_INVALID_ARGUMENT` instead of a state corruption. The `&mut` is what
// makes the frame bookkeeping single-writer; two concurrent waits would race
// `pending` as well as the Vulkan fence.
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

/// Fills `out` from the engine's counters.
///
/// Version discipline: the layout is append-only (v2 appends nine `u32`
/// counters after the frozen 56 B prefix), but this entry point serves only the
/// current `NVA_STATS_VERSION`. A v1 caller is refused with
/// `NVA_ERR_INVALID_ARGUMENT`. That is deliberate (spec #37, GATES §2.4,
/// docs/ARCHITECTURE.md §7.4): one build has one stats layout, and answering
/// an older version would mean inventing the counters it never had.
///
/// # Safety
/// `gfx` must be valid, and `out` must point to a writable `nva_stats` whose
/// `size` and `version` fields are already initialised by the caller.
// SAFETY: `gfx` must be live and `out` must point to a writable `nva_stats`
// held exclusively by the caller. `out.size` / `out.version` are read before
// the first write, so the caller has to initialise those two fields; an older
// or shorter struct is then refused with `NVA_ERR_INVALID_ARGUMENT`. Every
// field except `size` and `version` is written; those two stay the caller's.
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
    // v2 counters: #93/#96 produce them. Zero here so a caller that did not
    // initialise its struct never reads uninitialised memory back.
    out.presents = 0;
    out.surface_generation = 0;
    out.missing_variants = 0;
    out.draw_skipped = 0;
    out.xruns = 0;
    out.input_events = 0;
    out.input_consumed = 0;
    out.guest_svc_count = 0;
    out.draw_caller_in_guest = 0;
    NVA_OK
}

/// The NVN table is a later ticket's deliverable; the smoke path resolves its
/// six entry points through `nva_nvn_bootstrap`.
///
/// # Safety
/// `gfx` may be NULL.
// SAFETY: the parameter is never dereferenced — the table is a later ticket's
// deliverable and the body returns a constant `null` — so no validity
// requirement rides on `_gfx`: passing NULL is as sound as passing a live
// object.
#[no_mangle]
pub unsafe extern "C" fn nva_gfx_get_nvn_table(_gfx: *mut Gfx) -> *const nva_nvn_table {
    ptr::null()
}

/// Verifier-only probe: the four smoke JSON keys the frozen ABI does not
/// expose. Not part of `include/nva_abi.h` (#37 D4).
///
/// # Safety
/// `gfx` must be valid and `out` must point to a writable `nva_verify_state`.
// SAFETY: `gfx` must be live and `out` must point to a writable
// `nva_verify_state` held exclusively by the caller. `size` / `version` are
// checked first; the device name copy is clamped to `device_name.len() - 1` and
// the terminator written inside the array, so the guest buffer is never
// overrun. Only `nva-verify` links against this export (#37 D4).
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
// SAFETY: `gfx` may be NULL (checked), and `name` must point to a
// NUL-terminated string that stays readable for the call — `CStr::from_ptr`
// walks it to the terminator and keeps no borrow. A name that is not valid
// UTF-8 resolves to the zero stub rather than panicking, so a bad guest string
// cannot unwind across the ABI (`LOCAL §2`).
#[no_mangle]
pub unsafe extern "C" fn nva_nvn_bootstrap(gfx: *mut Gfx, name: *const c_char) -> u64 {
    if gfx.is_null() || name.is_null() {
        return 0;
    }
    let name = CStr::from_ptr(name).to_str().unwrap_or("");
    nvn_entry(name).unwrap_or(nvn_zero_stub as *const () as u64)
}

// SAFETY: unknown names resolve here. The slot ignores its arguments — the
// signature has none — and returns 0, the NVN "nothing was recorded" value, so
// no pointer is dereferenced whatever the guest passes. The marker is on the
// declaration because its address goes into the table as a callable slot; the
// body itself relies on nothing from the caller.
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

// SAFETY: `_builder` is opaque in this ticket and is never read or written —
// the slot is a deliberate no-op until the NVN object model lands — so the
// caller's only obligation is the NVN one: pass the builder it allocated. No
// pointer is dereferenced, and the slot returns nothing, which is where a
// failure status would otherwise go (`LOCAL §2`).
unsafe extern "C" fn nvn_device_builder_set_defaults(_builder: *mut c_void) {
    // The builder is opaque to the runtime in this ticket; a real default set
    // lands with the NVN object model.
}

// SAFETY: both pointers are null-checked and then never dereferenced; the slot
// only answers "initialized" once the guest has supplied two live objects. That
// makes the body sound for any non-NULL pair, and a NULL pair returns 0 rather
// than reading a device the runtime does not model yet.
unsafe extern "C" fn nvn_device_initialize(device: *mut c_void, builder: *const c_void) -> u8 {
    if device.is_null() || builder.is_null() {
        return 0;
    }
    1
}

// SAFETY: `color` must point to at least four readable `f32` — the NVN
// signature is `const float[4]` — and the body reads exactly indices 0..3
// before the borrow ends. `command_buffer` is null-checked and otherwise
// unused. `PENDING` is process-wide, and taking the lock means the clear state
// is written under exclusion. `pending()` recovers a poisoned lock instead of
// failing the call; its own comment gives the reasons, and the critical section
// here contains no panicking code.
unsafe extern "C" fn nvn_command_buffer_clear_color(
    command_buffer: *mut c_void,
    _index: i32,
    color: *const f32,
    _mask: u32,
) {
    if command_buffer.is_null() || color.is_null() {
        return;
    }
    let mut frame = pending();
    frame.clear_rgba = [*color, *color.add(1), *color.add(2), *color.add(3)];
}

// SAFETY: `command_buffer` is null-checked and never dereferenced; `count` is
// clamped at zero before it becomes a `u32` vertex count. The write goes to
// `PENDING` under the same process-wide lock as the clear slot, so a guest that
// records clear then draw on one thread sees them in order.
unsafe extern "C" fn nvn_command_buffer_draw_arrays(
    command_buffer: *mut c_void,
    _primitive: i32,
    _first: i32,
    count: i32,
) {
    if command_buffer.is_null() {
        return;
    }
    let mut frame = pending();
    frame.draw_vertices = count.max(0) as u32;
}

// SAFETY: `queue` and `command_buffers` are null-checked and the array is never
// read — the frame is built from the `PENDING` snapshot, not from the guest's
// command buffers — so `count >= 1` is the whole array contract this body
// needs. `CURRENT_GFX` is loaded with Acquire, and the `&mut *gfx` it yields is
// sound because the NVN entry points run on the guest's single render thread,
// which does not call `nva_gfx_destroy` or `nva_gfx_wait_fence` at the same
// time; `last_serial` and `pending` are only written under that assumption.
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
        let frame = pending();
        (frame.clear_rgba, frame.draw_vertices)
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

// SAFETY: `_queue` is never dereferenced — the fence wait is the explicit
// `nva_gfx_wait_fence` call in the smoke path — so the pointer carries no
// obligation beyond the NVN one of being a queue the guest owns.
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
// SAFETY: `desc` and `out` are guest-owned and must stay readable/writable for
// the call; both are null-checked and `size` / `version` are validated before
// any other field is read. This skeleton always writes NULL to `*out` and
// returns `NVA_ERR_UNSUPPORTED` (`LOCAL §2`: the failure is the return value,
// not a panic), so no object is ever handed out to own.
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
// SAFETY: no app object can be created in this ticket, so the only sound
// argument is NULL; the body ignores it either way and dereferences nothing.
#[no_mangle]
pub unsafe extern "C" fn nva_app_destroy(_app: *mut c_void) {}

/// # Safety
/// `desc` and `out` must be readable/writable as declared.
// SAFETY: `desc` and `out` are guest-owned and must stay readable/writable for
// the call; both are null-checked and `size` / `version` are validated before
// any other field is read. This skeleton always writes NULL to `*out` and
// returns `NVA_ERR_UNSUPPORTED`, so no object is ever handed out to own.
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
// SAFETY: no host object can be created in this ticket, so the only sound
// argument is NULL; the body ignores it either way and dereferences nothing.
#[no_mangle]
pub unsafe extern "C" fn nva_host_destroy(_host: *mut c_void) {}

fn max_sample_count(bits: u32) -> u32 {
    if bits == 0 {
        1
    } else {
        1 << (31 - bits.leading_zeros())
    }
}

#[cfg(test)]
mod tests {
    use super::{pending, PENDING};

    /// A panic while the guard is held poisons `PENDING`, and the poison
    /// outlives the test that caused it. A helper thread produces the panic
    /// without `catch_unwind`: `join` hands it back as an `Err` instead of
    /// letting it escape into this test.
    ///
    /// The poison is permanent, so this test is meaningful once per process:
    /// if another test poisons `PENDING` before this one runs, the assertion
    /// below fails.
    #[test]
    fn pending_recovers_a_poisoned_lock() {
        // Asserted, not assumed: without this line an already-poisoned
        // `PENDING` would let the test pass while proving nothing — the
        // helper's `expect` fails on the poisoned lock, `join` hands back an
        // `Err`, and `is_poisoned()` is already true. With it, that case is a
        // failure instead. #66 runs the crate's whole test binary, so another
        // test could get there first.
        assert!(!PENDING.is_poisoned(), "PENDING must start unpoisoned");
        let join_result = std::thread::spawn(|| {
            // TEST-ONLY: lock `PENDING` directly to create the poison. The
            // engine's own code reaches it only through `pending()`.
            let _guard = PENDING.lock().expect("PENDING starts unpoisoned");
            panic!("poison PENDING on purpose");
        })
        .join();
        assert!(join_result.is_err(), "the helper thread must have panicked");
        assert!(
            PENDING.is_poisoned(),
            "the panic must leave PENDING poisoned"
        );

        let mut frame = pending();
        frame.draw_vertices = 7;
        assert_eq!(
            frame.draw_vertices, 7,
            "the recovered guard must be writable"
        );
        // Restore the initial value rather than leave the poisoned static
        // describing this test.
        frame.draw_vertices = 0;
    }
}
