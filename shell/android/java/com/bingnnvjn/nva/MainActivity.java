package com.bingnnvjn.nva;

import android.app.Activity;
import android.graphics.Rect;
import android.os.Build;
import android.os.Bundle;
import android.util.Log;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;

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
 */
public class MainActivity extends Activity {
    private static final String TAG = "nva-shell";

    /** {@code NVA_OK} from include/nva_abi.h. */
    private static final int NVA_OK = 0;

    /** What each log line starts with when a runtime call is refused. */
    private static final String REFUSED = "refused: status=";

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

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        // The #46 evidence stays alive: JNI_OnLoad ran and a real JNI call works.
        try {
            Log.i(TAG, shellInfo() + " on " + Build.MODEL + " / Android " + Build.VERSION.RELEASE);
        } catch (Throwable t) {
            Log.i(TAG, "native call failed: " + t);
        }

        SurfaceView view = new SurfaceView(this);
        view.getHolder().addCallback(new SurfaceEvents());
        setContentView(view);

        // After the view is in place, so a surface event can never arrive before
        // the runtime has an object to hand it to.
        int status = appCreate(getFilesDir().getAbsolutePath());
        if (status != NVA_OK) {
            Log.i(TAG, "app create " + REFUSED + status);
        }
    }

    @Override
    protected void onStart() {
        super.onStart();
        report("start", appStart());
    }

    @Override
    protected void onResume() {
        super.onResume();
        report("resume", appResume());
    }

    @Override
    protected void onPause() {
        // Before super: the runtime hears about the pause while the surface is
        // still valid, which is the order #93's present loop needs.
        report("pause", appPause());
        super.onPause();
    }

    @Override
    protected void onStop() {
        report("stop", appStop());
        super.onStop();
    }

    @Override
    protected void onDestroy() {
        appDestroy();
        super.onDestroy();
    }

    private void report(String what, int status) {
        if (status != NVA_OK) {
            Log.i(TAG, what + " " + REFUSED + status);
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
            Log.i(TAG, "surface has no size yet: " + width + "x" + height);
            return;
        }
        int status = windowAttached(holder.getSurface(), width, height, generation);
        if (status != NVA_OK) {
            report("attach", status);
            return;
        }
        attachedWidth = width;
        attachedHeight = height;
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
                Log.i(TAG, "surface changed with no size yet: " + width + "x" + height);
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
            report("detach", windowDetached());
            attachedWidth = 0;
            attachedHeight = 0;
        }
    }
}
