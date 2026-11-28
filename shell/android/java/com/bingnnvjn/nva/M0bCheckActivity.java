package com.bingnnvjn.nva;

import android.app.Activity;
import android.content.ClipData;
import android.content.Context;
import android.content.Intent;
import android.graphics.Bitmap;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.system.Os;
import android.system.OsConstants;
import android.util.Log;
import android.view.Gravity;
import android.view.PixelCopy;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.widget.Button;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONObject;

import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.text.SimpleDateFormat;
import java.util.Date;
import java.util.Locale;
import java.util.TimeZone;
import java.util.zip.ZipEntry;
import java.util.zip.ZipOutputStream;

/**
 * The M0b acceptance screen (#109): collect raw evidence on a phone, with no
 * adb and no judgement. The PC verifier ({@code nva-verify device
 * --from-bundle}) is the only thing that decides; this class only records what
 * happened.
 *
 * <p>Flow:
 *
 * <ol>
 *   <li>Attach a {@link SurfaceView}. The runtime renders the first green frame.
 *   <li>Wait for a present ({@code presents >= 1}) and take the window centre
 *       with {@link PixelCopy}. PixelCopy needs no permission.
 *   <li>Write the identity and the first-phase evidence ({@code m0b.json},
 *       {@code screen.raw}) into the app's private directory.
 *   <li>Ask the user to press HOME. {@code surfaceDestroyed} detaches the
 *       window, reads the frozen counters and upgrades the evidence to
 *       {@code complete}.
 * </ol>
 *
 * <p>HOME is a user action: no {@code moveTaskToBack}, no background activity.
 * Reopening the app is how the zip reaches the share sheet, which is the only
 * moment Android lets an app raise one.
 *
 * <p>No lambdas and no androidx: the no-Gradle build has only {@code android.jar}
 * on the javac bootclasspath (CODING_STANDARDS.md, "Android / JNI"). Callbacks
 * are named inner classes.
 *
 * <p>Surface generation follows {@link MainActivity}'s rule exactly: created
 * advances, changed advances only on a real geometry change, destroyed does not.
 */
public class M0bCheckActivity extends Activity {
    static final String TAG = "nva-shell";

    /** The evidence file names, also the zip entry names. */
    static final String EVIDENCE_JSON = "m0b.json";
    static final String EVIDENCE_RAW = "screen.raw";
    static final String EVIDENCE_LOG = "nva-shell.log";
    private static final String SENT_SUFFIX = ".sent";

    private static final int NVA_OK = ShellNative.NVA_OK;
    /** Retries for the first present: 20 × 100 ms = 2 s. */
    private static final int PRESENT_ATTEMPTS = 20;
    private static final long PRESENT_RETRY_MS = 100L;

    private SurfaceView surfaceView;
    private TextView prompt;
    private Handler handler;

    private int generation;
    private int attachedWidth;
    private int attachedHeight;
    /**
     * True once the operator pressed 开始 on the M0a declaration screen. No
     * runtime object is created before this: cancellation leaves no evidence
     * and no log line.
     */
    private boolean confirmed;
    /** True once phase-one evidence exists; guards the complete write. */
    private boolean started;
    /**
     * True once {@link #onStop} fired. A stopped activity is the end of the
     * run (HOME, recents or finishing). Rotation does not call it — with
     * {@code configChanges} it only delivers surface callbacks — so a rotation
     * still re-attaches instead of being mistaken for HOME.
     */
    private boolean stopping;
    /**
     * True once the run is finalized (stopping and evidence written). Late
     * PixelCopy callbacks and retry runnables must not downgrade the complete
     * evidence to partial, and a resumed activity shares instead of rendering.
     */
    private boolean detached;

    private String runId;
    private String tsUtc;
    private String commit;
    private String apkSha256;
    private String deviceModel;
    private String pageSize;
    private String buildVersion;

    private int attachGeneration;
    private int attachPresents;
    private int detachPresents;
    private int pixel;
    private boolean pixelSampled;
    private String pixelError;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        // A process restart can relaunch this activity with the old evidence
        // still on disk (the task's top activity is restored). Share it before
        // starting a new run; overwriting it here would destroy the only record
        // of what happened.
        if (hasPendingEvidence(this)) {
            sharePendingEvidence(this);
            finish();
            return;
        }
        handler = new Handler(Looper.getMainLooper());
        showDeclaration();
    }

    /**
     * The M0a operator declaration. Nothing is created or written until the
     * operator presses 开始: the declaration is an action, not something the
     * code may assert on the operator's behalf (GATES §1).
     */
    private void showDeclaration() {
        TextView message = new TextView(this);
        message.setTextSize(18);
        message.setPadding(48, 48, 48, 48);
        message.setText("本验收假设 M0a 机器门已绿。\n\n"
                + "点「开始」继续采集；点「取消」退出，不写任何证据。");

        Button start = new Button(this);
        start.setText("开始");
        start.setOnClickListener(new ConfirmStart());
        Button cancel = new Button(this);
        cancel.setText("取消");
        cancel.setOnClickListener(new ConfirmCancel());

        LinearLayout column = new LinearLayout(this);
        column.setOrientation(LinearLayout.VERTICAL);
        column.setPadding(24, 24, 24, 24);
        column.addView(message);
        column.addView(start);
        column.addView(cancel);

        FrameLayout root = new FrameLayout(this);
        FrameLayout.LayoutParams params = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT);
        params.gravity = Gravity.CENTER;
        root.addView(column, params);
        setContentView(root);
    }

    private final class ConfirmStart implements View.OnClickListener {
        @Override
        public void onClick(View view) {
            if (confirmed) {
                return;
            }
            confirmed = true;
            startAcceptance();
        }
    }

    private final class ConfirmCancel implements View.OnClickListener {
        @Override
        public void onClick(View view) {
            finish();
        }
    }

    /**
     * Begins the run after the operator's declaration. The identity and the
     * timestamps are taken here, at the moment the run really starts, so the
     * evidence does not carry the time the declaration screen was merely open.
     */
    private void startAcceptance() {
        // A new run must not inherit a previous frame or JSON. Without this,
        // a run that fails to sample would still ship an old green raw and the
        // PC could call it green; `pixel_sampled` alone would only catch it if
        // the JSON were also fresh.
        deleteEvidenceFile(EVIDENCE_JSON);
        deleteEvidenceFile(EVIDENCE_RAW);
        deleteEvidenceFile("sha256.txt");

        runId = utcStamp("yyyyMMdd'T'HHmmss'Z'") + "-" + android.os.Process.myPid();
        tsUtc = utcStamp("yyyy-MM-dd'T'HH:mm:ss'Z'");
        collectIdentity();

        // create/start/resume happen before the surface view exists, so no
        // surface callback can reach a runtime that is not ready. onStart and
        // onResume already ran while the declaration was showing; with
        // `confirmed` false they did nothing, so the transitions are made here.
        int status = ShellNative.appCreate(getFilesDir().getAbsolutePath());
        if (status != NVA_OK) {
            note("acceptance app create failed: status=" + status);
        } else {
            ShellNative.appStart();
            ShellNative.appResume();
            note("acceptance run " + runId + " on " + deviceModel
                    + " / Android " + buildVersion);
        }
        showSurface();
    }

    private void showSurface() {
        FrameLayout root = new FrameLayout(this);
        surfaceView = new SurfaceView(this);
        surfaceView.getHolder().addCallback(new SurfaceEvents());
        root.addView(
                surfaceView,
                new FrameLayout.LayoutParams(
                        FrameLayout.LayoutParams.MATCH_PARENT,
                        FrameLayout.LayoutParams.MATCH_PARENT));
        prompt = new TextView(this);
        prompt.setTextSize(18);
        prompt.setText("等待首帧…");
        FrameLayout.LayoutParams promptParams = new FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT);
        promptParams.gravity = Gravity.CENTER;
        root.addView(prompt, promptParams);
        setContentView(root);
    }

    @Override
    protected void onStart() {
        super.onStart();
        if (confirmed && !detached) {
            ShellNative.appStart();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        // Tapping the app icon after HOME may bring this task back with this
        // activity on top rather than starting MainActivity. The surface was
        // destroyed (detached), the evidence is on disk, and reopening is the
        // only allowed moment to raise the share sheet — so share and leave.
        if (detached) {
            if (hasPendingEvidence(this)) {
                sharePendingEvidence(this);
            }
            finish();
            return;
        }
        if (confirmed) {
            stopping = false;
            ShellNative.appResume();
        }
    }

    @Override
    protected void onPause() {
        // Before super, like MainActivity: the runtime hears about the pause
        // while the surface is still valid.
        if (confirmed && !detached) {
            ShellNative.appPause();
        }
        super.onPause();
    }

    @Override
    protected void onStop() {
        // A stop is the end of the run. The detach snapshot may still be
        // pending in surfaceDestroyed (if it has not fired yet), which rewrites
        // the same complete evidence once the window is really gone.
        if (confirmed && !detached) {
            stopping = true;
            ShellNative.appStop();
            if (started) {
                writeEvidence("complete");
                detached = true;
            }
        }
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        if (confirmed) {
            ShellNative.appDestroy();
        }
        super.onDestroy();
    }

    private void note(String line) {
        try {
            ShellNative.nativeLog(line);
        } catch (Throwable t) {
            Log.i(TAG, line);
        }
    }

    /** Identity fields the gate names. Unknowns stay {@code unknown}. */
    private void collectIdentity() {
        deviceModel = Build.MODEL == null ? "unknown" : Build.MODEL;
        buildVersion = Build.VERSION.RELEASE == null ? "unknown" : Build.VERSION.RELEASE;
        commit = safeCommit();
        apkSha256 = apkSha256();
        pageSize = pageSize();
    }

    private String safeCommit() {
        try {
            String value = ShellNative.shellCommit();
            return value == null || value.isEmpty() ? "unknown" : value;
        } catch (Throwable t) {
            return "unknown";
        }
    }

    /** SHA-256 of the installed APK; the operator compares it on the PC. */
    private String apkSha256() {
        try {
            String path = getApplicationInfo().sourceDir;
            return sha256Hex(new File(path));
        } catch (Throwable t) {
            note("apk hash failed: " + t);
            return "unknown";
        }
    }

    private String pageSize() {
        try {
            long size = Os.sysconf(OsConstants._SC_PAGE_SIZE);
            return size > 0 ? Long.toString(size) : "unknown";
        } catch (Throwable t) {
            note("page size failed: " + t);
            return "unknown";
        }
    }

    /** Writes the current evidence. Phase one says {@code partial}; the HOME
     * detach upgrades it to {@code complete}. */
    private void writeEvidence(String phase) {
        JSONObject json = new JSONObject();
        try {
            json.put("scenario", "m0b");
            json.put("source", "app");
            json.put("commit", commit);
            json.put("apk_sha256", apkSha256);
            json.put("pack_id", "none");
            json.put("build_id", "none");
            json.put("device_model", deviceModel);
            json.put("driver_version", "unknown");
            json.put("vulkan_api", "unknown");
            json.put("page_size", pageSize);
            json.put("build_version", buildVersion);
            json.put("run_id", runId);
            json.put("ts_utc", tsUtc);
            // The operator declared M0a green on the confirmation screen; this
            // is their action, recorded as GATES §1's stoppable manual step.
            json.put("m0a_machine_green", true);
            JSONObject declared = new JSONObject();
            declared.put("step", "m0a_declared");
            declared.put("ts", tsUtc);
            json.put("operator_input", declared);
            json.put("surface_generation", attachGeneration);
            json.put("presents", attachPresents);
            json.put("presents_before", attachPresents);
            json.put("presents_after", detachPresents > 0 ? detachPresents : attachPresents);
            json.put("pixel_rgba", String.format(Locale.US, "0x%08x", pixel));
            json.put("pixel_sampled", pixelSampled);
            if (pixelError != null) {
                json.put("pixel_error", pixelError);
            }
            json.put("detached_submit", logHasDetachedSubmit());
            json.put("crashed", false);
            json.put("evidence", phase);
        } catch (Throwable t) {
            note("evidence json failed: " + t);
            return;
        }
        writeFile(EVIDENCE_JSON, json.toString().getBytes(StandardCharsets.UTF_8));
    }

    private void deleteEvidenceFile(String name) {
        File file = new File(getFilesDir(), name);
        if (file.exists() && !file.delete()) {
            note("evidence cleanup failed for " + name);
        }
    }

    private void writeFile(String name, byte[] bytes) {
        File file = new File(getFilesDir(), name);
        FileOutputStream out = null;
        try {
            out = new FileOutputStream(file);
            out.write(bytes);
        } catch (IOException e) {
            note("evidence write failed for " + name + ": " + e);
        } finally {
            closeQuietly(out);
        }
    }

    private JSONObject readStats() {
        try {
            String json = ShellNative.shellStatsJson();
            return json == null ? null : new JSONObject(json);
        } catch (Throwable t) {
            note("stats read failed: " + t);
            return null;
        }
    }

    private boolean hasAttachedWindow() {
        return attachedWidth > 0 && attachedHeight > 0;
    }

    private void attach(SurfaceHolder holder, int width, int height) {
        if (width <= 0 || height <= 0) {
            note("acceptance surface has no size yet: " + width + "x" + height);
            return;
        }
        if (ShellNative.windowAttached(holder.getSurface(), width, height, generation)
                != NVA_OK) {
            return;
        }
        attachedWidth = width;
        attachedHeight = height;
        attachGeneration = generation;
        // Phase one exists before the pixel wait, so a user who presses HOME
        // immediately still leaves a bundle the PC can judge (and reject).
        started = true;
        writeEvidence("partial");
        awaitFirstFrame(0);
    }

    private void awaitFirstFrame(int attempt) {
        if (detached) {
            return;
        }
        JSONObject stats = readStats();
        if (stats != null) {
            int presents = stats.optInt("presents", 0);
            if (presents >= 1) {
                attachPresents = presents;
                int reported = stats.optInt("surface_generation", attachGeneration);
                if (reported > 0) {
                    attachGeneration = reported;
                }
                requestPixelCopy();
                return;
            }
        }
        if (attempt >= PRESENT_ATTEMPTS) {
            recordPixelFailure("no present within " + (PRESENT_ATTEMPTS
                    * PRESENT_RETRY_MS) + " ms");
            writeRaw(null);
            finishPixelPhase();
            return;
        }
        handler.postDelayed(new RetryFirstFrame(attempt + 1), PRESENT_RETRY_MS);
    }

    /** A named retry, because this build has no LambdaMetafactory. */
    private final class RetryFirstFrame implements Runnable {
        private final int attempt;

        RetryFirstFrame(int attempt) {
            this.attempt = attempt;
        }

        @Override
        public void run() {
            awaitFirstFrame(attempt);
        }
    }

    private void requestPixelCopy() {
        if (detached) {
            return;
        }
        int width = surfaceView.getWidth() > 0 ? surfaceView.getWidth() : attachedWidth;
        int height = surfaceView.getHeight() > 0 ? surfaceView.getHeight() : attachedHeight;
        if (width <= 0 || height <= 0) {
            recordPixelFailure("surface has no size for PixelCopy");
            writeRaw(null);
            finishPixelPhase();
            return;
        }
        Bitmap bitmap;
        try {
            bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888);
        } catch (Throwable t) {
            recordPixelFailure("bitmap allocation failed: " + t);
            writeRaw(null);
            finishPixelPhase();
            return;
        }
        try {
            PixelCopy.request(surfaceView, bitmap, new CopyFinished(bitmap), handler);
        } catch (Throwable t) {
            // The callback will not run; finish the phase here.
            recordPixelFailure("PixelCopy request failed: " + t);
            writeRaw(bitmap);
            finishPixelPhase();
        }
    }

    private final class CopyFinished implements PixelCopy.OnPixelCopyFinishedListener {
        private final Bitmap bitmap;

        CopyFinished(Bitmap bitmap) {
            this.bitmap = bitmap;
        }

        @Override
        public void onPixelCopyFinished(int result) {
            if (detached) {
                return;
            }
            if (result == PixelCopy.SUCCESS) {
                int width = bitmap.getWidth();
                int height = bitmap.getHeight();
                int[] argb = new int[width * height];
                bitmap.getPixels(argb, 0, width, 0, 0, width, height);
                pixel = argb[(height / 2) * width + (width / 2)];
                pixelSampled = true;
                pixelError = null;
                note("acceptance pixel sampled 0x" + String.format(Locale.US, "%08x", pixel));
            } else {
                // A black or failed copy is recorded, never called green. The
                // PC verifier compares the raw frame and exits 2.
                recordPixelFailure("PixelCopy result " + result);
            }
            writeRaw(bitmap);
            finishPixelPhase();
        }
    }

    /**
     * Records a failed pixel. The caller still writes a raw frame and finishes
     * the phase, so the bundle is complete and the failure is visible to the
     * PC verifier (which compares the raw centre and exits 2).
     */
    private void recordPixelFailure(String reason) {
        pixelSampled = false;
        pixelError = reason;
        pixel = 0;
        note("acceptance pixel failed: " + reason);
    }

    private void finishPixelPhase() {
        if (detached) {
            return;
        }
        writeEvidence("partial");
        if (pixelSampled) {
            prompt.setText("请按一次 HOME 键结束采集");
        } else {
            prompt.setText("像素采集失败，仍请按 HOME 结束");
        }
    }

    /**
     * Copies the frame into the app's raw format: {@code u32 width, u32 height,
     * u32 format = 1} followed by each pixel as an ARGB int in little-endian
     * byte order. Android's ARGB int packs A<<24|R<<16|G<<8|B, so the four
     * bytes are B, G, R, A; a little-endian u32 read of them gives the ARGB
     * value back, which is what {@code centre_pixel} compares with
     * {@code 0xff00ff00}.
     */
    private void writeRaw(Bitmap bitmap) {
        if (bitmap == null) {
            ByteBuffer empty = ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN);
            empty.putInt(1);
            empty.putInt(1);
            empty.putInt(1);
            empty.putInt(0);
            writeFile(EVIDENCE_RAW, empty.array());
            return;
        }
        int width = bitmap.getWidth();
        int height = bitmap.getHeight();
        int[] argb = new int[width * height];
        bitmap.getPixels(argb, 0, width, 0, 0, width, height);
        ByteBuffer buffer = ByteBuffer.allocate(12 + width * height * 4)
                .order(ByteOrder.LITTLE_ENDIAN);
        buffer.putInt(width);
        buffer.putInt(height);
        buffer.putInt(1);
        for (int value : argb) {
            buffer.put((byte) (value & 0xff));
            buffer.put((byte) ((value >> 8) & 0xff));
            buffer.put((byte) ((value >> 16) & 0xff));
            buffer.put((byte) ((value >> 24) & 0xff));
        }
        writeFile(EVIDENCE_RAW, buffer.array());
    }

    /**
     * Whether the shell log already holds the {@code detached_submit} signal.
     * The runtime writes it inside {@code windowDetached}, so reading after that
     * call sees it. Only the last run counts: an earlier run's signal must not
     * leak into this result.
     */
    private boolean logHasDetachedSubmit() {
        File log = new File(getFilesDir(), EVIDENCE_LOG);
        if (!log.isFile()) {
            return false;
        }
        try {
            byte[] bytes = readAll(log);
            String text = new String(bytes, StandardCharsets.UTF_8);
            int start = text.lastIndexOf("=== run ");
            String tail = start >= 0 ? text.substring(start) : text;
            return tail.contains("\"event\":\"detached_submit\"");
        } catch (Throwable t) {
            note("log read failed: " + t);
            return false;
        }
    }

    /** The three surface events, with MainActivity's generation rule. */
    private final class SurfaceEvents implements SurfaceHolder.Callback {
        @Override
        public void surfaceCreated(SurfaceHolder holder) {
            if (detached) {
                return;
            }
            generation++;
            attachedWidth = 0;
            attachedHeight = 0;
            Rect frame = holder.getSurfaceFrame();
            attach(holder, frame.width(), frame.height());
        }

        @Override
        public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            if (detached) {
                return;
            }
            if (width <= 0 || height <= 0) {
                note("acceptance surface changed with no size: " + width + "x" + height);
                return;
            }
            if (hasAttachedWindow() && width == attachedWidth && height == attachedHeight) {
                return;
            }
            if (hasAttachedWindow()) {
                generation++;
            }
            attach(holder, width, height);
        }

        @Override
        public void surfaceDestroyed(SurfaceHolder holder) {
            handler.removeCallbacksAndMessages(null);
            if (hasAttachedWindow()) {
                ShellNative.windowDetached();
                attachedWidth = 0;
                attachedHeight = 0;
            }
            // Rotation (configChanges) delivers this without onStop; the next
            // surfaceCreated re-attaches, so only a stopped activity finalizes.
            if (!stopping) {
                return;
            }
            detached = true;
            JSONObject stats = readStats();
            detachPresents = stats != null ? stats.optInt("presents", attachPresents)
                    : attachPresents;
            if (started) {
                writeEvidence("complete");
                note("acceptance detach captured");
            }
        }
    }

    // --- static evidence retention and sharing ------------------------------

    /** Whether a bundle from a previous acceptance run is waiting to be shared. */
    public static boolean hasPendingEvidence(Context context) {
        return new File(context.getFilesDir(), EVIDENCE_JSON).isFile();
    }

    /**
     * Builds the zip, raises the share sheet and marks the evidence sent.
     *
     * <p>Called from {@link MainActivity#onCreate} on the next launch after
     * HOME. If the process died mid-run, the evidence is {@code partial} and
     * the log has no detach snapshot; the zip still goes out, and the PC
     * verifier rejects it. That is the honest outcome, so this method does not
     * pretend to complete it.
     */
    public static void sharePendingEvidence(Activity activity) {
        if (!hasPendingEvidence(activity)) {
            return;
        }
        File dir = activity.getFilesDir();
        try {
            File evidence = ensureFile(dir, EVIDENCE_JSON, "{}".getBytes(StandardCharsets.UTF_8));
            File raw = ensureFile(dir, EVIDENCE_RAW, placeholderRaw());
            File log = ensureFile(dir, EVIDENCE_LOG, new byte[0]);

            byte[] evidenceBytes = readAll(evidence);
            byte[] rawBytes = readAll(raw);
            byte[] logBytes = readAll(log);

            // Sorted by file name, the order the ledger and the verifier use.
            String manifest =
                    manifestLine(EVIDENCE_JSON, evidenceBytes)
                            + manifestLine(EVIDENCE_LOG, logBytes)
                            + manifestLine(EVIDENCE_RAW, rawBytes);

            File zip = new File(dir, EvidenceProvider.NAME);
            writeZip(zip, evidenceBytes, rawBytes, logBytes, manifest);
            shareZip(activity);
            markSent(evidence);
        } catch (Throwable t) {
            Log.i(TAG, "bundle share failed: " + t);
        }
    }

    private static File ensureFile(File dir, String name, byte[] fallback) throws IOException {
        File file = new File(dir, name);
        if (!file.isFile()) {
            FileOutputStream out = new FileOutputStream(file);
            try {
                out.write(fallback);
            } finally {
                out.close();
            }
        }
        return file;
    }

    private static byte[] placeholderRaw() {
        ByteBuffer buffer = ByteBuffer.allocate(16).order(ByteOrder.LITTLE_ENDIAN);
        buffer.putInt(1);
        buffer.putInt(1);
        buffer.putInt(1);
        buffer.putInt(0);
        return buffer.array();
    }

    private static void markSent(File evidence) {
        File directory = evidence.getParentFile();
        File sent = new File(directory, evidence.getName() + SENT_SUFFIX);
        if (!evidence.renameTo(sent)) {
            // A rename that fails must not leave the file pending, or every
            // launch would offer the same bundle again.
            deleteQuietly(evidence);
        }
        // The raw frame and log belong to this run only. Leaving them behind
        // is how a later, unsampled run could find an old green frame; the
        // next run deletes them again at start, and this keeps them from
        // being mistaken for current evidence in between.
        deleteQuietly(new File(directory, EVIDENCE_RAW));
        deleteQuietly(new File(directory, EVIDENCE_LOG));
    }

    private static void deleteQuietly(File file) {
        if (file.exists()) {
            // Best effort: the evidence is already inside the shared zip.
            file.delete();
        }
    }

    private static void writeZip(File zip, byte[] evidence, byte[] raw, byte[] log,
            String manifest) throws IOException {
        ZipOutputStream out = new ZipOutputStream(
                new BufferedOutputStream(new FileOutputStream(zip)));
        try {
            addEntry(out, EVIDENCE_JSON, evidence);
            addEntry(out, EVIDENCE_RAW, raw);
            addEntry(out, EVIDENCE_LOG, log);
            addEntry(out, "sha256.txt", manifest.getBytes(StandardCharsets.UTF_8));
        } finally {
            out.close();
        }
    }

    private static void addEntry(ZipOutputStream out, String name, byte[] data)
            throws IOException {
        out.putNextEntry(new ZipEntry(name));
        out.write(data);
        out.closeEntry();
    }

    private static void shareZip(Activity activity) {
        Uri uri = EvidenceProvider.uri();
        Intent send = new Intent(Intent.ACTION_SEND);
        send.setType("application/zip");
        send.putExtra(Intent.EXTRA_STREAM, uri);
        send.setClipData(ClipData.newRawUri(EvidenceProvider.NAME, uri));
        send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        Intent chooser = Intent.createChooser(send, "Share M0b evidence");
        chooser.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
        activity.startActivity(chooser);
    }

    // --- small helpers ------------------------------------------------------

    private static String manifestLine(String name, byte[] bytes) {
        return sha256Hex(bytes) + "  " + name + "\n";
    }

    private static String utcStamp(String pattern) {
        SimpleDateFormat format = new SimpleDateFormat(pattern, Locale.US);
        format.setTimeZone(TimeZone.getTimeZone("UTC"));
        return format.format(new Date());
    }

    private static String sha256Hex(File file) throws IOException {
        InputStream in = new FileInputStream(file);
        try {
            MessageDigest digest;
            try {
                digest = MessageDigest.getInstance("SHA-256");
            } catch (Exception e) {
                throw new IOException("SHA-256 is unavailable", e);
            }
            byte[] buffer = new byte[8192];
            int read;
            while ((read = in.read(buffer)) > 0) {
                digest.update(buffer, 0, read);
            }
            return hex(digest.digest());
        } finally {
            in.close();
        }
    }

    private static String sha256Hex(byte[] bytes) {
        try {
            MessageDigest digest = MessageDigest.getInstance("SHA-256");
            return hex(digest.digest(bytes));
        } catch (Exception e) {
            throw new IllegalStateException("SHA-256 is unavailable", e);
        }
    }

    private static String hex(byte[] bytes) {
        StringBuilder out = new StringBuilder(bytes.length * 2);
        for (byte value : bytes) {
            out.append(String.format(Locale.US, "%02x", value & 0xff));
        }
        return out.toString();
    }

    private static byte[] readAll(File file) throws IOException {
        long length = file.length();
        if (length > Integer.MAX_VALUE) {
            throw new IOException("evidence file is too large: " + file);
        }
        byte[] bytes = new byte[(int) length];
        InputStream in = new FileInputStream(file);
        try {
            int offset = 0;
            while (offset < bytes.length) {
                int read = in.read(bytes, offset, bytes.length - offset);
                if (read < 0) {
                    break;
                }
                offset += read;
            }
            return bytes;
        } finally {
            in.close();
        }
    }

    private static void closeQuietly(FileOutputStream out) {
        if (out != null) {
            try {
                out.close();
            } catch (IOException ignored) {
                // Best effort: the write already reported its own failure.
            }
        }
    }
}
