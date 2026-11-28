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
 * #93 adds the read side of the stats snapshot: a successful attach and a
 * successful detach each end with one `attach stats …` / `detach stats …` line
 * (`shell_log_stats`). The counters are produced by the runtime's render
 * thread, which lives in `libnva_runtime.so` — this file only asks for them.
 *
 * ## Who holds the window reference
 *
 * `ANativeWindow_fromSurface` returns a reference the *caller* owns, and this
 * file is one of two owners. It takes its reference in `windowAttached` and
 * releases it in `windowDetached` **after** `nva_app_detach_window` has returned
 * successfully, which is the order §7.3 requires ("detach 返回后壳才 release").
 * A refused detach leaves the record and this file's reference together — see
 * the comment on that branch.
 *
 * The other owner is the engine (#93), and the two counts are independent: the
 * runtime acquires its own reference when it starts the render thread and
 * releases it when that thread exits, which `nva_app_detach_window` waits for
 * through the join. So detach returning means the engine holds no pointer and
 * this file may drop its own, while the window itself stays alive until it
 * does. Neither side believes the other one owns the reference.
 *
 * ## Thread
 *
 * Every function here runs on the Android main thread: activity callbacks and
 * SurfaceHolder callbacks are both delivered there, so the statics below need
 * no lock. The runtime keeps its own state behind a mutex, and #93's render
 * thread never touches it — it is handed an owned job plus two counters, and
 * reports back through a condition variable the attach thread waits on. That is
 * also why the render thread never calls `nva_platform.log`: the log file has
 * one writer, and it is this thread.
 *
 * ## The device log (#99)
 *
 * Every *event* line this file writes goes through `shell_log`, which formats
 * once and hands the same bytes to both exits: logcat, and
 * `<data_dir>/nva-shell.log`. (The run header is built and emitted directly.)
 * Formatting once is the point — a log read off the phone and the same run read
 * in `adb logcat` must not be two accounts that disagree. See
 * docs/DEVICE-LOGS.md for how the file is retrieved and what the fields mean.
 *
 * One writer, no lock. Only `shell_emit` writes `g_log_file`: `shell_log_open`,
 * `shell_rotate` and `shell_log_close` merely open, rename and close it, and
 * `shell_count_runs` reads through a second handle it owns outright. A line is
 * one `fwrite` including its newline, so a write either lands whole or is
 * re-measured — never half a line waiting for the next one to complete it.
 * The Java side never opens the file at all: its lines come back through
 * `nativeLog`. Two writers would interleave half lines, which is the one thing
 * the ticket rules out. `fflush` runs after every line because the interesting
 * forensics happen after the system kills the process, and a kill discards
 * whatever is still buffered.
 *
 * The file is opened in `appCreate` and closed in `appDestroy`; a line emitted
 * before that — `JNI_OnLoad`, `shellInfo` — reaches logcat only. The run header
 * is written when the file opens, so on a create that succeeds, `app created`
 * is the line after it.
 *
 * The runtime's own lines (#101) come back through `nva_platform.log`, which
 * lands in `shell_platform_log` and then in the same `shell_log`: one file, one
 * formatter, one timestamp. `[rt]` is the whole difference a reader has to know.
 */
#include <jni.h>

#include <android/log.h>
#include <android/native_window.h>
#include <android/native_window_jni.h>

#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <time.h>

#include "nva_abi.h"

#define SHELL_LOG_TAG "nva-shell"
#define SHELL_LOG_FILE "nva-shell.log"

/*
 * The version the phone shows (#103): `build_apk.sh` passes the same string to
 * `aapt2 --version-name`, so the line below and Android's "App info" agree. The
 * fallback only covers a compile outside that script — a package built that way
 * is not a package anyone should install.
 */
#ifndef SHELL_VERSION
#define SHELL_VERSION "0.0 (unversioned build)"
#endif

/*
 * One line: `HH:MM:SS.mmm ` plus the message. 320 bytes holds the longest line
 * here (the device line from Java) with room to spare; anything longer is cut
 * rather than wrapped, because a wrapped line is two lines that a reader
 * cannot tell from two events.
 */
#define SHELL_LOG_LINE 320

/* An app-private path is ~60 bytes; anything longer degrades to logcat. */
#define SHELL_LOG_PATH 400

/* Rotation threshold, from the ticket. */
#define SHELL_LOG_LIMIT (256u * 1024u)

/* Created in onCreate, destroyed in onDestroy. NULL means "not created yet". */
static nva_app *g_app;

/*
 * The log file, and the byte count that decides when to rotate. A counter is
 * cheaper than asking the filesystem on every line, and it can only drift if a
 * write comes back short — which resyncs it below.
 */
static char g_log_path[SHELL_LOG_PATH];
static FILE *g_log_file;
static unsigned long g_log_bytes;

/* Declared ahead of the writer so gcc checks every call site's format string. */
static void shell_log(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void shell_rotate(void);
static void shell_log_close(void);

/*
 * One instant, read once, shared by both formatters. Splitting the read between
 * them would let an event line land under a run header stamped by a different
 * clock reading.
 *
 * 0 on failure. The callers disagree on what that means, on purpose: an event
 * line is dropped rather than written undated (a forensic line with no time is
 * worse than no line), while the run header falls back to "unknown" because the
 * header is the file's anchor and must exist.
 */
static int shell_now_local(struct tm *out, long *millis)
{
    struct timespec now;
    if (clock_gettime(CLOCK_REALTIME, &now) != 0) {
        return 0;
    }
    if (localtime_r(&now.tv_sec, out) == NULL) {
        return 0;
    }
    *millis = now.tv_nsec / 1000000L;
    return 1;
}

/* `HH:MM:SS.mmm ` — the prefix on every event line. */
static size_t shell_format_clock(char *out, size_t cap)
{
    struct tm local;
    long millis;
    if (!shell_now_local(&local, &millis)) {
        return 0;
    }
    int n = snprintf(out, cap, "%02d:%02d:%02d.%03ld ", local.tm_hour, local.tm_min,
                     local.tm_sec, millis);
    if (n <= 0 || (size_t)n >= cap) {
        return 0;
    }
    return (size_t)n;
}

/*
 * `yyyy-mm-ddThh:mm:ss.mmm+hh:mm` for the run header. The zone offset is not
 * decoration: two logs from two phones are only separable by a stamp that says
 * which wall clock it is on.
 */
static size_t shell_format_stamp(char *out, size_t cap)
{
    struct tm local;
    long millis;
    if (!shell_now_local(&local, &millis)) {
        return 0;
    }
    /* `%z` is the portable spelling; it yields `+0800`, and the colon is ours. */
    char zone[8];
    if (strftime(zone, sizeof zone, "%z", &local) != 5) {
        return 0;
    }
    int n = snprintf(out, cap, "%04d-%02d-%02dT%02d:%02d:%02d.%03ld%c%c%c:%c%c",
                     local.tm_year + 1900, local.tm_mon + 1, local.tm_mday, local.tm_hour,
                     local.tm_min, local.tm_sec, millis, zone[0], zone[1], zone[2], zone[3],
                     zone[4]);
    if (n <= 0 || (size_t)n >= cap) {
        return 0;
    }
    return (size_t)n;
}

/* One more than the headers the file already holds. */
static int shell_count_runs(const char *path)
{
    FILE *f = fopen(path, "r");
    if (f == NULL) {
        return 0;
    }
    int runs = 0;
    /* A header is ~43 bytes, so a 64-byte buffer never splits one. */
    char line[64];
    while (fgets(line, (int)sizeof line, f) != NULL) {
        if (strncmp(line, "=== run ", 8) == 0) {
            runs++;
        }
    }
    fclose(f);
    return runs;
}

/*
 * Rename to `.1` and reopen. Called from the writer, so it must not log through
 * the file it is closing — `shell_log` writes to logcat only while the file is
 * NULL, which is exactly the state this function leaves behind on failure.
 */
static void shell_rotate(void)
{
    if (g_log_file == NULL) {
        return;
    }
    fclose(g_log_file);
    g_log_file = NULL;
    g_log_bytes = 0;

    char rotated[SHELL_LOG_PATH + 2];
    int n = snprintf(rotated, sizeof rotated, "%s.1", g_log_path);
    if (n <= 0 || (size_t)n >= sizeof rotated) {
        shell_log("log rotate failed: %s", strerror(errno));
    } else if (rename(g_log_path, rotated) != 0) {
        /*
         * POSIX rename replaces the destination; not every C library does, and
         * one that refuses would leave the old file at its full size with a
         * fresh counter — unbounded growth, the thing the limit exists to
         * prevent. Dropping the previous generation first keeps the promise
         * either way: one old file, never two, and never a live log that grew
         * past the limit because rotation kept failing.
         */
        if (remove(rotated) != 0 || rename(g_log_path, rotated) != 0) {
            shell_log("log rotate failed: %s", strerror(errno));
        }
    }
    g_log_file = fopen(g_log_path, "a");
    if (g_log_file == NULL) {
        shell_log("log reopen failed: %s", strerror(errno));
        return;
    }
    /*
     * Measured, not assumed zero. A failed rename leaves the old file in place
     * at its full size; a counter at 0 would let it grow another whole limit
     * before the next attempt, and fail again — doubling forever, which is the
     * "unbounded" the ticket rules out. `shell_log_open` measures the same way.
     */
    if (fseek(g_log_file, 0, SEEK_END) == 0) {
        long at = ftell(g_log_file);
        g_log_bytes = at > 0 ? (unsigned long)at : 0;
    }
}

/*
 * The writer: the one place that writes `g_log_file`. `line` holds the text and
 * `len` its length, with room for a newline behind it; this appends the newline
 * and hands the whole thing to one `fwrite`, so a line either lands whole or
 * forces a re-measure — it is never half a line waiting for the next one to
 * complete it. Both exits receive these exact bytes, which is what keeps the
 * file and logcat comparable.
 */
static void shell_emit(char *line, size_t len)
{
    if (len == 0) {
        return;
    }
    __android_log_print(ANDROID_LOG_INFO, SHELL_LOG_TAG, "%s", line);

    if (g_log_file == NULL) {
        return;
    }
    /* The newline counts toward the limit, and the check runs before the write,
     * so the file never passes it. */
    if (g_log_bytes + len + 1 > SHELL_LOG_LIMIT) {
        shell_rotate();
        if (g_log_file == NULL) {
            return;
        }
    }
    line[len++] = '\n';
    size_t wrote = fwrite(line, 1, len, g_log_file);
    fflush(g_log_file);
    if (wrote == len) {
        g_log_bytes += (unsigned long)len;
    } else {
        /* A short write means the count is a guess; re-measure instead of
         * rotating at a size the file is not actually at. */
        long at = ftell(g_log_file);
        g_log_bytes = at > 0 ? (unsigned long)at : 0;
    }
}

static void shell_log(const char *fmt, ...)
{
    char line[SHELL_LOG_LINE];
    size_t used = shell_format_clock(line, sizeof line);
    if (used == 0) {
        return;
    }
    /* Two bytes are kept back for the newline the writer appends and the
     * terminator, so even a truncated message still fits. */
    size_t room = sizeof line - used - 2;
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(line + used, room + 1, fmt, ap);
    va_end(ap);
    if (n < 0) {
        return;
    }
    shell_emit(line, used + ((size_t)n < room ? (size_t)n : room));
}

/*
 * The one formatter entry that takes already-built text: Java's lines arrive
 * here. `text` may hold `%` — it is data, never a format. Newlines and carriage
 * returns become spaces, because a line break inside Java text would split the
 * line the writer is about to emit, and a payload starting with `=== run `
 * would forge a run number. Java only ever sends this file's own diagnostics,
 * so this is a guard against a future caller, not a workaround for a current
 * one.
 */
static void shell_log_text(const char *text)
{
    char line[SHELL_LOG_LINE];
    size_t used = shell_format_clock(line, sizeof line);
    if (used == 0) {
        return;
    }
    size_t room = sizeof line - used - 2;
    size_t n = 0;
    for (const char *p = text; *p != '\0' && n < room; p++) {
        line[used + n] = (*p == '\n' || *p == '\r') ? ' ' : *p;
        n++;
    }
    line[used + n] = '\0';
    shell_emit(line, used + n);
}

/*
 * The names the runtime's `nva_platform.log` levels get. The ABI numbers
 * (`NVA_LOG_*` in `include/nva_abi.h`) are ours, not Android's `Log.*`
 * priorities, so this function is where one becomes the other. A level outside
 * the four is named rather than mapped: a silently wrong priority is worse than
 * an odd-looking word.
 */
static const char *shell_level_name(uint32_t level)
{
    switch (level) {
    case NVA_LOG_ERROR: return "error";
    case NVA_LOG_WARN:  return "warn";
    case NVA_LOG_INFO:  return "info";
    case NVA_LOG_DEBUG: return "debug";
    default:            return "unknown";
    }
}

/*
 * `nva_platform.log` (#101): the runtime's own lines. They go through
 * `shell_log`, so they carry the same timestamp prefix and reach the same file
 * and the same logcat exit as this file's events; `[rt]` is what tells the two
 * apart when the file is read back. `msg` is one line by contract — the runtime
 * builds it from literals and numbers — and is passed as data, never as a
 * format.
 *
 * Called synchronously on whichever thread called `nva_app_*` (see the
 * "Thread" section above): every export is reached from the main thread today,
 * which is what keeps `shell_emit`'s single-writer assumption true.
 */
static void shell_platform_log(void *user, uint32_t level, const char *tag, const char *msg)
{
    (void)user;
    shell_log("[rt] %s %s: %s", shell_level_name(level), tag != NULL ? tag : "?",
              msg != NULL ? msg : "(null)");
}

/*
 * What the runtime borrows until `nva_app_destroy`, so it must be static
 * storage: a local in `appCreate` would dangle the moment that call returned.
 * The audio, rumble and clock slots stay zero — this ticket adds the log seam
 * only, and a placeholder that answered successfully would be a lie the runtime
 * has no way to detect.
 */
static const nva_platform g_platform = {
    .size = (uint32_t)sizeof(nva_platform),
    .version = NVA_PLATFORM_VERSION,
    .log = shell_platform_log,
};

/*
 * One stats snapshot as one line (#93).
 *
 * `when` is `attach` or `detach`: the same fields are read at the two moments
 * the ticket names, and the word is what tells the two lines apart when the
 * file is read back. `presents` is the field the acceptance check looks at — at
 * attach it must be at least 1, and after a detach it must be the value the run
 * reached, not a larger one.
 *
 * The struct is zeroed and then given its own `size` / `version`: the runtime
 * writes every field today, but a caller that handed it a half-initialised
 * struct would be reading its own memory back as a count on any field a later
 * version appends.
 *
 * Called on the main thread only, like everything else here: `attach` has
 * already waited for the first frame, so this line is a snapshot, not a poll.
 */
static void shell_log_stats(const char *when)
{
    if (g_app == NULL) {
        return;
    }
    nva_stats stats;
    memset(&stats, 0, sizeof stats);
    stats.size = (uint32_t)sizeof(stats);
    stats.version = NVA_STATS_VERSION;
    nva_status status = nva_app_get_stats(g_app, &stats);
    if (status != NVA_OK) {
        shell_log("%s stats failed: status=%u", when, (unsigned)status);
        return;
    }
    shell_log("%s stats presents=%u generation=%u submits=%llu", when,
              (unsigned)stats.presents, (unsigned)stats.surface_generation,
              (unsigned long long)stats.submits);
}

/*
 * Open the file and write the run header. Called with the directory the Java
 * side passed to `appCreate`, before the runtime is created, so the header and
 * `app created` land in the same file.
 *
 * Failing to open is not fatal: the shell is a product and the log is evidence.
 * A device with a full disk must still start, and it says so on logcat.
 */
static void shell_log_open(const char *dir)
{
    /*
     * A create that lands while a file is still open would leak the old handle
     * and leave two writers behind it. `appDestroy` closes, so this only fires
     * if a create ever runs without a destroy in between.
     */
    if (g_log_file != NULL) {
        shell_log_close();
    }
    if (dir == NULL || dir[0] == '\0') {
        shell_log("no data dir: the log file stays closed");
        return;
    }
    int n = snprintf(g_log_path, sizeof g_log_path, "%s/%s", dir, SHELL_LOG_FILE);
    if (n <= 0 || (size_t)n >= sizeof g_log_path) {
        g_log_path[0] = '\0';
        shell_log("data dir is too long: the log file stays closed");
        return;
    }

    int run = shell_count_runs(g_log_path) + 1;
    g_log_file = fopen(g_log_path, "a");
    if (g_log_file == NULL) {
        shell_log("log open failed: %s", strerror(errno));
        return;
    }
    if (fseek(g_log_file, 0, SEEK_END) == 0) {
        long at = ftell(g_log_file);
        g_log_bytes = at > 0 ? (unsigned long)at : 0;
    } else {
        g_log_bytes = 0;
    }

    char stamp[40];
    if (shell_format_stamp(stamp, sizeof stamp) == 0) {
        snprintf(stamp, sizeof stamp, "unknown");
    }
    /* One byte kept back for the newline the writer appends. */
    char header[SHELL_LOG_LINE];
    int h = snprintf(header, sizeof header - 1, "=== run %d %s ===", run, stamp);
    if (h > 0 && (size_t)h < sizeof header - 1) {
        shell_emit(header, (size_t)h);
    }
}

static void shell_log_close(void)
{
    if (g_log_file != NULL) {
        fclose(g_log_file);
        g_log_file = NULL;
    }
    g_log_bytes = 0;
}

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
    shell_log("JNI_OnLoad");
    return JNI_VERSION_1_6;
}

JNIEXPORT jstring JNICALL
Java_com_bingnnvjn_nva_MainActivity_shellInfo(JNIEnv *env, jobject thiz)
{
    (void)thiz;
    /* No status to report: the entry cannot fail. It still logs one line, so
     * every entry point appears in the file with the same shape. */
    shell_log("shellInfo status=%u", (unsigned)NVA_OK);
    return (*env)->NewStringUTF(env, "NVA shell " SHELL_VERSION " (JNI_OnLoad ran)");
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_appCreate(JNIEnv *env, jobject thiz, jstring data_dir)
{
    (void)thiz;
    if (g_app != NULL) {
        shell_log("app already created status=%u", (unsigned)NVA_ERR_INVALID_ARGUMENT);
        return NVA_ERR_INVALID_ARGUMENT;
    }

    const char *dir = NULL;
    if (data_dir != NULL) {
        dir = (*env)->GetStringUTFChars(env, data_dir, NULL);
    }
    /*
     * The log file opens here, before the runtime exists, so the run header and
     * `app created` land in the same file. `shell_log_open` copies `dir` into
     * its own buffer, which is why releasing the Java string below is still
     * correct.
     */
    shell_log_open(dir);
    /*
     * No callbacks: the shell has nothing to start when a surface arrives, and
     * the two logs around this call are the evidence this ticket asks for. The
     * engine is started by `nva_app_attach_window` itself (#93), not by
     * `on_surface_ready`; the callback stays a shell-side hook the runtime
     * raises when one is set.
     *
     * `platform` is what lets the runtime say anything at all (#101): without
     * it every refused transition stayed inside Rust and the file showed
     * nothing. `g_platform` is static, so the borrow survives to `destroy`.
     *
     * `desc` and `dir` live for this call only: the runtime copies the callback
     * pointers and the platform's `user` / `log` fields and stores neither
     * `data_dir` nor the descriptor, so releasing `dir` right after is correct.
     * It does keep calling `log` with that `user` until `nva_app_destroy`, which
     * is the other reason `g_platform` is static rather than a local here.
     */
    nva_app_desc desc = {
        .size = (uint32_t)sizeof(desc),
        .version = NVA_APP_DESC_VERSION,
        .flags = 0,
        .reserved = 0,
        .data_dir = dir,
        .callbacks = NULL,
        .platform = &g_platform,
    };
    nva_app *app = NULL;
    nva_status status = nva_app_create(&desc, &app);
    if (dir != NULL) {
        (*env)->ReleaseStringUTFChars(env, data_dir, dir);
    }
    if (status != NVA_OK) {
        /* The file stays open: a run that failed to create is still a run, and
         * its log is what says why. */
        shell_log("app create failed: status=%u", (unsigned)status);
        return (jint)status;
    }

    g_app = app;
    shell_log("app created status=%u", (unsigned)NVA_OK);
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
        shell_log("%s without an app status=%u", step, (unsigned)NVA_ERR_INVALID_ARGUMENT);
        return NVA_ERR_INVALID_ARGUMENT;
    }
    nva_status status = call(g_app);
    if (status == NVA_OK) {
        shell_log("%s status=%u", event, (unsigned)status);
    } else {
        shell_log("%s failed: status=%u", step, (unsigned)status);
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
        shell_log("surface attached without an app status=%u", (unsigned)NVA_ERR_INVALID_ARGUMENT);
        return NVA_ERR_INVALID_ARGUMENT;
    }
    if (surface == NULL) {
        shell_log("surface is null status=%u", (unsigned)NVA_ERR_SURFACE_LOST);
        return NVA_ERR_SURFACE_LOST;
    }

    ANativeWindow *window = ANativeWindow_fromSurface(env, surface);
    if (window == NULL) {
        shell_log("ANativeWindow_fromSurface failed status=%u", (unsigned)NVA_ERR_SURFACE_LOST);
        return NVA_ERR_SURFACE_LOST;
    }

    nva_status status = nva_app_attach_window(g_app, window, (uint32_t)width, (uint32_t)height,
                                             (uint32_t)generation);
    if (status != NVA_OK) {
        /* The runtime did not take the window, so its reference is ours alone. */
        ANativeWindow_release(window);
        shell_log("surface attach failed: status=%u", (unsigned)status);
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
    shell_log("surface attached (generation=%d) %dx%d status=%u", (int)generation, (int)width,
              (int)height, (unsigned)NVA_OK);
    /* #93: the runtime started its render thread inside the attach and waited
     * up to a second for the first frame, so this snapshot is normally
     * `presents >= 1`. A 0 here is the evidence for a failed or late first
     * frame, not a reason to fail the attach. */
    shell_log_stats("attach");
    return (jint)NVA_OK;
}

JNIEXPORT jint JNICALL
Java_com_bingnnvjn_nva_MainActivity_windowDetached(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    if (g_app == NULL) {
        shell_log("detach without an app status=%u", (unsigned)NVA_ERR_INVALID_ARGUMENT);
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
        shell_log("detach failed: status=%u", (unsigned)status);
        return (jint)status;
    }

    /* §7.3: the shell releases only after detach has returned. */
    if (g_window != NULL) {
        ANativeWindow_release(g_window);
        g_window = NULL;
    }
    shell_log("detached (generation=%u) status=%u", (unsigned)g_generation, (unsigned)NVA_OK);
    /* #93: detach stopped and joined the render thread, so `presents` here is
     * final for this run. On HOME the pair of these two lines is the evidence
     * that the loop stopped instead of erroring out. */
    shell_log_stats("detach");
    g_generation = 0;
    return (jint)NVA_OK;
}

/*
 * `onDestroy`. Android normally delivers `surfaceDestroyed` first; when it does
 * not, the activity is going away with a live surface, so the reference is
 * dropped here rather than leaked.
 *
 * The file closes last: `destroyed` has to reach it, and a later run in the
 * same process reopens with a fresh header.
 */
JNIEXPORT void JNICALL
Java_com_bingnnvjn_nva_MainActivity_appDestroy(JNIEnv *env, jobject thiz)
{
    (void)env;
    (void)thiz;
    if (g_app == NULL) {
        /* Not a record line: nothing was destroyed. Worth saying anyway, since
         * it means `appCreate` never succeeded. */
        shell_log("destroy without an app");
        shell_log_close();
        return;
    }
    if (g_window != NULL) {
        /* Also a diagnostic, not a record: the normal teardown is
         * `surfaceDestroyed` then `onDestroy`, and this line says it was not
         * that. Kept for #93, which rebuilds the swapchain on attach. */
        shell_log("destroy with a window still attached (generation=%u)",
                  (unsigned)g_generation);
        ANativeWindow_release(g_window);
        g_window = NULL;
    }
    nva_app_destroy(g_app);
    g_app = NULL;
    shell_log("destroyed status=%u", (unsigned)NVA_OK);
    shell_log_close();
}

/*
 * The Java side's way into the same file (#99). It does not open the file
 * itself: a second writer would interleave half lines with this one, and the
 * ticket rules that out. The text goes through `shell_log_text`, which treats
 * it as data and keeps it on one line.
 */
JNIEXPORT void JNICALL
Java_com_bingnnvjn_nva_MainActivity_nativeLog(JNIEnv *env, jobject thiz, jstring text)
{
    (void)thiz;
    if (text == NULL) {
        return;
    }
    const char *chars = (*env)->GetStringUTFChars(env, text, NULL);
    if (chars == NULL) {
        return;
    }
    shell_log_text(chars);
    (*env)->ReleaseStringUTFChars(env, text, chars);
}
