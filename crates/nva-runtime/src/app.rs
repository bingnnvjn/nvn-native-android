//! The shell-facing app object: the lifecycle and the window (`nva_app_*`).
//!
//! `docs/ARCHITECTURE.md` §3.7 makes the platform shell the owner of the
//! surface: it sees `surfaceCreated` / `surfaceChanged` / `surfaceDestroyed`,
//! keeps the generation counter, and forwards each event through this module.
//! §7.1 calls that the shell → runtime seam; this is its first real traffic.
//!
//! The module records what the shell reports — the lifecycle state, the window,
//! its size, the generation — and calls back into the shell when a window
//! arrives or leaves. It builds no Vulkan object: `nva_app_attach_window` stops
//! at the recording on purpose. The surface/swapchain pair, and the engine that
//! owns it, arrive with #93.
//!
//! ## Who owns the `ANativeWindow` reference
//!
//! The shell (`docs/ARCHITECTURE.md` §7.3): it acquires one reference before
//! `nva_app_attach_window` and releases it only after
//! `nva_app_detach_window` has returned. This module borrows the bare pointer
//! in between, never acquires a reference, and never touches the window after
//! detach returns. When #93's engine starts building a `VkSurfaceKHR` it takes
//! a reference of its own at attach, as §7.3 says; the two references are
//! independent counts on the same window.
//!
//! That split is safe because of what comes between the two calls: the runtime
//! borrows the window only while its record exists, and the record exists only
//! between a successful attach and a successful detach. There is no moment when
//! the shell has released a reference the runtime still points at, and none
//! when the runtime holds a pointer past a release.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

use std::sync::{Mutex, MutexGuard};

use nva_abi::{
    nva_app_callbacks, nva_app_desc, nva_stats, nva_status, NVA_APP_CALLBACKS_VERSION,
    NVA_APP_DESC_VERSION, NVA_ERR_INVALID_ARGUMENT, NVA_ERR_STALE, NVA_OK, NVA_STATS_VERSION,
};

/// Where the shell has taken the app, following Android's activity callbacks.
///
/// `Started` and `Resumed` are separate because Android separates them: a cold
/// start is `onStart` then `onResume`, and an app that treated the second call
/// as invalid would log a refusal on every launch. `Created` and `Stopped` both
/// accept `start` for the same reason, since returning from the background runs
/// `onStart` again.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lifecycle {
    /// `nva_app_create` returned; `onStart` has not run yet.
    Created,
    /// Between `onStart` and `onResume`, or between `onPause` and `onResume`.
    Started,
    /// Between `onResume` and `onPause`: the interactive state.
    Resumed,
    /// Between `onPause` and the next `onResume` — still visible.
    Paused,
    /// Between `onStop` and the next `onStart`.
    Stopped,
}

/// The window the shell attached, borrowed until it is detached.
///
/// The fields are this ticket's output: the record that `attach` writes and
/// `detach` clears. Nothing reads them yet — #93's engine is the reader, taking
/// `native` to build the `VkSurfaceKHR` and the size to size the swapchain — so
/// the three sit behind an `allow` until that reader lands. Removing them
/// instead would remove the thing the ticket exists to produce.
struct AttachedWindow {
    #[allow(dead_code)]
    native: *mut c_void,
    #[allow(dead_code)]
    width: u32,
    #[allow(dead_code)]
    height: u32,
    generation: u32,
}

struct State {
    lifecycle: Lifecycle,
    window: Option<AttachedWindow>,
}

/// The shell callbacks this module can raise, copied out of `nva_app_desc`.
///
/// Copying ends the borrow: the shell may keep its `nva_app_callbacks` wherever
/// it likes, and nothing here dereferences that struct again. Only the two
/// surface callbacks are copied because nothing in this skeleton can raise
/// `on_device_lost` or `on_fatal`; they come with the tickets that can.
#[derive(Clone, Copy)]
struct SurfaceCallbacks {
    user: *mut c_void,
    ready: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32)>,
    lost: Option<unsafe extern "C" fn(*mut c_void, u32)>,
}

impl SurfaceCallbacks {
    /// A set that raises nothing, for a shell that passes no callbacks.
    fn none() -> Self {
        Self {
            user: ptr::null_mut(),
            ready: None,
            lost: None,
        }
    }

    fn from_callbacks(callbacks: &nva_app_callbacks) -> Self {
        Self {
            user: callbacks.user,
            ready: callbacks.on_surface_ready,
            lost: callbacks.on_surface_lost,
        }
    }
}

/// The object behind the opaque `nva_app *` (`include/nva_abi.h`).
///
/// The shell creates one, drives it from its main thread, and destroys it in
/// `onDestroy`. The state sits behind a mutex so that the single-writer
/// assumption is written down in the type rather than in a comment: today every
/// caller is the shell's main thread, which is why nothing here needs `App` to
/// be `Send`. #93 adds a reader on the render thread, and that ticket has to
/// decide how the two threads share the object — making this type `Sync`
/// silently would answer that question in the wrong place.
pub struct App {
    state: Mutex<State>,
    callbacks: SurfaceCallbacks,
}

/// Locks the app state.
///
/// Nothing holds this guard across a call that can panic: every critical
/// section below is a field read or a field write, and the shell callbacks run
/// after the guard is dropped. A poisoned lock would therefore mean a panic
/// escaped from some future critical section, and the record could not be
/// trusted afterwards. `LOCAL §4` recovers on poison for `PENDING` only and says
/// the exception does not extend to any other lock, so this one fails the call
/// with `NVA_ERR_STALE` instead of reaching inside. Under `panic = "abort"`
/// (`LOCAL §1`) the branch cannot be taken at all; it exists for the profile
/// where it can, and for a future panic that slips in.
fn lock(app: &App) -> Result<MutexGuard<'_, State>, nva_status> {
    app.state.lock().map_err(|_| NVA_ERR_STALE)
}

/// Create the app object (`nva_app_create`).
///
/// # Safety
/// `desc` must point to a readable `nva_app_desc` and `out` to a writable
/// pointer slot.
// SAFETY: `desc` and `out` are shell-owned and must stay readable/writable for
// the whole call. Both are null-checked, and `size` / `version` are validated
// before any other field is read, so a short or stale struct fails with
// `NVA_ERR_INVALID_ARGUMENT` instead of being dereferenced. `desc.callbacks`,
// when non-NULL, has its own `size` / `version` checked before it is read.
// Nothing in `desc` is retained: `data_dir` and `platform` are not stored in
// this skeleton, and the callback pointers are copied. On success `*out` owns
// the boxed `App`, and only `nva_app_destroy` may free it.
#[no_mangle]
pub unsafe extern "C" fn nva_app_create(
    desc: *const nva_app_desc,
    out: *mut *mut App,
) -> nva_status {
    if desc.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let desc = &*desc;
    if desc.size < size_of::<nva_app_desc>() as u32 || desc.version != NVA_APP_DESC_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    *out = ptr::null_mut();

    let callbacks = if desc.callbacks.is_null() {
        SurfaceCallbacks::none()
    } else {
        // SAFETY: non-NULL, so the shell is handing over a callbacks struct that
        // it keeps alive for the call; `size` / `version` are checked before any
        // field is read, so a stale struct is refused rather than misread.
        let callbacks = unsafe { &*desc.callbacks };
        if callbacks.size < size_of::<nva_app_callbacks>() as u32
            || callbacks.version != NVA_APP_CALLBACKS_VERSION
        {
            return NVA_ERR_INVALID_ARGUMENT;
        }
        SurfaceCallbacks::from_callbacks(callbacks)
    };

    let app = Box::new(App {
        state: Mutex::new(State {
            lifecycle: Lifecycle::Created,
            window: None,
        }),
        callbacks,
    });
    *out = Box::into_raw(app);
    NVA_OK
}

/// Destroy the app object (`nva_app_destroy`).
///
/// # Safety
/// `app` must be a pointer returned by `nva_app_create`, or NULL, and must not
/// be used again.
// SAFETY: `app` is the pointer `nva_app_create` wrote and is freed exactly
// once here; NULL is accepted and does nothing. The shell must have detached
// its window first: this function frees the record, not the reference — the
// `ANativeWindow` reference belongs to the shell (§7.3), and releasing it here
// would release something this module never acquired.
#[no_mangle]
pub unsafe extern "C" fn nva_app_destroy(app: *mut App) {
    if app.is_null() {
        return;
    }
    drop(Box::from_raw(app));
}

/// Move to `Running` (`nva_app_start`, Android's `onStart`).
///
/// # Safety
/// `app` must be a pointer returned by `nva_app_create`, or NULL.
// SAFETY: the pointer is null-checked and then borrowed only for the call; the
// state is a plain `Mutex<State>` inside the box the shell owns, and the guard
// is dropped before this function returns. No field of the borrowed struct is
// read through a pointer with an inferred lifetime beyond the call.
#[no_mangle]
pub unsafe extern "C" fn nva_app_start(app: *mut App) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let Ok(mut state) = lock(&*app) else {
        return NVA_ERR_STALE;
    };
    match state.lifecycle {
        Lifecycle::Created | Lifecycle::Stopped => {
            state.lifecycle = Lifecycle::Started;
            NVA_OK
        }
        Lifecycle::Started | Lifecycle::Resumed | Lifecycle::Paused => NVA_ERR_INVALID_ARGUMENT,
    }
}

/// Move to `Paused` (`nva_app_pause`, Android's `onPause`).
///
/// # Safety
/// `app` must be a pointer returned by `nva_app_create`, or NULL.
// SAFETY: as `nva_app_start`: null-checked, borrowed for the call, and the
// guard drops before return.
#[no_mangle]
pub unsafe extern "C" fn nva_app_pause(app: *mut App) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let Ok(mut state) = lock(&*app) else {
        return NVA_ERR_STALE;
    };
    match state.lifecycle {
        Lifecycle::Resumed => {
            state.lifecycle = Lifecycle::Paused;
            NVA_OK
        }
        Lifecycle::Created | Lifecycle::Started | Lifecycle::Paused | Lifecycle::Stopped => {
            NVA_ERR_INVALID_ARGUMENT
        }
    }
}

/// Move back to `Running` (`nva_app_resume`, Android's `onResume`).
///
/// # Safety
/// `app` must be a pointer returned by `nva_app_create`, or NULL.
// SAFETY: as `nva_app_start`: null-checked, borrowed for the call, and the
// guard drops before return.
#[no_mangle]
pub unsafe extern "C" fn nva_app_resume(app: *mut App) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let Ok(mut state) = lock(&*app) else {
        return NVA_ERR_STALE;
    };
    match state.lifecycle {
        Lifecycle::Started | Lifecycle::Paused => {
            state.lifecycle = Lifecycle::Resumed;
            NVA_OK
        }
        Lifecycle::Created | Lifecycle::Resumed | Lifecycle::Stopped => NVA_ERR_INVALID_ARGUMENT,
    }
}

/// Move to `Stopped` (`nva_app_stop`, Android's `onStop`).
///
/// A window may still be attached here: Android destroys the surface on its own
/// schedule, and §3.7 puts the ≤50 ms wait for that at detach, not at stop.
///
/// # Safety
/// `app` must be a pointer returned by `nva_app_create`, or NULL.
// SAFETY: as `nva_app_start`: null-checked, borrowed for the call, and the
// guard drops before return.
#[no_mangle]
pub unsafe extern "C" fn nva_app_stop(app: *mut App) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let Ok(mut state) = lock(&*app) else {
        return NVA_ERR_STALE;
    };
    match state.lifecycle {
        Lifecycle::Started | Lifecycle::Resumed | Lifecycle::Paused => {
            state.lifecycle = Lifecycle::Stopped;
            NVA_OK
        }
        Lifecycle::Created | Lifecycle::Stopped => NVA_ERR_INVALID_ARGUMENT,
    }
}

/// Record the window the shell just got (`nva_app_attach_window`).
///
/// A generation of 0 is refused: the shell numbers surfaces from 1, and 0 is
/// what a variable that was never set to a surface holds.
///
/// Attaching while a window is already attached replaces the record. The shell
/// emits that on a size change (rotation), where the new geometry supersedes the
/// old one and the generation has advanced; it is not a detach, so
/// `on_surface_lost` is not raised for the window being replaced.
///
/// The geometry is recorded exactly as reported, including a zero on either
/// axis. Refusing that would move a decision into the ABI that belongs to the
/// shell, which is the one place that knows whether a surface is momentarily
/// unmeasured; #93 is where a zero-sized swapchain has to be caught.
///
/// # Safety
/// `app` must come from `nva_app_create`, and `native_window` must stay valid
/// until the matching `nva_app_detach_window` returns.
// SAFETY: `app` is null-checked and borrowed for the call. `native_window` is
// never dereferenced here — the pointer is stored as an opaque value and handed
// back to the shell untouched — so the only requirement it carries is the
// ownership one in the doc comment: a live window that outlives the record.
// The shell callback runs after the state guard is dropped, so a shell that
// calls back into `nva_app_*` from it cannot deadlock on this mutex.
#[no_mangle]
pub unsafe extern "C" fn nva_app_attach_window(
    app: *mut App,
    native_window: *mut c_void,
    width: u32,
    height: u32,
    generation: u32,
) -> nva_status {
    if app.is_null() || native_window.is_null() || generation == 0 {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let app = &*app;
    {
        let Ok(mut state) = lock(app) else {
            return NVA_ERR_STALE;
        };
        state.window = Some(AttachedWindow {
            native: native_window,
            width,
            height,
            generation,
        });
    }
    let callbacks = app.callbacks;
    if let Some(ready) = callbacks.ready {
        // SAFETY: `ready` is a shell function the shell itself wrote into
        // `nva_app_callbacks`; its contract is the C signature in
        // `include/nva_abi.h` (user, generation, width, height), which is what
        // is passed. `user` is the shell's own opaque pointer, unmodified.
        unsafe { ready(callbacks.user, generation, width, height) };
    }
    NVA_OK
}

/// Forget the window (`nva_app_detach_window`).
///
/// Called with the generation the shell attached. A different generation means
/// the two sides disagree about which window is live, which is the one state
/// worth failing loudly: silently clearing would leave the shell holding a
/// reference for a window this module no longer knows about.
///
/// # Safety
/// `app` must come from `nva_app_create`.
// SAFETY: `app` is null-checked and borrowed for the call. The window record is
// dropped before the callback runs, and the stored `native_window` is not
// touched again — not read, not released — so the shell is free to release its
// reference as soon as this call returns (§7.3).
#[no_mangle]
pub unsafe extern "C" fn nva_app_detach_window(app: *mut App, generation: u32) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let app = &*app;
    {
        let Ok(mut state) = lock(app) else {
            return NVA_ERR_STALE;
        };
        match state.window.as_ref() {
            Some(window) if window.generation == generation => {}
            Some(_) | None => return NVA_ERR_INVALID_ARGUMENT,
        }
        state.window = None;
    }
    let callbacks = app.callbacks;
    if let Some(lost) = callbacks.lost {
        // SAFETY: `lost` is a shell function the shell wrote into
        // `nva_app_callbacks`, called with the C signature from
        // `include/nva_abi.h` (user, generation) and the shell's own `user`.
        unsafe { lost(callbacks.user, generation) };
    }
    NVA_OK
}

/// Write the stats snapshot (`nva_app_get_stats`).
///
/// The skeleton has nothing to count yet: this ticket renders nothing and
/// presents nothing, so every counter is written as 0 rather than left holding
/// whatever the shell passed in. `surface_generation` becomes a field of
/// `nva_stats` in the ABI append #93 needs, and gets its production writer
/// there.
///
/// # Safety
/// `app` must come from `nva_app_create` and `out` must point to a writable
/// `nva_stats`.
// SAFETY: `app` is null-checked and borrowed only to prove the caller holds a
// live object. `out` is shell-owned and must stay writable for the call;
// `size` / `version` are checked before the first write, so an older or shorter
// struct is refused with `NVA_ERR_INVALID_ARGUMENT` instead of overrun.
// Nothing is retained: the snapshot is copied out and the borrow ends with the
// call.
#[no_mangle]
pub unsafe extern "C" fn nva_app_get_stats(app: *mut App, out: *mut nva_stats) -> nva_status {
    if app.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let out = &mut *out;
    if out.size < size_of::<nva_stats>() as u32 || out.version != NVA_STATS_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    out.unsupported_calls = 0;
    out.unknown_resolves = 0;
    out.stub_aborts = 0;
    out.sticky_error = 0;
    out.last_serial = 0;
    out.flags = 0;
    out.frames = 0;
    out.draw_calls = 0;
    out.submits = 0;
    NVA_OK
}

#[cfg(test)]
mod tests {
    use core::ffi::c_void;
    use core::mem::size_of;
    use core::ptr;

    use std::sync::atomic::{AtomicU32, Ordering};

    use nva_abi::{
        nva_app_callbacks, nva_app_desc, nva_stats, NVA_APP_CALLBACKS_VERSION,
        NVA_APP_DESC_VERSION, NVA_ERR_INVALID_ARGUMENT, NVA_OK, NVA_STATS_VERSION,
    };

    use super::{
        nva_app_attach_window, nva_app_create, nva_app_destroy, nva_app_detach_window,
        nva_app_get_stats, nva_app_pause, nva_app_resume, nva_app_start, nva_app_stop, App,
    };
    use crate::CURRENT_GFX;

    /// What the shell callbacks record. Each test owns one and passes it as
    /// `user`, so the tests share no state and can run in parallel.
    #[derive(Default)]
    struct Probe {
        ready_calls: AtomicU32,
        ready_generation: AtomicU32,
        ready_width: AtomicU32,
        ready_height: AtomicU32,
        lost_calls: AtomicU32,
        lost_generation: AtomicU32,
    }

    unsafe extern "C" fn on_surface_ready(
        user: *mut c_void,
        generation: u32,
        width: u32,
        height: u32,
    ) {
        // SAFETY: every test passes `user` as a pointer to a `Probe` that
        // outlives the app it belongs to; the callbacks are only reachable
        // through that app.
        let probe = unsafe { &*(user as *const Probe) };
        probe.ready_calls.fetch_add(1, Ordering::SeqCst);
        probe.ready_generation.store(generation, Ordering::SeqCst);
        probe.ready_width.store(width, Ordering::SeqCst);
        probe.ready_height.store(height, Ordering::SeqCst);
    }

    unsafe extern "C" fn on_surface_lost(user: *mut c_void, generation: u32) {
        // SAFETY: as `on_surface_ready`.
        let probe = unsafe { &*(user as *const Probe) };
        probe.lost_calls.fetch_add(1, Ordering::SeqCst);
        probe.lost_generation.store(generation, Ordering::SeqCst);
    }

    fn callbacks(probe: &Probe) -> nva_app_callbacks {
        nva_app_callbacks {
            size: size_of::<nva_app_callbacks>() as u32,
            version: NVA_APP_CALLBACKS_VERSION,
            user: probe as *const Probe as *mut c_void,
            on_surface_ready: Some(on_surface_ready),
            on_surface_lost: Some(on_surface_lost),
            on_device_lost: None,
            on_fatal: None,
        }
    }

    fn desc(callbacks: *const nva_app_callbacks) -> nva_app_desc {
        nva_app_desc {
            size: size_of::<nva_app_desc>() as u32,
            version: NVA_APP_DESC_VERSION,
            flags: 0,
            reserved: 0,
            data_dir: ptr::null(),
            callbacks,
            platform: ptr::null(),
        }
    }

    /// Creates an app and returns it with the guard that destroys it.
    fn create(callbacks: *const nva_app_callbacks) -> *mut App {
        let desc = desc(callbacks);
        let mut app: *mut App = ptr::null_mut();
        // SAFETY: both pointers are to locals of this function.
        let status = unsafe { nva_app_create(&desc, &mut app) };
        assert_eq!(status, NVA_OK, "a well-formed desc must create an app");
        assert!(!app.is_null(), "create must hand out an object");
        app
    }

    /// Destroys an app created by `create`.
    fn destroy(app: *mut App) {
        // SAFETY: `app` came from `create` and is not used afterwards.
        unsafe { nva_app_destroy(app) };
    }

    #[test]
    fn create_rejects_missing_or_malformed_arguments() {
        let mut app: *mut App = ptr::null_mut();
        let valid = desc(ptr::null());
        // SAFETY: NULL is the null case the export documents.
        assert_eq!(
            unsafe { nva_app_create(ptr::null(), &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a NULL desc is refused"
        );
        // SAFETY: NULL is the null case the export documents.
        assert_eq!(
            unsafe { nva_app_create(&valid, ptr::null_mut()) },
            NVA_ERR_INVALID_ARGUMENT,
            "a NULL out slot is refused"
        );

        let short = nva_app_desc {
            size: (size_of::<nva_app_desc>() - 8) as u32,
            ..desc(ptr::null())
        };
        // SAFETY: `short` is a local whose `size` field is the only thing read.
        assert_eq!(
            unsafe { nva_app_create(&short, &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a desc shorter than the struct is refused"
        );

        let stale = nva_app_desc {
            version: NVA_APP_DESC_VERSION + 1,
            ..desc(ptr::null())
        };
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_create(&stale, &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a desc with an unknown version is refused"
        );
        assert!(app.is_null(), "a refused create leaves the out slot NULL");
    }

    #[test]
    fn create_refuses_a_malformed_callbacks_struct() {
        let probe = Probe::default();
        let mut callbacks = callbacks(&probe);
        callbacks.version = NVA_APP_CALLBACKS_VERSION + 1;
        let desc = desc(&callbacks);
        let mut app: *mut App = ptr::null_mut();
        // SAFETY: `desc` points at a local callbacks struct that outlives the call.
        assert_eq!(
            unsafe { nva_app_create(&desc, &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a callbacks struct with an unknown version must not be read"
        );
        assert!(app.is_null());
    }

    #[test]
    fn create_accepts_a_shell_with_no_callbacks() {
        let app = create(ptr::null());
        // Attaching must work with no callbacks at all: the shell is allowed to
        // leave them out, and the runtime must not require them to record a
        // window.
        // SAFETY: `app` is live and `1` is a valid generation.
        assert_eq!(
            unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 1) },
            NVA_OK
        );
        destroy(app);
    }

    #[test]
    fn attach_records_the_window_and_notifies_the_shell() {
        let probe = Probe::default();
        let callbacks = callbacks(&probe);
        let app = create(&callbacks);

        // SAFETY: `app` is live; the window pointer is opaque to the runtime and
        // is never dereferenced, so a non-NULL sentinel is a legal argument.
        let status = unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 1) };
        assert_eq!(status, NVA_OK);
        assert_eq!(probe.ready_calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.ready_generation.load(Ordering::SeqCst), 1);
        assert_eq!(probe.ready_width.load(Ordering::SeqCst), 1920);
        assert_eq!(probe.ready_height.load(Ordering::SeqCst), 1080);

        // The attach path must not create the gfx object: `CURRENT_GFX` is
        // written by `nva_gfx_create` alone, so an empty slot after an attach
        // says the window path never reached the Vulkan object graph (#93 owns
        // that). This does not by itself prove no `VkInstance` was made; the
        // proof is the code, which records a pointer and nothing else.
        assert!(
            CURRENT_GFX.load(Ordering::SeqCst).is_null(),
            "attach must leave the gfx slot empty"
        );

        destroy(app);
    }

    #[test]
    fn attach_replaces_the_window_when_the_generation_advances() {
        let probe = Probe::default();
        let callbacks = callbacks(&probe);
        let app = create(&callbacks);

        // SAFETY: as in `attach_records_the_window_and_notifies_the_shell`.
        unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 1) };
        // SAFETY: as above, with a different generation.
        let status = unsafe { nva_app_attach_window(app, 0x2000 as *mut c_void, 1080, 1920, 2) };
        assert_eq!(status, NVA_OK, "a size change re-attaches");
        assert_eq!(probe.ready_calls.load(Ordering::SeqCst), 2);
        assert_eq!(probe.ready_generation.load(Ordering::SeqCst), 2);
        assert_eq!(
            probe.lost_calls.load(Ordering::SeqCst),
            0,
            "replacing a window is not a detach"
        );

        destroy(app);
    }

    #[test]
    fn attach_refuses_a_null_window_and_generation_zero() {
        let app = create(ptr::null());
        // SAFETY: `app` is live; both arguments are the invalid ones the export
        // documents.
        assert_eq!(
            unsafe { nva_app_attach_window(app, ptr::null_mut(), 1920, 1080, 1) },
            NVA_ERR_INVALID_ARGUMENT,
            "a NULL window is refused"
        );
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 0) },
            NVA_ERR_INVALID_ARGUMENT,
            "generation 0 is reserved for 'no surface yet'"
        );
        destroy(app);
    }

    #[test]
    fn detach_clears_the_window_and_notifies_the_shell() {
        let probe = Probe::default();
        let callbacks = callbacks(&probe);
        let app = create(&callbacks);

        // SAFETY: `app` is live and the generation matches what was attached.
        unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 1) };
        // SAFETY: as above.
        assert_eq!(unsafe { nva_app_detach_window(app, 1) }, NVA_OK);
        assert_eq!(probe.lost_calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.lost_generation.load(Ordering::SeqCst), 1);

        // A second detach has nothing to clear and must not raise the callback
        // again: the shell already released its reference.
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_detach_window(app, 1) },
            NVA_ERR_INVALID_ARGUMENT
        );
        assert_eq!(probe.lost_calls.load(Ordering::SeqCst), 1);

        destroy(app);
    }

    #[test]
    fn detach_refuses_a_generation_that_was_not_attached() {
        let probe = Probe::default();
        let callbacks = callbacks(&probe);
        let app = create(&callbacks);

        // SAFETY: `app` is live; 2 was never attached.
        unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 1920, 1080, 1) };
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_detach_window(app, 2) },
            NVA_ERR_INVALID_ARGUMENT,
            "a mismatched generation is a disagreement, not a no-op"
        );
        assert_eq!(
            probe.lost_calls.load(Ordering::SeqCst),
            0,
            "a refused detach must not tell the shell the surface is gone"
        );

        destroy(app);
    }

    #[test]
    fn lifecycle_rejects_the_transitions_android_never_makes() {
        let app = create(ptr::null());
        // SAFETY: `app` is live; the argument is the tested state, not a pointer.
        unsafe {
            assert_eq!(
                nva_app_pause(app),
                NVA_ERR_INVALID_ARGUMENT,
                "onPause before onResume does not happen"
            );
            assert_eq!(
                nva_app_resume(app),
                NVA_ERR_INVALID_ARGUMENT,
                "onResume before onStart does not happen"
            );
            assert_eq!(
                nva_app_stop(app),
                NVA_ERR_INVALID_ARGUMENT,
                "onStop before onStart does not happen"
            );
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(
                nva_app_start(app),
                NVA_ERR_INVALID_ARGUMENT,
                "onStart does not repeat without a pause or a stop"
            );
            assert_eq!(
                nva_app_pause(app),
                NVA_ERR_INVALID_ARGUMENT,
                "onPause does not arrive between onStart and onResume"
            );
            assert_eq!(nva_app_resume(app), NVA_OK);
            assert_eq!(
                nva_app_pause(app),
                NVA_OK,
                "onPause from the interactive state"
            );
            assert_eq!(nva_app_resume(app), NVA_OK, "onResume after onPause");
            assert_eq!(nva_app_stop(app), NVA_OK, "onStop from resumed");
            assert_eq!(
                nva_app_start(app),
                NVA_OK,
                "returning from the background calls onStart again"
            );
            assert_eq!(nva_app_stop(app), NVA_OK, "onStop straight from onStart");
        }
        destroy(app);
    }

    #[test]
    fn a_cold_start_runs_start_then_resume() {
        let app = create(ptr::null());
        // SAFETY: `app` is live; only the state machine is exercised.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(
                nva_app_resume(app),
                NVA_OK,
                "Android always calls onResume right after onStart; refusing it \
                 would log a failure on every launch"
            );
        }
        destroy(app);
    }

    #[test]
    fn stop_accepts_the_background_order_android_uses() {
        let app = create(ptr::null());
        // SAFETY: `app` is live; only the state machine is exercised.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(nva_app_resume(app), NVA_OK);
            assert_eq!(nva_app_pause(app), NVA_OK);
            assert_eq!(nva_app_stop(app), NVA_OK, "onPause then onStop");
        }
        destroy(app);
    }

    #[test]
    fn get_stats_zeroes_the_skeleton() {
        let app = create(ptr::null());
        let mut stats = nva_stats {
            size: size_of::<nva_stats>() as u32,
            version: NVA_STATS_VERSION,
            unsupported_calls: 7,
            unknown_resolves: 7,
            stub_aborts: 7,
            sticky_error: 7,
            last_serial: 7,
            flags: 7,
            frames: 7,
            draw_calls: 7,
            submits: 7,
        };
        // SAFETY: `app` is live and `stats` is a local of the right size.
        assert_eq!(unsafe { nva_app_get_stats(app, &mut stats) }, NVA_OK);
        assert_eq!(
            stats.frames + stats.draw_calls + stats.submits,
            0,
            "the skeleton presents nothing, so the counters are zero"
        );
        assert_eq!(stats.unsupported_calls, 0);
        assert_eq!(stats.sticky_error, 0);

        // A stale struct is refused before the first write, so the caller's
        // values survive intact rather than being half-overwritten.
        let mut stale = nva_stats {
            version: NVA_STATS_VERSION + 1,
            ..stats
        };
        stale.frames = 9;
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_get_stats(app, &mut stale) },
            NVA_ERR_INVALID_ARGUMENT
        );
        assert_eq!(stale.frames, 9, "a refused call writes nothing");

        // SAFETY: NULL is the documented null case.
        assert_eq!(
            unsafe { nva_app_get_stats(app, ptr::null_mut()) },
            NVA_ERR_INVALID_ARGUMENT
        );
        destroy(app);
    }

    #[test]
    fn every_export_accepts_a_null_handle() {
        // SAFETY: NULL is the documented null case for all of them.
        unsafe {
            assert_eq!(nva_app_start(ptr::null_mut()), NVA_ERR_INVALID_ARGUMENT);
            assert_eq!(nva_app_pause(ptr::null_mut()), NVA_ERR_INVALID_ARGUMENT);
            assert_eq!(nva_app_resume(ptr::null_mut()), NVA_ERR_INVALID_ARGUMENT);
            assert_eq!(nva_app_stop(ptr::null_mut()), NVA_ERR_INVALID_ARGUMENT);
            assert_eq!(
                nva_app_attach_window(ptr::null_mut(), 0x1000 as *mut c_void, 1, 1, 1),
                NVA_ERR_INVALID_ARGUMENT
            );
            assert_eq!(
                nva_app_detach_window(ptr::null_mut(), 1),
                NVA_ERR_INVALID_ARGUMENT
            );
            nva_app_destroy(ptr::null_mut());
        }
    }
}
