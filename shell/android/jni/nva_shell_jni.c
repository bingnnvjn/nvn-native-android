/*
 * JNI library for the product shell (#46, #91).
 *
 * #46 made this the smallest thing that satisfies docs/GATES.md §3: the library
 * exports JNI_OnLoad, and the activity makes one real JNI call so the export is
 * a live entry point rather than a symbol nobody loads.
 *
 * #91 turns it into the first real traffic over the shell → runtime seam
 * (docs/ARCHITECTURE.md §7.1): the SurfaceView's three surface events and the
 * activity lifecycle are forwarded to the `nva_app_*` exports, and the
 * ANativeWindow reference is acquired and released here. Still no Vulkan, no
 * guest, no audio in this library.
 *
 * ## Who holds the window reference
 *
 * `ANativeWindow_fromSurface` returns a reference the *caller* owns. This file
 * is that caller, and the only one: it takes the reference in
 * `windowAttached` and releases it in `windowDetached` **after**
 * `nva_app_detach_window` has returned successfully, which is the order §7.3
 * requires ("detach 返回后壳才 release"). A refused detach leaves the record
 * and the reference together — see the comment on that branch. The runtime
 * borrows the bare pointer in between and releases nothing, so there is exactly
 * one owner and no shape where both sides believe the other one owns it.
 *
 * ## Thread
 *
 * Every function here runs on the Android main thread: activity callbacks and
 * SurfaceHolder callbacks are both delivered there, so the statics below need
 * no lock. The runtime locks its own state regardless, because the render
 * thread #93 adds will read it while these callbacks are still arriving.
 */
#include <jni.h>

#include <android/log.h>
#include <android/native_window.h>
#include <android/native_window_jni.h>

#include "nva_abi.h"

#define SHELL_LOG(...) \
    __android_log_print(ANDROID_LOG_INFO, "nva-shell", __VA_ARGS__)

#define SHELL_VERSION "0.1.0-m0a"

/* Created in onCreate, destroyed in onDestroy. NULL means "not created yet". */
static nva_app *g_app;

/* The one reference this file owns; NULL when no surface is attached. */
static ANativeWindow *g_window;

/* The generation the runtime holds; 0 when no window is attached. */
static uint32_t g_generation;

/*
 * Returning a version the VM does not support makes the load fail loudly, which
 * is what we want: a shell that silently runs without its JNI entry point is
 * the failure mode this assertion exists to catch.
 */
JNIEXPORT jint JNICALL
JNI_OnLoad(JavaVM *vm, void *reserved)
{
    (void)vm;
    (void)reserved;
    SHELL_LOG("JNI_OnLoad");
    return JNI_VERSION_1_6;
}

JNIEXPORT jstring JNICALL
Java_com_bingnnvjn_nva_MainActivity_shellInfo(JNIEnv *env, jobject thiz)
{
    (void)thiz;
    SHELL_LOG("shellInfo");
    return (*env)->NewStringUTF(env, "NVA shell " SHELL_VERSION " (JNI_OnLoad ran)");
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appCreate(JNIEnv *env, jobject thiz, jstring data_dir)
{
    (void)thiz;
    if (g_app != NULL) {
        SHELL_LOG("app already created");
        return NVA_ERR_INVALID_ARGUMENT;
    }

    const char *dir = NULL;
    if (data_dir != NULL) {
        dir = (*env)->GetStringUTFChars(env, data_dir, NULL);
    }
    /*
     * No callbacks yet: the shell has nothing to start when a surface arrives,
     * and the two logs around this call are the evidence this ticket asks for.
     * #93 passes `on_surface_ready` here so the engine can build the swapchain;
     * the runtime already calls it when it is set
     * (crates/nva-runtime/src/app.rs).
     *
     * `desc` and `dir` live for this call only. The runtime copies the callback
     * pointers and stores none of the other fields, so releasing `dir` right
     * after is correct; the day it does store `data_dir`, the release moves too.
     */
    nva_app_desc desc = {
        .size = (uint32_t)sizeof(desc),
        .version = NVA_APP_DESC_VERSION,
        .flags = 0,
        .reserved = 0,
        .data_dir = dir,
        .callbacks = NULL,
        .platform = NULL,
    };
    nva_app *app = NULL;
    nva_status status = nva_app_create(&desc, &app);
    if (dir != NULL) {
        (*env)->ReleaseStringUTFChars(env, data_dir, dir);
    }
    if (status != NVA_OK) {
        SHELL_LOG("app create failed: status=%u", (unsigned)status);
        return (jint)status;
    }

    g_app = app;
    SHELL_LOG("app created");
    return (jint)NVA_OK;
}

/*
 * One lifecycle step: forward to the runtime, then log the event the ticket
 * names (`started`, `paused`, `resumed`, `stopped`) or the refusal.
 *
 * `step` and `event` are the present and past tense of the same transition, and
 * they differ on purpose: the log line is the evidence a device run is read
 * for, so it says what happened, while a refusal names the call that failed.
 */
static jint lifecycle_step(const char *step, const char *event, nva_status (*call)(nva_app *))
{
    if (g_app == NULL) {
        SHELL_LOG("%s without an app", step);
        return NVA_ERR_INVALID_ARGUMENT;
    }
    nva_status status = call(g_app);
    if (status == NVA_OK) {
        SHELL_LOG("%s", event);
    } else {
        SHELL_LOG("%s failed: status=%u", step, (unsigned)status);
    }
    return (jint)status;
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appStart(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    return lifecycle_step("start", "started", nva_app_start);
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appPause(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    return lifecycle_step("pause", "paused", nva_app_pause);
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appResume(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    return lifecycle_step("resume", "resumed", nva_app_resume);
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appStop(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    return lifecycle_step("stop", "stopped", nva_app_stop);
}

/*
 * The Java side skips zero-size `surfaceChanged` events, so a width or height
 * of 0 here is a caller bug rather than a transient state: the runtime records
 * what it is told, and a 0-shaped window would be #93's problem to detect.
 */
JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_windowAttached(JNIEnv *env, jobject thiz, jobject surface,
                                                   jint width, jint height, jint generation)
{
    (void)thiz;
    if (g_app == NULL) {
        SHELL_LOG("surface attached without an app");
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if (surface == NULL) {
        SHELL_LOG("surface is null");
        return NVA_ERR_SURFACE_LOST;
    }

    ANativeWindow *window = ANativeWindow_fromSurface(env, surface);
    if (window == NULL) {
        SHELL_LOG("ANativeWindow_fromSurface failed");
        return NVA_ERR_SURFACE_LOST;
    }

    nva_status status = nva_app_attach_window(g_app, window, (uint32_t)width, (uint32_t)height,
                                             (uint32_t)generation);
    if (status != NVA_OK) {
        /* The runtime did not take the window, so its reference is ours alone. */
        ANativeWindow_release(window);
        SHELL_LOG("surface attach failed: status=%u", (unsigned)status);
        return (jint)status;
    }

    /*
     * A second attach while one is live is the rotation path: the new window
     * supersedes the old one, so the old reference goes back as soon as the
     * runtime has stopped pointing at it.
     */
    if (g_window != NULL) {
        ANativeWindow_release(g_window);
    }
    g_window = window;
    /* The generation the runtime holds. `windowDetached` uses this rather than
     * a value from Java, so the attach/detach pair can never name two different
     * windows and the release below always follows a successful detach. */
    g_generation = (uint32_t)generation;
    SHELL_LOG("surface attached (generation=%d)", (int)generation);
    return (jint)NVA_OK;
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_windowDetached(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    if (g_app == NULL) {
        SHELL_LOG("detach without an app");
        return NVA_ERR_INVALID_ARGUMENT;
    }

    nva_status status = nva_app_detach_window(g_app, g_generation);
    if (status != NVA_OK) {
        /*
         * The runtime still holds the record. Releasing our reference anyway
         * would leave it pointing at a window nobody owns, so the record and
         * the reference stay together: the next attach replaces both, and a
         * `destroy` releases what is left.
         */
        SHELL_LOG("detach failed: status=%u", (unsigned)status);
        return (jint)status;
    }

    /* §7.3: the shell releases only after detach has returned. */
    if (g_window != NULL) {
        ANativeWindow_release(g_window);
        g_window = NULL;
    }
    SHELL_LOG("detached (generation=%u)", (unsigned)g_generation);
    g_generation = 0;
    return (jint)NVA_OK;
}

/*
 * `onDestroy`. Android normally delivers `surfaceDestroyed` first; when it does
 * not, the activity is going away with a live surface, so the reference is
 * dropped here rather than leaked.
 */
JNIEXPORT void JNICALL
Java_com_bingnnvjn_nva_MainActivity_appDestroy(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    if (g_app == NULL) {
        return;
    }
    if (g_window != NULL) {
        SHELL_LOG("destroy with a window still attached");
        ANativeWindow_release(g_window);
        g_window = NULL;
    }
    nva_app_destroy(g_app);
    g_app = NULL;
    SHELL_LOG("destroyed");
}
