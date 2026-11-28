//! The shell-facing app object: the lifecycle and the window (`nva_app_*`).
//!
//! `docs/ARCHITECTURE.md` §3.7 makes the platform shell the owner of the
//! surface: it sees `surfaceCreated` / `surfaceChanged` / `surfaceDestroyed`,
//! keeps the generation counter, and forwards each event through this module.
//! §7.1 calls that the shell → runtime seam; this is its first real traffic.
//!
//! The module records what the shell reports — the lifecycle state, the window,
//! its size, the generation — and calls back into the shell when a window
//! arrives or leaves. On Android it also starts the render thread that builds
//! the surface/swapchain pair and presents the clear frame (#93); that engine
//! lives in `window.rs`, and this module is the only thing that starts and
//! stops it.
//!
//! ## Speaking to the shell
//!
//! Every transition and every refusal is one line through `nva_platform.log`
//! (`desc.platform`, borrowed until `destroy`), tagged `nva-runtime`. The shell
//! writes it to the same file as its own events, so one device run shows both
//! ends of this seam interleaved (`docs/DEVICE-LOGS.md` §8). The callback always
//! runs after the state guard is dropped: it reaches a file write, and holding
//! the mutex across it would deadlock the moment a shell called back into
//! `nva_app_*` from there. It runs synchronously, on whichever thread called the
//! export — never queued for later — so a shell that writes a file from it is
//! writing on the caller's thread. The render thread never calls it: what the
//! loop has to say is picked up by `attach` or `detach`, which run on that same
//! shell thread.
//!
//! A shell that passes no platform logs nothing, which is not an error: the
//! seam is optional (`desc.platform` may be NULL). Two silences are deliberate.
//! `nva_app_create`'s own refusals stay unlogged, because a create that
//! produced no object has no sink to speak through and the shell already logs
//! that failure itself. `nva_app_get_stats` logs nothing either: it is a query,
//! not an event, and logging one per frame would bury the events it sits among.
//!
//! ## Who owns the `ANativeWindow` reference
//!
//! The shell (`docs/ARCHITECTURE.md` §7.3): it acquires one reference before
//! `nva_app_attach_window` and releases it only after
//! `nva_app_detach_window` has returned. This module borrows the bare pointer
//! in between — it never dereferences it — and never touches the window after
//! detach returns. The render thread takes a second, independent reference of
//! its own at attach (§7.3) and releases it when the thread exits, which
//! `detach` waits for through the join.
//!
//! That split is safe because of what comes between the two calls: the runtime
//! borrows the window only while its record exists, and the record exists only
//! between a successful attach and a successful detach. There is no moment when
//! the shell has released a reference the runtime still points at, and none
//! when the runtime holds a pointer past a release.

use core::ffi::{c_char, c_void, CStr};
use core::mem::size_of;
use core::ptr;

use std::ffi::CString;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use nva_abi::{
    nva_app_callbacks, nva_app_desc, nva_platform, nva_stats, nva_status,
    NVA_APP_CALLBACKS_VERSION, NVA_APP_DESC_VERSION, NVA_ERR_INVALID_ARGUMENT, NVA_ERR_STALE,
    NVA_LOG_ERROR, NVA_LOG_INFO, NVA_LOG_WARN, NVA_OK, NVA_PLATFORM_VERSION, NVA_STATS_VERSION,
};

/// The presentation engine's types (`window.rs`, declared in `lib.rs`).
#[cfg(target_os = "android")]
use crate::window::{FirstFrame, RenderLoop, StartRequest, FIRST_FRAME_TIMEOUT};

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

impl Lifecycle {
    /// The word this state carries in a log line, which is how a refusal says
    /// which state it came from.
    fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Started => "started",
            Self::Resumed => "resumed",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
        }
    }
}

/// The tag on every line this module raises.
const TAG: &CStr = c"nva-runtime";

/// The tag on the stats snapshots (#94).
///
/// The shell hands this straight to logcat, so `logcat -d -s nva-stats`
/// selects exactly the snapshots and nothing else (`docs/DEVICE-LOGS.md` §9).
const NVA_STATS_TAG: &CStr = c"nva-stats";

/// The shell's log callback, copied out of `nva_platform`.
///
/// Only the two fields this module uses are copied, so the rest of the module
/// never has to reach back into the shell's struct. `log` is `None` when the
/// shell passed no platform or left the field NULL; both mean "say nothing".
#[derive(Clone, Copy)]
struct PlatformSink {
    user: *mut c_void,
    log: Option<unsafe extern "C" fn(*mut c_void, u32, *const c_char, *const c_char)>,
}

impl PlatformSink {
    /// A sink that raises nothing, for a shell that passes no platform.
    fn none() -> Self {
        Self {
            user: ptr::null_mut(),
            log: None,
        }
    }

    fn from_platform(platform: &nva_platform) -> Self {
        Self {
            user: platform.user,
            log: platform.log,
        }
    }

    /// One line to the shell at `level`.
    ///
    /// A message with an interior NUL is dropped rather than cut: every message
    /// here is built from literals and integers, so that arm is unreachable
    /// today, and dropping keeps a future format string from turning into a
    /// line that ends mid-sentence with no sign it was truncated.
    fn say(&self, level: u32, tag: &'static CStr, message: &str) {
        let Some(log) = self.log else {
            return;
        };
        let Ok(message) = CString::new(message) else {
            return;
        };
        // SAFETY: `log` is the shell's function, written into `nva_platform` and
        // called with the C signature from `include/nva_abi.h` (user, level,
        // tag, msg). `user` is the shell's own opaque pointer, unmodified. The
        // type says `tag: &'static CStr`, so its pointer stays valid for the
        // whole process; `message` is NUL-terminated by `CString` above and
        // outlives the call. The shell must not retain either pointer after
        // the call returns.
        unsafe { log(self.user, level, tag.as_ptr(), message.as_ptr()) };
    }
}

/// What one export decided under the lock, and the line it speaks after
/// dropping it.
///
/// Splitting the decision from the announcement is the point: the callback
/// cannot run while the state guard is alive (see the module doc), so each
/// export builds one of these inside the critical section and hands it to
/// `say` once the guard is gone.
struct Outcome {
    status: nva_status,
    level: u32,
    text: String,
}

impl Outcome {
    /// A transition that happened.
    fn done(text: impl Into<String>) -> Self {
        Self {
            status: NVA_OK,
            level: NVA_LOG_INFO,
            text: text.into(),
        }
    }

    /// A call that changed nothing, with the reason in `detail`.
    fn refused(status: nva_status, operation: &str, detail: impl Into<String>) -> Self {
        Self {
            status,
            level: NVA_LOG_ERROR,
            text: format!("{operation} refused: {}", detail.into()),
        }
    }
}

/// The window the shell attached, borrowed until it is detached.
///
/// `attach` records it, `detach` matches its generation against the caller's,
/// and the platform build hands the same four values to the render thread. The
/// pointer itself is never dereferenced here: the engine takes its own
/// `ANativeWindow` reference (`window.rs`), so this record is a token, not an
/// owner.
///
/// Only the platform build reads `native`, `width` and `height`; the host build
/// writes the record and never starts an engine, so those three are silenced
/// there rather than around the whole module.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
#[derive(Clone, Copy)]
struct AttachedWindow {
    native: *mut c_void,
    width: u32,
    height: u32,
    generation: u32,
}

struct State {
    lifecycle: Lifecycle,
    window: Option<AttachedWindow>,
    /// The render thread, while a window is attached. Only the platform build
    /// has one: the host build records the window and starts nothing, which is
    /// what the PC smoke tests rely on (`0x1000` is not a real window).
    #[cfg(target_os = "android")]
    render: Option<RenderLoop>,
}

/// What the render thread counts, shared with `nva_app_get_stats`.
///
/// Behind one `Arc` rather than three fields of `App` because the thread must not
/// hold a pointer into a value the shell owns by raw pointer: one allocation,
/// two owners, stable addresses.
///
/// `Relaxed` is sufficient by construction, and both readers already have a
/// happens-before edge to the increments. On attach, the loop calls
/// `record_present` before `Progress::presented`, which takes the mutex that
/// `Progress::wait` acquires — release/acquire on the same lock publishes the
/// count. On detach, `RenderLoop::stop_and_join` joins the thread that did the
/// increment. Nothing else is published alongside the counter, so no stronger
/// ordering buys anything.
#[derive(Default)]
pub(crate) struct RenderCounters {
    presents: AtomicU32,
    submits: AtomicU64,
    /// Present calls that began after `stop` was set (#94).
    ///
    /// Counted by the render thread, read once on the shell thread after the
    /// join. Same `Relaxed` argument as the other two: the join is the
    /// happens-before edge that publishes it.
    detached_submits: AtomicU32,
}

impl RenderCounters {
    /// Counts one frame that reached the display.
    ///
    /// Called by the platform build's render thread, which is why only this
    /// method and [`Self::record_submit`] are silenced on the host build.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn record_present(&self) {
        self.presents.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts one `vkQueueSubmit` the render loop made.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn record_submit(&self) {
        self.submits.fetch_add(1, Ordering::Relaxed);
    }

    /// The frames that reached the display.
    fn presents(&self) -> u32 {
        self.presents.load(Ordering::Relaxed)
    }

    /// The queue submits the render loop made.
    fn submits(&self) -> u64 {
        self.submits.load(Ordering::Relaxed)
    }

    /// Counts one present that found `stop` already set.
    ///
    /// Called by the render thread in `window.rs` when `detach` won the race
    /// with a frame in flight. The host build has no render thread; its tests
    /// call this directly to exercise the detach signal.
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    pub(crate) fn record_detached_submit(&self) {
        self.detached_submits.fetch_add(1, Ordering::Relaxed);
    }

    /// The presents that arrived after `stop` was set.
    pub(crate) fn detached_submits(&self) -> u32 {
        self.detached_submits.load(Ordering::Relaxed)
    }

    /// Clears the detached-submit count for the next attach (#94).
    ///
    /// The counter is process-wide. A rotation attaches the new window while
    /// the old loop exits, so without this reset the previous generation's
    /// detached submits would be reported by the signal a later detach raises
    /// for the new generation.
    pub(crate) fn reset_detached_submits(&self) {
        self.detached_submits.store(0, Ordering::Relaxed);
    }
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
/// assumption is written down in the type rather than in a comment: every
/// caller is the shell's main thread, which is why `App` is neither `Send` nor
/// `Sync` and stays that way. #93's render thread is given an owned job plus
/// these three shared counters (`window.rs`); it never holds an `&App`, so no
/// `unsafe impl Sync` is needed or wanted.
pub struct App {
    state: Mutex<State>,
    callbacks: SurfaceCallbacks,
    /// Where this object speaks. Behind no lock: it is copied at create and
    /// never changes, so reading it cannot race the state.
    platform: PlatformSink,
    /// The render thread's counters (`window.rs`).
    counters: Arc<RenderCounters>,
    /// The generation the last successful attach carried.
    ///
    /// Deliberately sticky: `detach` does not clear it, so `surface_generation`
    /// keeps naming the surface that last existed. #94 pairs it with its own
    /// `attached` flag to tell "no window now" from "the last one was N"; a
    /// value that fell back to 0 here would erase the second half of that.
    last_generation: AtomicU32,
}

impl App {
    /// Says one already-decided line and hands back its verdict.
    ///
    /// Every export ends here, which is what keeps the announcement outside the
    /// state guard: the guard is dropped where the `Outcome` was built, and this
    /// runs afterwards.
    fn announce(&self, outcome: &Outcome) -> nva_status {
        self.platform.say(outcome.level, TAG, &outcome.text);
        outcome.status
    }

    /// Says one stats snapshot through the shell (`docs/DEVICE-LOGS.md` §9).
    ///
    /// `attached` is decided by the call site rather than read back from the
    /// state: the detach snapshot is taken after the window record is gone, and
    /// a value read there would race the transition being reported.
    fn say_snapshot(&self, attached: bool) {
        self.platform
            .say(NVA_LOG_INFO, NVA_STATS_TAG, &self.stats_json(attached));
    }

    /// The one-line JSON snapshot #94 defines (`docs/DEVICE-LOGS.md` §9).
    ///
    /// Field order is part of the contract. The shell prefixes
    /// `[rt] info nva-stats: `, so the object itself must fit 284 B = 305 B
    /// (the message room after the 13-byte clock and the 2-byte reserve in
    /// `SHELL_LOG_LINE`) − 21 B of prefix; `stats_json_fits_the_shell_line`
    /// pins that bound. Serialising all twenty `nva_stats` fields here would
    /// cross it and the shell would silently truncate the line.
    ///
    /// `validation_available` and `validation_errors` are constant 0 in the
    /// product package: nothing in this ticket measures them and they do not
    /// gate M0b, so #96 must not read them as evidence of a clean validation
    /// run.
    fn stats_json(&self, attached: bool) -> String {
        let generation = self.last_generation.load(Ordering::Relaxed);
        format!(
            "{{\"presents\":{},\"surface_generation\":{},\"attached\":{},\"generation\":{},\"submits\":{},\"validation_available\":0,\"validation_errors\":0}}",
            self.counters.presents(),
            generation,
            attached,
            generation,
            self.counters.submits()
        )
    }
}

/// Starting, stopping and reporting the render thread.
///
/// Every method here runs on the shell's thread, between the state transitions
/// in `attach`, `detach` and `destroy`. The loop itself is joined here and
/// nowhere else: detach must not return while a submit could still be in
/// flight, and the shell releases its `ANativeWindow` reference as soon as it
/// does.
impl App {
    /// Says the notes one stage of the render loop left behind, each on its own
    /// line, prefixed with the call that is reporting them.
    ///
    /// `when` is `attach`, `detach` or `destroy`: the notes carry only what
    /// happened (`suboptimal`, `out of date`, why the loop stopped), so the
    /// moment they are printed is what puts them in a run's story.
    fn say_notes(&self, when: &str, notes: Vec<String>) {
        for note in notes {
            self.platform
                .say(NVA_LOG_WARN, TAG, &format!("{when}: {note}"));
        }
    }

    /// Stops the render thread, if one is running, and hands back what it noted.
    ///
    /// Off the platform build there is no thread — `attach` only ever recorded
    /// the window — so this is empty and the call sites stay identical.
    #[cfg(not(target_os = "android"))]
    fn stop_render_loop(&self) -> Vec<String> {
        Vec::new()
    }

    #[cfg(target_os = "android")]
    fn stop_render_loop(&self) -> Vec<String> {
        let render = match lock(self) {
            Ok(mut state) => state.render.take(),
            // The state lock is poisoned, which `panic = "abort"` makes
            // unreachable in a product build (`lock`). Nothing is left that
            // could be joined safely, so the call site finds no loop.
            Err(_) => None,
        };
        render.map(RenderLoop::stop_and_join).unwrap_or_default()
    }

    /// Starts the render thread for a window `attach` just recorded.
    ///
    /// A replaced window's loop is stopped first: on a rotation the old loop
    /// must be joined before the shell releases its reference to the old
    /// window, and two loops presenting into two windows is never wanted.
    #[cfg(target_os = "android")]
    fn start_render_loop(&self, window: AttachedWindow) {
        self.say_notes("attach", self.stop_render_loop());
        let request = StartRequest {
            native: window.native,
            width: window.width,
            height: window.height,
            generation: window.generation,
            counters: Arc::clone(&self.counters),
        };
        let render = match crate::window::start(request) {
            // A zero-sized surface: the window is recorded, and the next attach
            // (surfaceChanged) carries the measurement.
            Ok(None) => return,
            Ok(Some(render)) => render,
            Err(reason) => {
                // No thread, no frames: the snapshot stays at `presents = 0`,
                // which is what the acceptance check reads.
                self.platform
                    .say(NVA_LOG_ERROR, TAG, &format!("attach: {reason}"));
                return;
            }
        };
        let progress = render.progress();
        match lock(self) {
            Ok(mut state) => state.render = Some(render),
            Err(_) => {
                // Unreachable under `panic = "abort"` (`lock`). Join now rather
                // than leave a thread presenting into a window nothing will
                // ever detach.
                let _ = render.stop_and_join();
                self.platform
                    .say(NVA_LOG_ERROR, TAG, "attach: the state lock is poisoned");
                return;
            }
        }
        match progress.wait(FIRST_FRAME_TIMEOUT) {
            // Nothing to say: `presents` in the snapshot is the evidence.
            FirstFrame::Presented => {}
            // A timeout is not a failure (`window.rs`), so it is not an error
            // line either: the loop keeps presenting and `presents` will say so.
            FirstFrame::Pending => self.platform.say(
                NVA_LOG_WARN,
                TAG,
                &format!("attach: first frame not ready in {FIRST_FRAME_TIMEOUT:?}"),
            ),
            FirstFrame::Failed(reason) => {
                self.platform
                    .say(NVA_LOG_ERROR, TAG, &format!("attach: {reason}"))
            }
        }
        self.say_notes("attach", progress.take_notes());
    }
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
// `NVA_ERR_INVALID_ARGUMENT` instead of being dereferenced. `desc.callbacks` and
// `desc.platform`, when non-NULL, have their own `size` / `version` checked
// before they are read. `data_dir` is not stored, and the callback pointers are
// copied. Of `desc.platform` only `user` and `log` are copied, but the header's
// contract still holds: both must stay valid until `nva_app_destroy`, because
// `user` is handed back to `log` on every later line. On success `*out` owns the
// boxed `App`, and only `nva_app_destroy` may free it.
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

    let platform = if desc.platform.is_null() {
        PlatformSink::none()
    } else {
        // SAFETY: non-NULL, so the shell is handing over a platform struct; its
        // `size` / `version` are checked before any field is read, so a stale
        // struct is refused rather than misread. Only `user` and `log` are
        // copied, and no other slot is touched — the audio, rumble and clock
        // entries stay unimplemented until the tickets that need them.
        let platform = unsafe { &*desc.platform };
        if platform.size < size_of::<nva_platform>() as u32
            || platform.version != NVA_PLATFORM_VERSION
        {
            return NVA_ERR_INVALID_ARGUMENT;
        }
        PlatformSink::from_platform(platform)
    };

    let app = Box::new(App {
        state: Mutex::new(State {
            lifecycle: Lifecycle::Created,
            window: None,
            #[cfg(target_os = "android")]
            render: None,
        }),
        callbacks,
        platform,
        counters: Arc::new(RenderCounters::default()),
        last_generation: AtomicU32::new(0),
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
// once here; NULL is accepted and does nothing. The shell owns the
// `ANativeWindow` reference it took (§7.3) and releases it itself; the render
// thread's own reference is released by the job's drop, which the join below
// waits for, so nothing outlives this call holding the window.
#[no_mangle]
pub unsafe extern "C" fn nva_app_destroy(app: *mut App) {
    if app.is_null() {
        return;
    }
    let app = Box::from_raw(app);
    // A destroy with a window still attached is a shell that skipped
    // `surfaceDestroyed`. The loop must still be joined here: it is the last
    // moment anything can present into that window.
    app.say_notes("destroy", app.stop_render_loop());
    app.platform.say(NVA_LOG_INFO, TAG, "destroy");
    drop(app);
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
    let app = &*app;
    // The guard drops at the end of this statement, before anything is said:
    // the shell's callback writes a file and may call back into this module.
    let outcome = match lock(app) {
        Ok(mut state) => {
            let from = state.lifecycle;
            match from {
                Lifecycle::Created | Lifecycle::Stopped => {
                    state.lifecycle = Lifecycle::Started;
                    Outcome::done("start")
                }
                Lifecycle::Started | Lifecycle::Resumed | Lifecycle::Paused => Outcome::refused(
                    NVA_ERR_INVALID_ARGUMENT,
                    "start",
                    format!("no transition from {}", from.name()),
                ),
            }
        }
        Err(status) => Outcome::refused(status, "start", "the state lock is poisoned"),
    };
    app.announce(&outcome)
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
    let app = &*app;
    let outcome = match lock(app) {
        Ok(mut state) => {
            let from = state.lifecycle;
            match from {
                Lifecycle::Resumed => {
                    state.lifecycle = Lifecycle::Paused;
                    Outcome::done("pause")
                }
                Lifecycle::Created
                | Lifecycle::Started
                | Lifecycle::Paused
                | Lifecycle::Stopped => Outcome::refused(
                    NVA_ERR_INVALID_ARGUMENT,
                    "pause",
                    format!("no transition from {}", from.name()),
                ),
            }
        }
        Err(status) => Outcome::refused(status, "pause", "the state lock is poisoned"),
    };
    app.announce(&outcome)
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
    let app = &*app;
    let outcome = match lock(app) {
        Ok(mut state) => {
            let from = state.lifecycle;
            match from {
                Lifecycle::Started | Lifecycle::Paused => {
                    state.lifecycle = Lifecycle::Resumed;
                    Outcome::done("resume")
                }
                Lifecycle::Created | Lifecycle::Resumed | Lifecycle::Stopped => Outcome::refused(
                    NVA_ERR_INVALID_ARGUMENT,
                    "resume",
                    format!("no transition from {}", from.name()),
                ),
            }
        }
        Err(status) => Outcome::refused(status, "resume", "the state lock is poisoned"),
    };
    app.announce(&outcome)
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
    let app = &*app;
    let outcome = match lock(app) {
        Ok(mut state) => {
            let from = state.lifecycle;
            match from {
                Lifecycle::Started | Lifecycle::Resumed | Lifecycle::Paused => {
                    state.lifecycle = Lifecycle::Stopped;
                    Outcome::done("stop")
                }
                Lifecycle::Created | Lifecycle::Stopped => Outcome::refused(
                    NVA_ERR_INVALID_ARGUMENT,
                    "stop",
                    format!("no transition from {}", from.name()),
                ),
            }
        }
        Err(status) => Outcome::refused(status, "stop", "the state lock is poisoned"),
    };
    app.announce(&outcome)
}

/// Record the window the shell just got (`nva_app_attach_window`).
///
/// A generation of 0 is refused: the shell numbers surfaces from 1, and 0 is
/// what a variable that was never set to a surface holds.
///
/// Attaching while a window is already attached replaces the record. The shell
/// emits that on a size change (rotation), where the new geometry supersedes the
/// old one and the generation has advanced; it is not a detach, so
/// `on_surface_lost` is not raised for the window being replaced. The render
/// thread of the replaced window is stopped and joined before the new one
/// starts.
///
/// The geometry is recorded exactly as reported, including a zero on either
/// axis. Refusing that would move a decision into the ABI that belongs to the
/// shell, which is the one place that knows whether a surface is momentarily
/// unmeasured. A zero-sized record starts no render thread (`window.rs`); the
/// next attach, from `surfaceChanged`, carries the measurement.
///
/// # Safety
/// `app` must come from `nva_app_create`, and `native_window` must stay valid
/// until the matching `nva_app_detach_window` returns.
// SAFETY: `app` is null-checked and borrowed for the call. `native_window` is
// stored as an opaque value and handed to the render thread, which takes its
// own `ANativeWindow` reference before it uses it (§7.3) — so the shell's
// reference, which outlives the record, is what keeps the pointer valid here.
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
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let app = &*app;
    // Only the platform build starts an engine; the host build records and
    // returns, which is what the PC tests' `0x1000` sentinel needs.
    #[cfg(target_os = "android")]
    let mut recorded: Option<AttachedWindow> = None;
    let outcome = if native_window.is_null() {
        Outcome::refused(NVA_ERR_INVALID_ARGUMENT, "attach", "the window is NULL")
    } else if generation == 0 {
        Outcome::refused(
            NVA_ERR_INVALID_ARGUMENT,
            "attach",
            "generation 0 is reserved for 'no surface yet'",
        )
    } else {
        match lock(app) {
            Ok(mut state) => {
                let window = AttachedWindow {
                    native: native_window,
                    width,
                    height,
                    generation,
                };
                state.window = Some(window);
                #[cfg(target_os = "android")]
                {
                    recorded = Some(window);
                }
                Outcome::done(format!("attach generation={generation} {width}x{height}"))
            }
            Err(status) => Outcome::refused(status, "attach", "the state lock is poisoned"),
        }
    };
    if app.announce(&outcome) != NVA_OK {
        return outcome.status;
    }
    // Sticky: `detach` does not clear it. Recorded only for an attach that
    // actually happened, so a refused call cannot move it.
    app.last_generation.store(generation, Ordering::Relaxed);
    #[cfg(target_os = "android")]
    if let Some(window) = recorded {
        app.start_render_loop(window);
    }
    // #94: a rotation joins the old loop before the new one starts, so any
    // submit that raced that stop belonged to the previous generation. Clear
    // it here or the final detach's signal would name a generation that never
    // submitted after its own stop.
    app.counters.reset_detached_submits();
    // #94: the attach snapshot is the first of the contract's three moments.
    // It follows `start_render_loop` so `presents` is already the value the
    // first frame left (on Android, attach waited for it), and it precedes
    // `ready` so a shell that reads the log from inside that callback sees it.
    app.say_snapshot(true);
    let callbacks = app.callbacks;
    if let Some(ready) = callbacks.ready {
        // SAFETY: `ready` is a shell function the shell itself wrote into
        // `nva_app_callbacks`; its contract is the C signature in
        // `include/nva_abi.h` (user, generation, width, height), which is what
        // is passed. `user` is the shell's own opaque pointer, unmodified.
        //
        // Raised last on purpose: a shell that reads the snapshot from inside
        // this callback sees the first frame's `presents`, because `attach`
        // waited for it above.
        unsafe { ready(callbacks.user, generation, width, height) };
    }
    NVA_OK
}

/// Forget the window (`nva_app_detach_window`).
///
/// Called with the generation the shell attached. A different generation means
/// the two sides disagree about which window is live, which is the one state
/// worth failing loudly: silently clearing would leave the shell holding a
/// reference for a window this module no longer knows about. A refused detach
/// leaves the render thread running with it — the record, the loop and the two
/// references stay together.
///
/// A successful detach stops and joins the render thread before returning, so
/// nothing can submit for this window afterwards; that is what makes the
/// shell's release of its own reference safe (§7.3).
///
/// # Safety
/// `app` must come from `nva_app_create`.
// SAFETY: `app` is null-checked and borrowed for the call. The window record is
// dropped before the callback runs, and the stored `native_window` is not
// touched again — not read, not released by this module — so the shell is free
// to release its reference as soon as this call returns (§7.3). The engine's
// own reference is released by the render job as the joined thread exits.
#[no_mangle]
pub unsafe extern "C" fn nva_app_detach_window(app: *mut App, generation: u32) -> nva_status {
    if app.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let app = &*app;
    let outcome = match lock(app) {
        Ok(mut state) => {
            // The refusal reason is built while the record is still borrowed,
            // and the write below happens after that borrow ends.
            let refusal = match state.window.as_ref() {
                Some(window) if window.generation == generation => None,
                Some(window) => Some(format!(
                    "generation {generation} is not the attached one ({})",
                    window.generation
                )),
                None => Some(String::from("no window is attached")),
            };
            match refusal {
                None => {
                    state.window = None;
                    Outcome::done(format!("detach generation={generation}"))
                }
                Some(detail) => Outcome::refused(NVA_ERR_INVALID_ARGUMENT, "detach", detail),
            }
        }
        Err(status) => Outcome::refused(status, "detach", "the state lock is poisoned"),
    };
    // Only a detach that happened takes the loop down; a refusal must leave it
    // presenting into the window the record still names.
    let notes = if outcome.status == NVA_OK {
        app.stop_render_loop()
    } else {
        Vec::new()
    };
    if app.announce(&outcome) != NVA_OK {
        return outcome.status;
    }
    app.say_notes("detach", notes);
    // #94: the counts are frozen — the loop above was joined — and `attached`
    // is false because this call is the transition that made it so.
    app.say_snapshot(false);
    // A submit that raced the stop is its own signal, not an inference from
    // the snapshot counters. The join above means no later submit can change
    // the count between this read and the line.
    let detached_submits = app.counters.detached_submits();
    if detached_submits > 0 {
        app.platform.say(
            NVA_LOG_INFO,
            NVA_STATS_TAG,
            &format!(
                "{{\"event\":\"detached_submit\",\"count\":{detached_submits},\"generation\":{generation}}}"
            ),
        );
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
/// #93 gives three of the v2 fields a producer: `presents` and `submits` are the
/// render thread's counters, and `surface_generation` is the last generation an
/// attach carried — sticky, so a detach does not reset it. That is deliberate:
/// #94 pairs it with an `attached` flag of its own, and a value that fell back
/// to 0 here would erase which surface last existed.
///
/// Every field of `nva_stats` except `size` and `version` is written. That is
/// load-bearing: the v2 append (#90) added nine counters, and an implementation
/// that stopped at the v1 tail would leave the caller reading its own memory
/// back as counts. `get_stats_writes_every_field` pre-fills all of them, so a
/// field added without a writer here fails that test.
///
/// # Safety
/// `app` must come from `nva_app_create` and `out` must point to a writable
/// `nva_stats` whose `size` and `version` are already initialised.
// SAFETY: `app` is null-checked and borrowed only to read atomics; `out` is
// shell-owned and must stay writable for the call. `size` / `version` are read
// before the first write, so an older or shorter struct is refused with
// `NVA_ERR_INVALID_ARGUMENT` instead of overrun. Nothing is retained: the
// snapshot is copied out and both borrows end with the call.
#[no_mangle]
pub unsafe extern "C" fn nva_app_get_stats(app: *mut App, out: *mut nva_stats) -> nva_status {
    if app.is_null() || out.is_null() {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let out = &mut *out;
    if out.size < size_of::<nva_stats>() as u32 || out.version != NVA_STATS_VERSION {
        return NVA_ERR_INVALID_ARGUMENT;
    }
    let app = &*app;
    out.unsupported_calls = 0;
    out.unknown_resolves = 0;
    out.stub_aborts = 0;
    out.sticky_error = 0;
    out.last_serial = 0;
    out.flags = 0;
    out.frames = 0;
    out.draw_calls = 0;
    out.submits = app.counters.submits();
    // v2 counters: this ticket produces the first two, #96 the rest. The ones
    // without a producer are zeroed for the same reason `nva_gfx_get_stats`
    // zeroes them — a caller that did not initialise its struct must never read
    // its own memory back as a count.
    out.presents = app.counters.presents();
    out.surface_generation = app.last_generation.load(Ordering::Relaxed);
    out.missing_variants = 0;
    out.draw_skipped = 0;
    out.xruns = 0;
    out.input_events = 0;
    out.input_consumed = 0;
    out.guest_svc_count = 0;
    out.draw_caller_in_guest = 0;
    NVA_OK
}

#[cfg(test)]
mod tests {
    use core::ffi::{c_char, c_void, CStr};
    use core::mem::size_of;
    use core::ptr;

    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    use nva_abi::{
        nva_app_callbacks, nva_app_desc, nva_platform, nva_stats, NVA_APP_CALLBACKS_VERSION,
        NVA_APP_DESC_VERSION, NVA_ERR_INVALID_ARGUMENT, NVA_LOG_ERROR, NVA_LOG_INFO, NVA_OK,
        NVA_PLATFORM_VERSION, NVA_STATS_VERSION,
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

    /// One line the runtime said, as the shell's callback received it.
    struct Logged {
        level: u32,
        tag: String,
        text: String,
    }

    /// What the shell's `log` callback records. Each test owns one and passes it
    /// as `user`, so the tests capture only their own lines and can still run in
    /// parallel.
    #[derive(Default)]
    struct LogProbe {
        lines: Mutex<Vec<Logged>>,
    }

    impl LogProbe {
        /// Takes the lines said so far, leaving the probe empty.
        fn take(&self) -> Vec<Logged> {
            let mut lines = self.lines.lock().expect(
                "the capture mutex is only poisoned by a test that panicked while it held it",
            );
            core::mem::take(&mut *lines)
        }
    }

    unsafe extern "C" fn on_platform_log(
        user: *mut c_void,
        level: u32,
        tag: *const c_char,
        message: *const c_char,
    ) {
        // SAFETY: every test passes `user` as a pointer to a `LogProbe` that
        // outlives the app it belongs to, and the runtime only reaches this
        // callback through that app. `tag` and `message` are NUL-terminated
        // strings the runtime keeps alive for the duration of the call.
        let probe = unsafe { &*(user as *const LogProbe) };
        let tag = unsafe { CStr::from_ptr(tag) }
            .to_string_lossy()
            .into_owned();
        let text = unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned();
        probe
            .lines
            .lock()
            .expect("as `LogProbe::take`")
            .push(Logged { level, tag, text });
    }

    /// A platform whose only implemented entry is `log`.
    fn log_platform(probe: &LogProbe) -> nva_platform {
        nva_platform {
            size: size_of::<nva_platform>() as u32,
            version: NVA_PLATFORM_VERSION,
            user: probe as *const LogProbe as *mut c_void,
            audio_open: None,
            audio_write: None,
            audio_flush: None,
            audio_close: None,
            audio_set_volume: None,
            rumble: None,
            clock_now_ns: None,
            log: Some(on_platform_log),
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

    /// A snapshot with the header filled in and every count at 0.
    fn empty_stats() -> nva_stats {
        nva_stats {
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
            presents: 0,
            surface_generation: 0,
            missing_variants: 0,
            draw_skipped: 0,
            xruns: 0,
            input_events: 0,
            input_consumed: 0,
            guest_svc_count: 0,
            draw_caller_in_guest: 0,
        }
    }

    /// Creates an app and returns it with the guard that destroys it.
    fn create(callbacks: *const nva_app_callbacks) -> *mut App {
        create_with_platform(callbacks, ptr::null())
    }

    /// As `create`, with a platform the runtime may speak through.
    fn create_with_platform(
        callbacks: *const nva_app_callbacks,
        platform: *const nva_platform,
    ) -> *mut App {
        let desc = nva_app_desc {
            platform,
            ..desc(callbacks)
        };
        let mut app: *mut App = ptr::null_mut();
        // SAFETY: both pointers are to locals of this function, and `platform`
        // outlives the app in every test that passes one.
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
    fn get_stats_writes_every_field() {
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
            presents: 7,
            surface_generation: 7,
            missing_variants: 7,
            draw_skipped: 7,
            xruns: 7,
            input_events: 7,
            input_consumed: 7,
            guest_svc_count: 7,
            draw_caller_in_guest: 7,
        };
        // SAFETY: `app` is live and `stats` is a local of the right size.
        assert_eq!(unsafe { nva_app_get_stats(app, &mut stats) }, NVA_OK);
        assert_eq!(
            stats.frames + stats.draw_calls + stats.submits,
            0,
            "nothing is rendered, so the counters are zero"
        );
        assert_eq!(
            (
                stats.unsupported_calls,
                stats.unknown_resolves,
                stats.stub_aborts,
                stats.sticky_error,
                stats.last_serial,
                stats.flags,
            ),
            (0u32, 0, 0, 0, 0, 0),
            "every v1 counter is written, not left at the caller's 7"
        );
        assert_eq!(
            (
                stats.presents,
                stats.missing_variants,
                stats.draw_skipped,
                stats.xruns,
                stats.input_events,
                stats.input_consumed,
                stats.guest_svc_count,
                stats.draw_caller_in_guest,
            ),
            (0u32, 0, 0, 0, 0, 0, 0, 0),
            "every v2 counter is written too, so a field added to nva_stats \
             without a writer here fails this assertion instead of leaking"
        );
        assert_eq!(
            stats.surface_generation, 0,
            "no window is attached, so the generation is 0"
        );
        assert_eq!(stats.size, size_of::<nva_stats>() as u32);
        assert_eq!(stats.version, NVA_STATS_VERSION);

        // With a window attached the generation is reported as it stands. This
        // module counts it, so leaving 0 here would be a lie, not a placeholder.
        // SAFETY: the pointer is a token the runtime stores without touching.
        assert_eq!(
            unsafe { nva_app_attach_window(app, 0x1000 as *mut c_void, 640, 480, 3) },
            NVA_OK
        );
        // SAFETY: as above.
        assert_eq!(unsafe { nva_app_get_stats(app, &mut stats) }, NVA_OK);
        assert_eq!(
            stats.surface_generation, 3,
            "the attached window's generation is reported"
        );

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
    fn surface_generation_is_sticky_across_detach() {
        let probe = Probe::default();
        let callbacks = callbacks(&probe);
        let app = create(&callbacks);
        let mut snapshot = empty_stats();

        // SAFETY: `app` is live, and the window pointer is a token the runtime
        // stores without dereferencing on this target.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 640, 480, 3),
                NVA_OK
            );
            assert_eq!(nva_app_get_stats(app, &mut snapshot), NVA_OK);
            assert_eq!(snapshot.surface_generation, 3);

            assert_eq!(nva_app_detach_window(app, 3), NVA_OK);
            assert_eq!(nva_app_get_stats(app, &mut snapshot), NVA_OK);
        }
        assert_eq!(
            snapshot.surface_generation, 3,
            "detach keeps the last generation on purpose: #94 tells 'no window \
             now' from 'which one last' with a flag of its own, and a 0 here \
             would erase the second half"
        );
        assert_eq!(
            snapshot.presents, 0,
            "the host build records the window and starts no render thread"
        );
        assert_eq!(snapshot.submits, 0);

        destroy(app);
    }

    #[test]
    fn a_zero_sized_window_is_recorded_and_presents_nothing() {
        let app = create(ptr::null());
        let mut snapshot = empty_stats();

        // `surfaceCreated` can arrive before the view is measured. This test
        // pins the *record* semantics — the window is stored, its generation is
        // reported, and no frame is counted — which is all the host build can
        // see; the zero-size guard that refuses to build a 0x0 swapchain lives
        // in `window.rs` and only exists on the Android target.
        // SAFETY: `app` is live; the pointer is opaque to the runtime.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 0, 0, 1),
                NVA_OK
            );
            assert_eq!(nva_app_get_stats(app, &mut snapshot), NVA_OK);
            assert_eq!(nva_app_detach_window(app, 1), NVA_OK);
        }
        assert_eq!(snapshot.presents, 0, "no loop, no frames");
        assert_eq!(
            snapshot.surface_generation, 1,
            "the window was still recorded, so its generation is still reported"
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

    #[test]
    fn runtime_detach_snapshot_reports_attached_false() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; the window pointer is opaque to the runtime
        // and the host build never dereferences it.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 640, 480, 3),
                NVA_OK
            );
            assert_eq!(nva_app_detach_window(app, 3), NVA_OK);
        }
        destroy(app);

        let lines = logs.take();
        let snapshots: Vec<&Logged> = lines
            .iter()
            .filter(|line| line.tag == "nva-stats")
            .collect();
        assert_eq!(
            snapshots.len(),
            2,
            "one snapshot at attach and one at detach, got {}",
            snapshots.len()
        );
        assert!(
            snapshots[0].text.contains("\"attached\":true"),
            "the attach snapshot says a window is attached: {}",
            snapshots[0].text
        );
        let detach = snapshots[1];
        assert!(
            detach.text.contains("\"attached\":false"),
            "the detach snapshot must report attached=false: {}",
            detach.text
        );
        assert!(
            detach.text.contains("\"surface_generation\":3"),
            "detach keeps the generation the last attach carried: {}",
            detach.text
        );
        assert!(
            detach.text.contains("\"generation\":3"),
            "the sticky generation survives detach: {}",
            detach.text
        );
    }

    #[test]
    fn runtime_detached_submit_signal_names_the_count() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; the window pointer is opaque to the runtime
        // and the host build never dereferences it.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 640, 480, 4),
                NVA_OK
            );
        }
        // Inject one submit that arrived after the loop was stopped. On Android
        // `window.rs` counts this from the render thread; the host build has no
        // thread, so the test reaches the same private counter directly.
        // SAFETY: `app` is live and the borrow ends before `destroy`.
        let app_ref = unsafe { &*app };
        app_ref.counters.record_detached_submit();
        // SAFETY: `app` is live and 4 is the generation attached above.
        assert_eq!(unsafe { nva_app_detach_window(app, 4) }, NVA_OK);
        destroy(app);

        let lines = logs.take();
        let signal = lines
            .iter()
            .find(|line| {
                line.tag == "nva-stats" && line.text.contains("\"event\":\"detached_submit\"")
            })
            .expect("a detached submit must raise its own signal line");
        assert!(
            signal.text.contains("\"count\":1"),
            "the signal names the number of detached submits: {}",
            signal.text
        );
        assert!(
            signal.text.contains("\"generation\":4"),
            "the signal names the generation whose window was detached: {}",
            signal.text
        );
    }

    #[test]
    fn attach_clears_detached_submit_count() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; the window pointer is opaque to the runtime.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 640, 480, 1),
                NVA_OK
            );
        }
        // A rotation's old loop can race its stop; that count belongs to the
        // previous generation and must not leak into the next signal.
        // SAFETY: `app` is live and the borrow ends before `destroy`.
        let app_ref = unsafe { &*app };
        app_ref.counters.record_detached_submit();
        // SAFETY: as above; generation 2 replaces generation 1.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x2000 as *mut c_void, 1080, 1920, 2),
                NVA_OK
            );
            assert_eq!(nva_app_detach_window(app, 2), NVA_OK);
        }
        destroy(app);

        let lines = logs.take();
        assert!(
            !lines.iter().any(|line| {
                line.tag == "nva-stats" && line.text.contains("\"event\":\"detached_submit\"")
            }),
            "the previous generation's detached submit must be cleared on re-attach"
        );
    }

    #[test]
    fn stats_json_fits_the_shell_line() {
        let app = create(ptr::null());
        // SAFETY: `app` is live; the borrow ends before `destroy`.
        let app_ref = unsafe { &*app };
        // Worst case: the three numeric fields at their type maximum and
        // `attached:false` (the longer spelling).
        app_ref.counters.presents.store(u32::MAX, Ordering::Relaxed);
        app_ref.counters.submits.store(u64::MAX, Ordering::Relaxed);
        app_ref.last_generation.store(u32::MAX, Ordering::Relaxed);
        let worst = app_ref.stats_json(false);
        assert!(
            worst.len() <= 284,
            "the object must fit 284 B = 305 B of message room − 21 B of \
             `[rt] info nva-stats: `; got {} bytes: {worst}",
            worst.len()
        );
        destroy(app);
    }

    #[test]
    fn a_cold_start_records_each_transition_once() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live and `1` is a valid generation; the window
        // pointer is opaque to the runtime and never dereferenced.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(nva_app_resume(app), NVA_OK);
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 1080, 1920, 1),
                NVA_OK
            );
        }
        destroy(app);

        // The stats snapshots have their own tag and their own test; these
        // assertions are about the event lines, which all carry `TAG`.
        let lines: Vec<Logged> = logs
            .take()
            .into_iter()
            .filter(|line| line.tag == "nva-runtime")
            .collect();
        let said: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "start",
                "resume",
                "attach generation=1 1080x1920",
                "destroy"
            ],
            "one info line per transition, in the order they happened"
        );
        for line in &lines {
            assert_eq!(
                line.level, NVA_LOG_INFO,
                "transitions are info, got {}: {}",
                line.level, line.text
            );
        }
    }

    #[test]
    fn a_background_cycle_records_every_word_once() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; `1` is a valid generation and the window
        // pointer is opaque to the runtime and never dereferenced.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(nva_app_resume(app), NVA_OK);
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 1080, 1920, 1),
                NVA_OK
            );
            assert_eq!(nva_app_pause(app), NVA_OK);
            assert_eq!(nva_app_resume(app), NVA_OK);
            assert_eq!(nva_app_stop(app), NVA_OK);
            assert_eq!(nva_app_detach_window(app, 1), NVA_OK);
        }
        destroy(app);

        let lines: Vec<Logged> = logs
            .take()
            .into_iter()
            .filter(|line| line.tag == "nva-runtime")
            .collect();
        let said: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(
            said,
            [
                "start",
                "resume",
                "attach generation=1 1080x1920",
                "pause",
                "resume",
                "stop",
                "detach generation=1",
                "destroy"
            ],
            "each of the seven transitions says its own word exactly once, in order"
        );
        for line in &lines {
            assert_eq!(
                line.level, NVA_LOG_INFO,
                "every transition is info, got {}: {}",
                line.level, line.text
            );
        }
    }

    #[test]
    fn a_refused_transition_is_an_error_line() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; the second start is the refusal under test.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            logs.take();
            assert_eq!(
                nva_app_start(app),
                NVA_ERR_INVALID_ARGUMENT,
                "the refusal keeps the status it had before #101"
            );
        }
        destroy(app);

        let lines = logs.take();
        assert_eq!(
            lines.len(),
            2,
            "only the refusal and the destroy are said, got {}",
            lines.len()
        );
        assert_eq!(lines[0].level, NVA_LOG_ERROR, "a refusal is an error line");
        assert_eq!(
            lines[0].text, "start refused: no transition from started",
            "a refusal names the call and the state it came from"
        );
        assert_eq!(lines[1].text, "destroy");
    }

    #[test]
    fn a_refused_attach_names_its_reason() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: both arguments are the invalid ones the export documents.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, ptr::null_mut(), 1080, 1920, 1),
                NVA_ERR_INVALID_ARGUMENT
            );
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 1080, 1920, 0),
                NVA_ERR_INVALID_ARGUMENT
            );
        }
        destroy(app);

        let lines = logs.take();
        assert_eq!(
            lines[0].text, "attach refused: the window is NULL",
            "a NULL window says which argument was wrong"
        );
        assert_eq!(lines[0].level, NVA_LOG_ERROR);
        assert_eq!(
            lines[1].text,
            "attach refused: generation 0 is reserved for 'no surface yet'"
        );
        assert_eq!(lines[1].level, NVA_LOG_ERROR);
    }

    #[test]
    fn a_mismatched_detach_generation_is_an_error_line() {
        let logs = LogProbe::default();
        let platform = log_platform(&logs);
        let app = create_with_platform(ptr::null(), &platform);

        // SAFETY: `app` is live; 2 was never attached.
        unsafe {
            assert_eq!(
                nva_app_attach_window(app, 0x1000 as *mut c_void, 1080, 1920, 1),
                NVA_OK
            );
            assert_eq!(
                nva_app_detach_window(app, 2),
                NVA_ERR_INVALID_ARGUMENT,
                "a mismatched generation is still refused with the same code"
            );
        }
        destroy(app);

        let lines: Vec<Logged> = logs
            .take()
            .into_iter()
            .filter(|line| line.tag == "nva-runtime")
            .collect();
        assert_eq!(lines[1].level, NVA_LOG_ERROR);
        assert_eq!(
            lines[1].text, "detach refused: generation 2 is not the attached one (1)",
            "both generations appear, so the line says which side is wrong"
        );
    }

    #[test]
    fn a_malformed_platform_is_refused() {
        let logs = LogProbe::default();
        let mut app: *mut App = ptr::null_mut();

        let stale = nva_platform {
            version: NVA_PLATFORM_VERSION + 1,
            ..log_platform(&logs)
        };
        let stale_desc = nva_app_desc {
            platform: &stale,
            ..desc(ptr::null())
        };
        // SAFETY: `stale_desc` points at locals that outlive the call.
        assert_eq!(
            unsafe { nva_app_create(&stale_desc, &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a platform with an unknown version must not be used"
        );
        assert!(app.is_null(), "a refused create leaves the out slot NULL");

        let short = nva_platform {
            size: (size_of::<nva_platform>() - 8) as u32,
            ..log_platform(&logs)
        };
        let short_desc = nva_app_desc {
            platform: &short,
            ..desc(ptr::null())
        };
        // SAFETY: as above.
        assert_eq!(
            unsafe { nva_app_create(&short_desc, &mut app) },
            NVA_ERR_INVALID_ARGUMENT,
            "a platform shorter than the struct must not be used"
        );
        assert!(app.is_null());
    }

    #[test]
    fn a_shell_with_no_platform_still_runs_the_state_machine() {
        // No platform at all: every transition still happens, and nothing is
        // said. This is the shape a shell without a log has always had.
        let app = create(ptr::null());
        // SAFETY: `app` is live; only the state machine is exercised.
        unsafe {
            assert_eq!(nva_app_start(app), NVA_OK);
            assert_eq!(
                nva_app_start(app),
                NVA_ERR_INVALID_ARGUMENT,
                "a refusal without a platform is still a refusal"
            );
        }
        destroy(app);
    }
}
