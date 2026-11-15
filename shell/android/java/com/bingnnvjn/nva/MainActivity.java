package com.bingnnvjn.nva;

import android.app.Activity;
import android.content.ClipData;
import android.content.Intent;
import android.graphics.Rect;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;

/**
 * Product shell activity: a window, and the app lifecycle behind it (#91).
 *
 * <p>#46 shipped a text card whose whole job was to prove that the APK installs
 * and that {@code JNI_OnLoad} runs. That proof is kept — {@link #shellInfo()} is
 * still called once in {@link #onCreate} — but the content view is now a
 * {@link SurfaceView}, and its surface events and the activity lifecycle are
 * forwarded to the runtime through the {@code nva_app_*} exports
 * (docs/ARCHITECTURE.md §3.7, §7.1).
 *
 * <p>No lambdas here: the no-Gradle build puts {@code android.jar} on the javac
 * bootclasspath, which has no {@code LambdaMetafactory}
 * (CODING_STANDARDS.md, "Android / JNI"). The callback below is an anonymous
 * inner class for that reason, not for style.
 *
 * <p>Surface generation. The shell owns the counter (§3.7), and the rule is:
 *
 * <ul>
 *   <li>{@code surfaceCreated} advances it — a new Surface is a new window
 *       identity, which is what HOME-then-back looks like.
 *   <li>{@code surfaceChanged} advances it only when the geometry of the
 *       attached window actually changes — that is rotation. Android always
 *       delivers one {@code surfaceChanged} right after {@code surfaceCreated},
 *       with the same geometry; counting that would make the first surface
 *       generation 2 and break the {@code surface_generation == 1} gate
 *       (docs/GATES.md §4).
 *   <li>{@code surfaceDestroyed} does not advance it — the window is gone, not
 *       replaced.
 * </ul>
 *
 * <p>Rotation only reaches this class as {@code surfaceChanged} because the
 * manifest declares {@code configChanges}: without it the system would destroy
 * and recreate the activity, which restarts the generation at 1.
 *
 * <p>The window reference itself is not touched from Java: it is acquired and
 * released in the JNI layer, next to the runtime calls that need it
 * (shell/android/jni/nva_shell_jni.c).
 *
 * <p>#99 adds one thing to the picture: the events in logcat also go to a file
 * in the app's private directory, and a long press opens the share sheet with
 * that file attached, so a device run can be retrieved without adb. Every line
 * this class logs goes through {@link #note(String)} into the C formatter —
 * Java never opens the file itself. See docs/DEVICE-LOGS.md.
 */
public class MainActivity extends Activity {
    private static final String TAG = "nva-shell";

    /** {@code NVA_OK} from include/nva_abi.h. */
    private static final int NVA_OK = 0;

    static {
        // The JNI library links against the runtime, and Android resolves that
        // dependency at load time; loading it first turns a missing library into
        // one clear UnsatisfiedLinkError naming the runtime instead of a
        // confusing failure inside the shell library.
        System.loadLibrary("nva_runtime");
        System.loadLibrary("nva_shell_jni");
    }

    /** 0 means "no surface has been attached yet". */
    private int generation;

    /** Geometry of the surface the runtime currently holds; 0 means none. */
    private int attachedWidth;
    private int attachedHeight;

    private native String shellInfo();

    private native int appCreate(String dataDir);

    private native int appStart();

    private native int appPause();

    private native int appResume();

    private native int appStop();

    private native int windowAttached(Surface surface, int width, int height, int generation);

    private native int windowDetached();

    private native void appDestroy();

    /** Writes one line through the C side's log formatter (#99). */
    private native void nativeLog(String line);

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        // The #46 evidence stays alive: JNI_OnLoad ran and a real JNI call
        // works. It is logged further down, after appCreate, because only then
        // is the log file open and the line recorded there too (#99).
        String info;
        try {
            info = shellInfo();
        } catch (Throwable t) {
            info = "native call failed: " + t;
        }

        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(new SurfaceEvents());
        // Long press is the whole retrieval path (#99): no menu, no button, no
        // cable. A named inner class, not a lambda — this build has no
        // LambdaMetafactory (see the class comment). Setting the listener also
        // makes the view long-clickable, which is what lets onTouchEvent take
        // the DOWN without any other click handling here.
        view.setOnLongClickListener(new ShareLog());
        setContentView(view);

        // After the view is in place, so a surface event can never arrive before
        // the runtime has an object to hand it to. This call also opens the log
        // file, which is why the device line comes after it.
        //
        // A refusal is recorded once, by the JNI layer, next to the call that
        // failed: logging it here as well would put two differently-worded
        // lines on one event, and the ticket asks for one line per entry.
        appCreate(getFilesDir().getAbsolutePath());
        note(info + " on " + Build.MODEL + " / Android " + Build.VERSION.RELEASE);
    }

    @Override
    protected void onStart() {
        super.onStart();
        // Refusals are recorded by the JNI layer (see onCreate).
        appStart();
    }

    @Override
    protected void onResume() {
        super.onResume();
        appResume();
    }

    @Override
    protected void onPause() {
        // Before super: the runtime hears about the pause while the surface is
        // still valid, which is the order #93's present loop needs.
        appPause();
        super.onPause();
    }

    @Override
    protected void onStop() {
        appStop();
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        appDestroy();
        super.onDestroy();
    }

    /**
     * Every Java-side line goes to the C formatter (#99) rather than straight
     * to logcat: one formatter, two exits, so the file and logcat never tell
     * two different stories. Java must not open the log file itself — two
     * writers would interleave half lines with the C side.
     *
     * <p>Falls back to logcat when the library is not loaded, so a diagnostic
     * line can never be the thing that crashes the shell.
     */
    private void note(String line) {
        try {
            nativeLog(line);
        } catch (Throwable t) {
            Log.i(TAG, line);
        }
    }

    /**
     * Hand the current surface to the runtime, with the current generation.
     *
     * <p>The geometry is a parameter rather than something read from the holder:
     * during {@code surfaceCreated} the frame may not be measured yet, while
     * {@code surfaceChanged} is told the size outright. Reading it here would
     * attach a 0-shaped window on the created leg and then count the following
     * changed as a rotation.
     */
    private void attach(SurfaceHolder holder, int width, int height) {
        if (width <= 0 || height <= 0) {
            note("surface has no size yet: " + width + "x" + height);
            return;
        }
        // See onCreate: the JNI layer owns the record for a refused call.
        if (windowAttached(holder.getSurface(), width, height, generation) == NVA_OK) {
            attachedWidth = width;
            attachedHeight = height;
        }
    }

    private boolean hasAttachedWindow() {
        return attachedWidth > 0 && attachedHeight > 0;
    }

    /** The three surface events, forwarded to the runtime. */
    private final class SurfaceEvents implements SurfaceHolder.Callback {
        @Override
        public void surfaceCreated(SurfaceHolder holder) {
            generation++;
            attachedWidth = 0;
            attachedHeight = 0;
            Rect frame = holder.getSurfaceFrame();
            attach(holder, frame.width(), frame.height());
        }

        @Override
        public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            if (width <= 0 || height <= 0) {
                note("surface changed with no size yet: " + width + "x" + height);
                return;
            }
            if (hasAttachedWindow() && width == attachedWidth && height == attachedHeight) {
                // The redundant changed that follows every created.
                return;
            }
            if (hasAttachedWindow()) {
                generation++;
            }
            attach(holder, width, height);
        }

        @Override
        public void surfaceDestroyed(SurfaceHolder holder) {
            if (!hasAttachedWindow()) {
                return;
            }
            windowDetached();
            attachedWidth = 0;
            attachedHeight = 0;
        }
    }

    /**
     * Long press on the picture → the system share sheet with the log attached
     * (#99). This is the whole retrieval path: it replaces the adb cable, not
     * the operator's three actions.
     *
     * <p>The URI comes from {@link LogProvider}, not from a file path: the log
     * lives in the app's private directory, which no other app can read, and a
     * {@code content://} URI plus a read grant is the only way to hand it over.
     * No permission is involved — the grant is per-URI and lasts as long as the
     * receiving activity.
     *
     * <p>{@code ClipData} is set by hand instead of trusting the platform to
     * migrate {@code EXTRA_STREAM} for us: the grant only covers URIs the intent
     * actually carries, and a share that silently arrives without read access
     * would fail on the other side, where this code cannot see it.
     */
    private final class ShareLog implements View.OnLongClickListener {
        @Override
        public boolean onLongClick(View view) {
            Uri log = LogProvider.uri();
            Intent send = new Intent(Intent.ACTION_SEND);
            send.setType("text/plain");
            send.putExtra(Intent.EXTRA_STREAM, log);
            send.setClipData(ClipData.newRawUri(LogProvider.NAME, log));
            send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            Intent chooser = Intent.createChooser(send, "Share device log");
            chooser.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            try {
                startActivity(chooser);
            } catch (Throwable t) {
                // No share target installed, or the activity is already gone.
                note("share failed: " + t);
            }
            // Consumed either way: nothing else is bound to a long press here.
            return true;
        }
    }
}
