//! The window-side renderer: one `ANativeWindow`, one swapchain, one clear frame.
//!
//! #93's tracer bullet. `app.rs` records the window the shell attached and then
//! calls [`start`] here; this module takes its own `ANativeWindow` reference
//! (`docs/ARCHITECTURE.md` §7.3), builds a Vulkan device that can present to it,
//! and clears it to `0xff00ff00` until `stop` is set.
//!
//! ## Why this is not `vk.rs`
//!
//! `vk.rs` is the #44 offscreen path: one 64x64 image, no surface, no
//! swapchain, and a device created without `VK_KHR_swapchain`. Its object can
//! only be created through `nva_gfx_create`, which needs a host object the
//! shell does not have. Sharing that device would mean handing the offscreen
//! smoke a swapchain it never asks for; one more Vulkan device is the cheaper
//! trade.
//!
//! ## Vulkan 1.1
//!
//! The target device is an Adreno 725 reporting Vulkan 1.1.128 with no dynamic
//! rendering and `maxViewports = 1` (#26), so the clear goes through a classic
//! render pass with one attachment. There is no pipeline here, hence no
//! viewport to configure, so `maxViewports = 1` is not a constraint on this
//! path; nothing calls `vkCmdBeginRendering` either.
//!
//! ## What the render thread may say
//!
//! Nothing, directly. The shell's log writer assumes one writer, on the main
//! thread (`shell/android/jni/nva_shell_jni.c`, "Thread"), so the loop reports
//! into [`Progress`] and `attach` / `detach` — both on the shell thread — say
//! the lines. That is also why the job carries no platform sink: the only
//! reader of its report is a call that is already on the shell's thread.
//!
//! ## Only on Android
//!
//! `lib.rs` declares this module under `#[cfg(target_os = "android")]`: every
//! path here is a platform call, and the host build must keep the PC smoke
//! tests' zero-sized, `0x1000`-sentinel attach untouched (a dereference of that
//! pointer would crash the test process).
//!
//! ## One template, no copy
//!
//! The device, surface, swapchain and command objects are created inside the
//! thread and destroyed on the way out, by [`Presenter`]'s `Drop`. That includes
//! the failure path: `Presenter::new` stores each object as it is created, and
//! `create_swapchain` destroys the swapchain itself on the three returns that
//! would otherwise leave it ownerless (`VUID-vkDestroyDevice-device-00378`).
//! A failed attach (both `surfaceCreated` and `surfaceChanged` can start one)
//! leaks nothing and leaves nothing dangling.

use core::ffi::{c_void, CStr};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use ash::vk;

use crate::app::RenderCounters;

/// How long `attach` waits for the loop's first present.
///
/// A FIFO swapchain at 60 Hz needs about 16 ms, so one second is far more than
/// the normal case and still short enough not to trip Android's ANR limit. A
/// timeout is not a failure (`docs/GATES.md` §4 reads `presents` out of the
/// snapshot, not out of this wait): the line is evidence, and the loop keeps
/// going.
pub(crate) const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(1);

/// How long one `vkAcquireNextImageKHR` may block before the loop re-checks
/// `stop`.
///
/// 20 ms bounds only the acquire wait: an idle loop re-checks `stop` 50 times a
/// second. It does **not** bound `detach` — the join can also include one
/// frame's `vkQueuePresentKHR` (FIFO blocks until the display consumes the
/// image) and the `device_wait_idle` in `Drop`. `vkWaitForFences` could be
/// given a timeout, but a `VK_TIMEOUT` there is not a loss signal and the loop
/// would only re-arm it, so the wait stays unbounded on purpose; the fence
/// itself signals when the clear completes, which is short. A wedged GPU is a
/// case no Vulkan call can bound, and `detach` inherits that.
const ACQUIRE_TIMEOUT_NS: u64 = 20_000_000;

/// The clear colour behind `0xff00ff00`.
const CLEAR: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

/// The format #93 asks for. Vulkan 1.1 surfaces are free to offer only others,
/// so [`choose_format`] falls back, and the clear colour reads the same in any
/// 8-bit RGBA format because green is written as `1.0`.
const WANTED_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// The colour space that pairs with `WANTED_FORMAT` in the common case.
const WANTED_COLOR_SPACE: vk::ColorSpaceKHR = vk::ColorSpaceKHR::SRGB_NONLINEAR;

/// How many notes the loop may leave for the shell to print.
///
/// `SUBOPTIMAL` can come back on every frame; the loop notes it once and this
/// cap is the backstop for a future note that a driver can repeat.
const MAX_NOTES: usize = 4;

/// The two `ANativeWindow` calls the engine's own reference needs.
///
/// `#[link(name = "android")]` on this block is what puts `libandroid.so` in
/// the `.so`'s `DT_NEEDED` — it lives here, on the android-only module, because
/// the workspace gate (`ci/guard/check_workspace.sh`, #39) forbids any
/// Android reference in a member's build script, and a `build.rs` link
/// directive would trip it. On every other target this module does not exist,
/// so the link argument never reaches a host build.
#[link(name = "android")]
extern "C" {
    fn ANativeWindow_acquire(window: *mut c_void);
    fn ANativeWindow_release(window: *mut c_void);
}

/// The `ANativeWindow` reference the engine holds from attach to thread exit.
///
/// The shell holds its own reference until `nva_app_detach_window` returns
/// (§7.3), so the two counts are independent; this one exists because the
/// engine must not borrow a pointer whose lifetime is only stated by a comment.
/// Releasing happens in `Drop`, and the only value that owns one is the job the
/// render thread owns, so the release is after the last use by construction.
struct EngineWindow {
    native: *mut c_void,
}

impl EngineWindow {
    /// Takes the engine's reference, once, at attach.
    fn acquire(native: *mut c_void) -> Self {
        // SAFETY: `native` is the window the shell attached and still holds a
        // reference to; `ANativeWindow_acquire` only increments that count, so
        // the pointer must be a live `ANativeWindow` — which attach has already
        // checked for NULL and the shell owns until detach returns.
        unsafe { ANativeWindow_acquire(native) };
        Self { native }
    }
}

impl Drop for EngineWindow {
    fn drop(&mut self) {
        // SAFETY: `acquire` took exactly one reference for this value and this
        // is its only release. The value is owned by the render job, which
        // outlives every use of the pointer.
        unsafe { ANativeWindow_release(self.native) };
    }
}

// SAFETY: three things make the move sound, and each is a property this type
// keeps rather than a promise about the caller.
//
// 1. It owns exactly one `ANativeWindow` reference, taken in `acquire` and
//    released in `Drop`; it is moved once (into the job, then into the thread)
//    and never shared — the type has no `Sync`, so no `&EngineWindow` can be
//    handed to another thread.
// 2. `ANativeWindow_acquire`/`_release` are reference counts on an AOSP
//    `RefBase`, which are atomic; a count may be taken and dropped from
//    different threads.
// 3. The only other use of the pointer is `vkCreateAndroidSurfaceKHR`, and that
//    happens on the render thread with an instance the thread owns outright.
unsafe impl Send for EngineWindow {}

/// What `app.rs` hands the new thread. Owned, so nothing is borrowed across it.
pub(crate) struct StartRequest {
    pub(crate) native: *mut c_void,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) generation: u32,
    pub(crate) counters: Arc<RenderCounters>,
}

/// The owned work one render thread performs.
///
/// `window` is the engine's own reference ([`EngineWindow`]); the rest is the
/// shell's generation (for the thread's name in a tombstone), the counters the
/// thread bumps, and the two pieces of shared state: `stop` and `progress`.
struct RenderJob {
    window: EngineWindow,
    width: u32,
    height: u32,
    generation: u32,
    counters: Arc<RenderCounters>,
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
}

/// What the render loop has to say about the first frame, and the notes it
/// leaves behind.
///
/// This is the whole cross-thread conversation: a `Mutex` and a `Condvar`, no
/// locks shared with `App`, and no way for the shell thread to wait longer than
/// [`FIRST_FRAME_TIMEOUT`].
pub(crate) struct Progress {
    state: Mutex<ProgressState>,
    changed: Condvar,
}

struct ProgressState {
    first_frame: FirstFrame,
    notes: Vec<String>,
}

/// The answer [`Progress::wait`] gives, and the line `attach` builds from it.
pub(crate) enum FirstFrame {
    /// No present yet — the wait timed out.
    Pending,
    /// The loop presented at least once.
    Presented,
    /// The loop stopped before its first present; the text is the reason.
    Failed(String),
}

impl Progress {
    fn new() -> Self {
        Self {
            state: Mutex::new(ProgressState {
                first_frame: FirstFrame::Pending,
                notes: Vec::new(),
            }),
            changed: Condvar::new(),
        }
    }

    /// Waits up to `timeout` for the first present and reports what it saw.
    ///
    /// A poisoned lock is reported as a failure rather than recovered
    /// (`LOCAL §4` makes the recovery exception for `PENDING` only). Under
    /// `panic = "abort"` (`LOCAL §1`) the branch cannot be taken.
    pub(crate) fn wait(&self, timeout: Duration) -> FirstFrame {
        let Ok(mut state) = self.state.lock() else {
            return FirstFrame::Failed(String::from("the render progress lock is poisoned"));
        };
        let deadline = Instant::now() + timeout;
        loop {
            match &state.first_frame {
                FirstFrame::Pending => {}
                FirstFrame::Presented => return FirstFrame::Presented,
                FirstFrame::Failed(reason) => return FirstFrame::Failed(reason.clone()),
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return FirstFrame::Pending;
            };
            if remaining.is_zero() {
                return FirstFrame::Pending;
            }
            match self.changed.wait_timeout(state, remaining) {
                Ok((next, _)) => state = next,
                Err(_) => {
                    return FirstFrame::Failed(String::from("the render progress lock is poisoned"))
                }
            }
        }
    }

    /// Records the first successful present. Every later call is a no-op.
    fn presented(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if matches!(state.first_frame, FirstFrame::Pending) {
            state.first_frame = FirstFrame::Presented;
            self.changed.notify_all();
        }
    }

    /// Records why the loop stopped.
    ///
    /// Before the first present this is what `attach` prints; after it, the
    /// loop had already worked, so the reason becomes a note `detach` prints
    /// once it has joined the thread.
    fn fail(&self, reason: String) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        match state.first_frame {
            FirstFrame::Pending => {
                state.first_frame = FirstFrame::Failed(format!("first frame failed: {reason}"));
                self.changed.notify_all();
            }
            FirstFrame::Presented => {
                push_note(&mut state.notes, format!("render loop stopped: {reason}"))
            }
            FirstFrame::Failed(_) => {}
        }
    }

    /// Leaves one note for the shell thread to print.
    fn note(&self, note: String) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        push_note(&mut state.notes, note);
    }

    /// Takes the notes said so far, leaving the pad empty.
    pub(crate) fn take_notes(&self) -> Vec<String> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        core::mem::take(&mut state.notes)
    }
}

/// Adds `note` unless it is already there or the pad is full.
fn push_note(notes: &mut Vec<String>, note: String) {
    if notes.len() < MAX_NOTES && !notes.contains(&note) {
        notes.push(note);
    }
}

/// The handle `App` keeps while a window is attached.
pub(crate) struct RenderLoop {
    /// `Option` so `stop_and_join` can move the handle out of a type that owns a
    /// `Drop` impl; the flag in that impl is the net for a loop dropped without
    /// a join.
    handle: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
}

impl RenderLoop {
    /// The shared report, so `attach` can wait after the loop is in the state.
    pub(crate) fn progress(&self) -> Arc<Progress> {
        Arc::clone(&self.progress)
    }

    /// Stops the loop, waits for the thread to leave, and hands back its notes.
    ///
    /// After this returns nothing can submit for the window, which is what lets
    /// `detach` tell the shell the surface is gone and what lets the shell
    /// release its `ANativeWindow` reference (§7.3).
    pub(crate) fn stop_and_join(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            // A panicking thread would take the process with it under
            // `panic = "abort"` (`LOCAL §1`), so the error arm is unreachable
            // in a product build; in a test profile it means the thread
            // panicked, and the window reference was released by the job's drop
            // either way.
            let _ = handle.join();
        }
        self.progress.take_notes()
    }
}

impl Drop for RenderLoop {
    fn drop(&mut self) {
        // Defence in depth: a loop dropped without `stop_and_join` must not keep
        // presenting forever. This does not replace the join — only a join
        // proves the thread has left before the shell releases the window.
        self.stop.store(true, Ordering::Release);
    }
}

/// Starts the render thread for a freshly recorded window.
///
/// `Ok(None)` means no loop was started on purpose: the shell reported a
/// zero-sized surface (`surfaceCreated` before measurement, so the next
/// `surfaceChanged` will attach again). `Err` means the thread could not be
/// created, and the text is what `attach` prints.
pub(crate) fn start(request: StartRequest) -> Result<Option<RenderLoop>, String> {
    if request.width == 0 || request.height == 0 {
        // A 0x0 swapchain is invalid; the next attach carries the measured
        // size. The window stays recorded, so a detach still matches it.
        return Ok(None);
    }
    let window = EngineWindow::acquire(request.native);
    let stop = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Progress::new());
    let job = RenderJob {
        window,
        width: request.width,
        height: request.height,
        generation: request.generation,
        counters: Arc::clone(&request.counters),
        stop: Arc::clone(&stop),
        progress: Arc::clone(&progress),
    };
    let spawned = thread::Builder::new()
        // The generation makes a thread dump name the surface it belonged to.
        // Android truncates the name at 15 bytes; that is fine, the number is
        // the part worth reading.
        .name(format!("nva-render-{}", job.generation))
        .spawn(move || thread_main(job));
    match spawned {
        Ok(handle) => Ok(Some(RenderLoop {
            handle: Some(handle),
            stop,
            progress,
        })),
        Err(error) => {
            // The job — and with it the window reference — was dropped by
            // `spawn` on this thread when it returned the error.
            Err(format!("start the render thread: {error}"))
        }
    }
}

/// The render thread's entry point: build the objects, present until stopped.
fn thread_main(job: RenderJob) {
    // SAFETY: every handle `present_loop` creates is created and destroyed on
    // this thread, and the only outside pointer is the `ANativeWindow`
    // reference the job owns. The loop returns before the job drops, so the
    // window reference outlives every use.
    let result = unsafe { present_loop(&job) };
    if let Err(reason) = result {
        job.progress.fail(reason);
    }
}

/// Presents the clear colour until `stop` is set.
///
/// # Safety
/// Called once, on the thread that owns `job`. `job.window` is a live
/// `ANativeWindow` reference for the whole call.
unsafe fn present_loop(job: &RenderJob) -> Result<(), String> {
    let presenter = Presenter::new(job.window.native, job.width, job.height)?;
    while !job.stop.load(Ordering::Acquire) {
        // The frame and the submit are counted inside `present`; the loop only
        // records that the first frame arrived and says what the surface is
        // doing.
        match presenter.present(&job.counters, &job.stop) {
            Ok(Presented::Frame) => job.progress.presented(),
            Ok(Presented::Suboptimal) => {
                // The image did go up. Rebuilding belongs to a later ticket
                // (out of scope here), so the loop says so once and carries on.
                job.progress.presented();
                job.progress.note(String::from(
                    "the swapchain is suboptimal for the surface; #93 does not rebuild it",
                ));
            }
            Err(PresentError::OutOfDate) => {
                // Acquiring again would fail the same way; the shell destroys
                // the surface next, which turns into a detach. Stopping here
                // keeps the loop from spinning on a dead swapchain. `fail`
                // rather than `note`: before the first frame this is the root
                // cause `attach` should print straight away (and it wakes the
                // wait instead of letting it run out and report "not ready in
                // 1s" first); after it, `fail` turns into a detach note.
                job.progress.fail(String::from(
                    "the swapchain went out of date; the render loop stopped",
                ));
                return Ok(());
            }
            // `stop` was set while no image was available: an ordinary exit,
            // not a failure, and the only way `detach`'s join can be bounded.
            Err(PresentError::Stopped) => return Ok(()),
            Err(PresentError::Failed(reason)) => return Err(reason),
        }
    }
    Ok(())
}

/// What one present call did.
enum Presented {
    /// The image was presented; the swapchain is still a good match.
    Frame,
    /// The image was presented, but the surface no longer matches the
    /// swapchain exactly (`VK_SUBOPTIMAL_KHR`).
    Suboptimal,
}

/// Why one present call produced no frame.
enum PresentError {
    /// The swapchain no longer matches the surface.
    OutOfDate,
    /// `stop` was set while waiting for an image.
    Stopped,
    /// A Vulkan call failed.
    Failed(String),
}

/// The WSI object graph for one attached window.
///
/// Four fields own an object that only a loader-bearing call can destroy: the
/// instance, the instance's surface, the device and the swapchain. They are
/// `Option` so `Drop` can tell "created" from "did not get that far": `new`
/// fills every one of them before it returns `Ok`, so no code outside `new`
/// sees a `None` except the one guard at the top of [`Self::present`]. Every
/// other field is a handle whose `destroy_*` call accepts `VK_NULL_HANDLE`, so
/// "not created yet" needs no representation of its own; the vectors start
/// empty and take what has been created.
///
/// `Drop` is the only teardown path, which is what makes an early `?` in `new`
/// harmless: it destroys whatever the fields hold, in reverse creation order.
/// `_entry` is dropped last (the loader library must stay loaded until
/// `vkDestroyInstance` has run), which is true because it is a field and fields
/// drop after `drop` returns.
struct Presenter {
    _entry: ash::Entry,
    instance: Option<ash::Instance>,
    surface_loader: Option<ash::khr::surface::Instance>,
    surface: vk::SurfaceKHR,
    device: Option<ash::Device>,
    swapchain_loader: Option<ash::khr::swapchain::Device>,
    swapchain: vk::SwapchainKHR,
    render_pass: vk::RenderPass,
    /// One view per swapchain image. The framebuffers name them, so they are
    /// destroyed after the framebuffers, not before (a view must outlive every
    /// framebuffer that references it).
    image_views: Vec<vk::ImageView>,
    framebuffers: Vec<vk::Framebuffer>,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    image_available: vk::Semaphore,
    render_finished: vk::Semaphore,
    in_flight: vk::Fence,
    extent: vk::Extent2D,
}

impl Presenter {
    /// The state before anything has been created: one entry, everything else
    /// "absent" in the form `Drop` understands.
    fn empty(entry: ash::Entry) -> Self {
        Self {
            _entry: entry,
            instance: None,
            surface_loader: None,
            surface: vk::SurfaceKHR::null(),
            device: None,
            swapchain_loader: None,
            swapchain: vk::SwapchainKHR::null(),
            render_pass: vk::RenderPass::null(),
            image_views: Vec::new(),
            framebuffers: Vec::new(),
            queue: vk::Queue::null(),
            command_pool: vk::CommandPool::null(),
            command_buffer: vk::CommandBuffer::null(),
            image_available: vk::Semaphore::null(),
            render_finished: vk::Semaphore::null(),
            in_flight: vk::Fence::null(),
            extent: vk::Extent2D::default(),
        }
    }

    /// Builds the instance, surface, device and presentation objects.
    ///
    /// Each object is stored in the `Presenter` being built as soon as it
    /// exists, so every `?` below is cleaned up by the `Drop` impl below and
    /// nothing is left orphaned. That matters because a failed attach is not
    /// hypothetical: `surfaceCreated` and `surfaceChanged` both start one, so a
    /// rotation that fails twice would otherwise leak two instances.
    ///
    /// # Safety
    /// `window` must be a live `ANativeWindow` reference that outlives the
    /// returned value; `present_loop` passes the job's own reference.
    unsafe fn new(window: *mut c_void, width: u32, height: u32) -> Result<Self, String> {
        let entry = ash::Entry::load().map_err(|error| format!("load libvulkan: {error}"))?;
        let mut presenter = Self::empty(entry);

        let instance = create_instance(&presenter._entry)?;
        let surface_loader = ash::khr::surface::Instance::new(&presenter._entry, &instance);
        let android_loader = ash::khr::android_surface::Instance::new(&presenter._entry, &instance);
        presenter.instance = Some(instance);
        presenter.surface_loader = Some(surface_loader);
        // The `let … else` reborrows below cannot be `None` — the lines just
        // above filled them in — but they are the non-panicking form of that
        // invariant: a render thread has no useful panic path, so a broken
        // invariant reads like any other setup failure.
        let Some(instance) = presenter.instance.as_ref() else {
            return Err(String::from("the instance was not installed"));
        };
        presenter.surface = android_loader
            .create_android_surface(
                &vk::AndroidSurfaceCreateInfoKHR::default().window(window),
                None,
            )
            .map_err(|error| format!("vkCreateAndroidSurfaceKHR: {error}"))?;
        let Some(surface_loader) = presenter.surface_loader.as_ref() else {
            return Err(String::from("the surface loader was not installed"));
        };

        let (physical, family) = choose_device(instance, surface_loader, presenter.surface)?;
        let device = create_device(instance, physical, family)?;
        let swapchain_loader = ash::khr::swapchain::Device::new(instance, &device);
        presenter.queue = device.get_device_queue(family, 0);
        presenter.device = Some(device);
        presenter.swapchain_loader = Some(swapchain_loader);
        let Some(device) = presenter.device.as_ref() else {
            return Err(String::from("the device was not installed"));
        };
        let Some(swapchain_loader) = presenter.swapchain_loader.as_ref() else {
            return Err(String::from("the swapchain loader was not installed"));
        };

        let Swapchain {
            handle,
            format,
            extent,
            render_pass,
            images,
        } = create_swapchain(
            surface_loader,
            swapchain_loader,
            physical,
            presenter.surface,
            device,
            vk::Extent2D { width, height },
        )?;
        presenter.swapchain = handle;
        presenter.render_pass = render_pass;
        presenter.extent = extent;

        for image in &images {
            let view = device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(*image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(format)
                        .components(vk::ComponentMapping::default())
                        .subresource_range(
                            vk::ImageSubresourceRange::default()
                                .aspect_mask(vk::ImageAspectFlags::COLOR)
                                .base_mip_level(0)
                                .level_count(1)
                                .base_array_layer(0)
                                .layer_count(1),
                        ),
                    None,
                )
                .map_err(|error| format!("vkCreateImageView: {error}"))?;
            presenter.image_views.push(view);
            let framebuffer = device
                .create_framebuffer(
                    &vk::FramebufferCreateInfo::default()
                        .render_pass(render_pass)
                        .attachments(std::slice::from_ref(&view))
                        .width(extent.width)
                        .height(extent.height)
                        .layers(1),
                    None,
                )
                .map_err(|error| format!("vkCreateFramebuffer: {error}"))?;
            presenter.framebuffers.push(framebuffer);
        }

        presenter.command_pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .queue_family_index(family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
                None,
            )
            .map_err(|error| format!("vkCreateCommandPool: {error}"))?;
        presenter.command_buffer = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(presenter.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
            .map_err(|error| format!("vkAllocateCommandBuffers: {error}"))?
            .first()
            .copied()
            .ok_or_else(|| String::from("vkAllocateCommandBuffers returned no buffer"))?;
        presenter.image_available = device
            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            .map_err(|error| format!("vkCreateSemaphore(image available): {error}"))?;
        presenter.render_finished = device
            .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
            .map_err(|error| format!("vkCreateSemaphore(render finished): {error}"))?;
        presenter.in_flight = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(|error| format!("vkCreateFence: {error}"))?;

        Ok(presenter)
    }

    /// Clears one swapchain image, presents it, and waits for completion.
    ///
    /// Answers "did the surface still match the swapchain exactly", and counts
    /// the frame and the submit in `counters` — the loop that calls this has
    /// nothing to add to either count.
    ///
    /// # Safety
    /// Called on the thread that owns the device; the queue, command buffer and
    /// semaphores are used by no other thread.
    unsafe fn present(
        &self,
        counters: &RenderCounters,
        stop: &AtomicBool,
    ) -> Result<Presented, PresentError> {
        let (Some(device), Some(swapchain_loader)) =
            (self.device.as_ref(), self.swapchain_loader.as_ref())
        else {
            // `new` fills both before it returns `Ok`. This arm is the total
            // form of that invariant: a render thread has no useful panic path,
            // and the loop turns the error into a note.
            return Err(PresentError::Failed(String::from(
                "the presenter was not completed",
            )));
        };
        let (index, suboptimal) = loop {
            match swapchain_loader.acquire_next_image(
                self.swapchain,
                ACQUIRE_TIMEOUT_NS,
                self.image_available,
                vk::Fence::null(),
            ) {
                Ok(pair) => break pair,
                // A FIFO swapchain with no consumer hands out no images; the
                // wait is re-armed so `stop` is noticed within one timeout.
                Err(vk::Result::TIMEOUT) => {
                    if stop.load(Ordering::Acquire) {
                        return Err(PresentError::Stopped);
                    }
                }
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Err(PresentError::OutOfDate),
                Err(error) => {
                    return Err(PresentError::Failed(format!(
                        "vkAcquireNextImageKHR: {error}"
                    )))
                }
            }
        };
        let Some(framebuffer) = self.framebuffers.get(index as usize).copied() else {
            return Err(PresentError::Failed(format!(
                "the swapchain handed out image {index}, which it does not own"
            )));
        };

        device
            .reset_fences(std::slice::from_ref(&self.in_flight))
            .map_err(|error| PresentError::Failed(format!("vkResetFences: {error}")))?;
        device
            .reset_command_buffer(self.command_buffer, vk::CommandBufferResetFlags::empty())
            .map_err(|error| PresentError::Failed(format!("vkResetCommandBuffer: {error}")))?;
        device
            .begin_command_buffer(
                self.command_buffer,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )
            .map_err(|error| PresentError::Failed(format!("vkBeginCommandBuffer: {error}")))?;

        let clear = vk::ClearValue {
            color: vk::ClearColorValue { float32: CLEAR },
        };
        let begin = vk::RenderPassBeginInfo::default()
            .render_pass(self.render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: self.extent,
            })
            .clear_values(std::slice::from_ref(&clear));
        device.cmd_begin_render_pass(self.command_buffer, &begin, vk::SubpassContents::INLINE);
        device.cmd_end_render_pass(self.command_buffer);
        device
            .end_command_buffer(self.command_buffer)
            .map_err(|error| PresentError::Failed(format!("vkEndCommandBuffer: {error}")))?;

        let submit = vk::SubmitInfo::default()
            .wait_semaphores(std::slice::from_ref(&self.image_available))
            .wait_dst_stage_mask(&[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
            .command_buffers(std::slice::from_ref(&self.command_buffer))
            .signal_semaphores(std::slice::from_ref(&self.render_finished));
        device
            .queue_submit(self.queue, std::slice::from_ref(&submit), self.in_flight)
            .map_err(|error| PresentError::Failed(format!("vkQueueSubmit: {error}")))?;
        counters.record_submit();
        // One submit in flight, so waiting its fence is waiting the frame; it is
        // also what makes the present below the last thing that can fail. The
        // wait is unbounded on purpose — a device that stops signalling reports
        // `VK_ERROR_DEVICE_LOST` instead of hanging.
        device
            .wait_for_fences(
                std::slice::from_ref(&self.in_flight),
                true, // waitAll
                u64::MAX,
            )
            .map_err(|error| PresentError::Failed(format!("vkWaitForFences: {error}")))?;

        let present = vk::PresentInfoKHR::default()
            .wait_semaphores(std::slice::from_ref(&self.render_finished))
            .swapchains(std::slice::from_ref(&self.swapchain))
            .image_indices(std::slice::from_ref(&index));
        match swapchain_loader.queue_present(self.queue, &present) {
            Ok(also_suboptimal) => {
                counters.record_present();
                Ok(if suboptimal || also_suboptimal {
                    Presented::Suboptimal
                } else {
                    Presented::Frame
                })
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => Err(PresentError::OutOfDate),
            Err(error) => Err(PresentError::Failed(format!("vkQueuePresentKHR: {error}"))),
        }
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        // SAFETY: every handle here was created by `new` on this thread, and
        // this is the only place any of them is destroyed — the two `Option`
        // reads are "did `new` get that far", not a second owner. The device is
        // idle first, so no submission still references them. Every child is
        // destroyed before the device (VUID-vkDestroyDevice-device-00378): the
        // framebuffers before the views they name and before the render pass
        // they use, the swapchain before the device, and the device before the
        // surface and the instance. A swapchain that `create_swapchain` could
        // not hand over was destroyed there, so nothing here has to know about
        // it. `_entry` is a field, so the loader library outlives
        // `vkDestroyInstance`. A handle that was never created is null, which
        // every `destroy_*` call accepts.
        unsafe {
            if let Some(device) = &self.device {
                let _ = device.device_wait_idle();
                for framebuffer in self.framebuffers.drain(..) {
                    device.destroy_framebuffer(framebuffer, None);
                }
                for view in self.image_views.drain(..) {
                    device.destroy_image_view(view, None);
                }
                device.destroy_render_pass(self.render_pass, None);
                device.destroy_semaphore(self.image_available, None);
                device.destroy_semaphore(self.render_finished, None);
                device.destroy_fence(self.in_flight, None);
                device.destroy_command_pool(self.command_pool, None);
                if let Some(swapchain_loader) = &self.swapchain_loader {
                    swapchain_loader.destroy_swapchain(self.swapchain, None);
                }
                device.destroy_device(None);
            }
            if let Some(surface_loader) = &self.surface_loader {
                surface_loader.destroy_surface(self.surface, None);
            }
            if let Some(instance) = &self.instance {
                instance.destroy_instance(None);
            }
        }
    }
}

/// The instance, with the one platform extension this path needs.
///
/// # Safety
/// Vulkan entry points are raw FFI; the loader must stay loaded while the
/// instance lives, which `Presenter::_entry` guarantees.
unsafe fn create_instance(entry: &ash::Entry) -> Result<ash::Instance, String> {
    let available = entry
        .enumerate_instance_extension_properties(None)
        .map_err(|error| format!("vkEnumerateInstanceExtensionProperties: {error}"))?;
    for required in [ash::khr::surface::NAME, ash::khr::android_surface::NAME] {
        let present = available
            .iter()
            .any(|property| CStr::from_ptr(property.extension_name.as_ptr()) == required);
        if !present {
            return Err(format!(
                "the Vulkan loader does not offer {}",
                required.to_string_lossy()
            ));
        }
    }

    let extensions = [
        ash::khr::surface::NAME.as_ptr(),
        ash::khr::android_surface::NAME.as_ptr(),
    ];
    let app_name = c"nva-runtime";
    let app_info = vk::ApplicationInfo::default()
        .application_name(app_name)
        .application_version(1)
        .engine_name(app_name)
        .engine_version(1)
        .api_version(vk::make_api_version(0, 1, 1, 0));
    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&extensions);
    entry
        .create_instance(&create_info, None)
        .map_err(|error| format!("vkCreateInstance: {error}"))
}

/// Picks a physical device with a graphics queue that can present to `surface`.
///
/// # Safety
/// `surface` must belong to `instance`, and `surface_loader` must have been
/// loaded from it.
unsafe fn choose_device(
    instance: &ash::Instance,
    surface_loader: &ash::khr::surface::Instance,
    surface: vk::SurfaceKHR,
) -> Result<(vk::PhysicalDevice, u32), String> {
    let devices = instance
        .enumerate_physical_devices()
        .map_err(|error| format!("vkEnumeratePhysicalDevices: {error}"))?;
    for physical in devices {
        let extensions = instance
            .enumerate_device_extension_properties(physical)
            .map_err(|error| format!("vkEnumerateDeviceExtensionProperties: {error}"))?;
        if !extensions.iter().any(|property| {
            CStr::from_ptr(property.extension_name.as_ptr()) == ash::khr::swapchain::NAME
        }) {
            continue;
        }
        let families = instance.get_physical_device_queue_family_properties(physical);
        for (index, family) in families.iter().enumerate() {
            if !family.queue_flags.contains(vk::QueueFlags::GRAPHICS) {
                continue;
            }
            let index = index as u32;
            // A failed query is treated as "this family cannot present": the
            // next candidate is a cheaper answer than failing the attach.
            if !surface_loader
                .get_physical_device_surface_support(physical, index, surface)
                .unwrap_or(false)
            {
                continue;
            }
            return Ok((physical, index));
        }
    }
    Err(String::from(
        "no Vulkan device offers a graphics queue that can present to this surface",
    ))
}

/// Creates the logical device with `VK_KHR_swapchain` and one queue.
///
/// # Safety
/// `physical` must come from `instance`, and `family` must be one of its queue
/// families with `GRAPHICS`.
unsafe fn create_device(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    family: u32,
) -> Result<ash::Device, String> {
    let priorities = [1.0f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family)
        .queue_priorities(&priorities)];
    let extensions = [ash::khr::swapchain::NAME.as_ptr()];
    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queues)
        .enabled_extension_names(&extensions);
    instance
        .create_device(physical, &create_info, None)
        .map_err(|error| format!("vkCreateDevice: {error}"))
}

/// The swapchain plus the render pass that clears it.
struct Swapchain {
    handle: vk::SwapchainKHR,
    format: vk::Format,
    extent: vk::Extent2D,
    render_pass: vk::RenderPass,
    images: Vec<vk::Image>,
}

/// Creates the swapchain and the render pass that writes to it.
///
/// # Safety
/// `surface` must belong to the instance behind `loader`, `device` must have
/// been created from the same instance with `VK_KHR_swapchain`, and both
/// loaders must be loaded from the same instance/device pair.
unsafe fn create_swapchain(
    surface_loader: &ash::khr::surface::Instance,
    loader: &ash::khr::swapchain::Device,
    physical: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    device: &ash::Device,
    requested: vk::Extent2D,
) -> Result<Swapchain, String> {
    let capabilities = surface_loader
        .get_physical_device_surface_capabilities(physical, surface)
        .map_err(|error| format!("vkGetPhysicalDeviceSurfaceCapabilitiesKHR: {error}"))?;
    if capabilities.max_image_extent.width == 0 || capabilities.max_image_extent.height == 0 {
        return Err(String::from("the surface reports no usable extent"));
    }
    let extent = if capabilities.current_extent.width != u32::MAX {
        capabilities.current_extent
    } else {
        vk::Extent2D {
            width: requested.width.clamp(
                capabilities.min_image_extent.width,
                capabilities.max_image_extent.width,
            ),
            height: requested.height.clamp(
                capabilities.min_image_extent.height,
                capabilities.max_image_extent.height,
            ),
        }
    };

    let formats = surface_loader
        .get_physical_device_surface_formats(physical, surface)
        .map_err(|error| format!("vkGetPhysicalDeviceSurfaceFormatsKHR: {error}"))?;
    let chosen = choose_format(&formats)?;

    let mut image_count = capabilities.min_image_count + 1;
    if capabilities.max_image_count != 0 && image_count > capabilities.max_image_count {
        image_count = capabilities.max_image_count;
    }

    let create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(surface)
        .min_image_count(image_count)
        .image_format(chosen.format)
        .image_color_space(chosen.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
        .pre_transform(capabilities.current_transform)
        .composite_alpha(composite_alpha(capabilities.supported_composite_alpha))
        // FIFO is the one mode every surface must support; it also means a
        // present blocks on the display, which is why 60 frames take ~1 s.
        .present_mode(vk::PresentModeKHR::FIFO)
        .clipped(true)
        .old_swapchain(vk::SwapchainKHR::null());
    let handle = loader
        .create_swapchain(&create_info, None)
        .map_err(|error| format!("vkCreateSwapchainKHR: {error}"))?;
    // The swapchain exists and the `Swapchain` that would own it does not, so
    // each early return below destroys it first: leaving it alive and then
    // destroying the device is VUID-vkDestroyDevice-device-00378 (every child
    // object must be gone), which `Presenter::drop` cannot repair afterwards.
    let images = match loader.get_swapchain_images(handle) {
        Ok(images) => images,
        Err(error) => {
            loader.destroy_swapchain(handle, None);
            return Err(format!("vkGetSwapchainImagesKHR: {error}"));
        }
    };
    if images.is_empty() {
        loader.destroy_swapchain(handle, None);
        return Err(String::from("the swapchain created no images"));
    }

    // Vulkan 1.1 has no dynamic rendering (#26): one attachment, cleared on
    // load, handed to the presentation engine on store.
    let attachment = vk::AttachmentDescription::default()
        .format(chosen.format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);
    let color_reference = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_reference));
    let dependencies = [
        vk::SubpassDependency::default()
            .src_subpass(vk::SUBPASS_EXTERNAL)
            .dst_subpass(0)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dependency_flags(vk::DependencyFlags::BY_REGION),
        vk::SubpassDependency::default()
            .src_subpass(0)
            .dst_subpass(vk::SUBPASS_EXTERNAL)
            .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags::BOTTOM_OF_PIPE)
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE),
    ];
    let render_pass = match device.create_render_pass(
        &vk::RenderPassCreateInfo::default()
            .attachments(std::slice::from_ref(&attachment))
            .subpasses(std::slice::from_ref(&subpass))
            .dependencies(&dependencies),
        None,
    ) {
        Ok(render_pass) => render_pass,
        Err(error) => {
            loader.destroy_swapchain(handle, None);
            return Err(format!("vkCreateRenderPass: {error}"));
        }
    };

    Ok(Swapchain {
        handle,
        format: chosen.format,
        extent,
        render_pass,
        images,
    })
}

/// The requested format when the surface offers it, else the first 8-bit RGBA
/// it does offer, else whatever it lists first.
fn choose_format(formats: &[vk::SurfaceFormatKHR]) -> Result<vk::SurfaceFormatKHR, String> {
    let Some(first) = formats.first().copied() else {
        return Err(String::from("the surface offers no formats"));
    };
    let exact = formats
        .iter()
        .find(|format| format.format == WANTED_FORMAT && format.color_space == WANTED_COLOR_SPACE);
    let same_format = formats.iter().find(|format| format.format == WANTED_FORMAT);
    let eight_bit = formats
        .iter()
        .find(|format| is_eight_bit_rgba(format.format));
    let chosen = exact
        .or(same_format)
        .or(eight_bit)
        .copied()
        .unwrap_or(first);
    Ok(chosen)
}

/// Whether `format` is one of the 8-bit RGBA layouts whose green channel is
/// full scale at `1.0`.
fn is_eight_bit_rgba(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::R8G8B8A8_UNORM
            | vk::Format::R8G8B8A8_SRGB
            | vk::Format::B8G8R8A8_UNORM
            | vk::Format::B8G8R8A8_SRGB
            | vk::Format::A8B8G8R8_UNORM_PACK32
            | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

/// `OPAQUE` when the surface supports it, else the first mode it does support.
///
/// A surface that supports none of the four is not something Vulkan allows, so
/// the last arm is a conservative default rather than a fifth check.
fn composite_alpha(supported: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    for candidate in [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::INHERIT,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
    ] {
        if supported.contains(candidate) {
            return candidate;
        }
    }
    vk::CompositeAlphaFlagsKHR::OPAQUE
}
