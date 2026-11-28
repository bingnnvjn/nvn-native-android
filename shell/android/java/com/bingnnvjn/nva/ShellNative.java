package com.bingnnvjn.nva;

import android.view.Surface;

/**
 * The shell's JNI entry points, shared by {@link MainActivity} and
 * {@link M0bCheckActivity} (#109).
 *
 * <p>Before #109 every native method was declared on {@code MainActivity}, which
 * meant the acceptance activity could not reach the same runtime without a
 * second set of exports over the same static state. These methods are static so
 * one declaration serves both activities; the JNI names in
 * {@code shell/android/jni/nva_shell_jni.c} follow this class.
 *
 * <p>The library is loaded here, once, and the runtime first: the JNI library
 * links against it, and Android resolves that dependency at load time, so
 * loading the runtime first turns a missing library into one clear
 * {@code UnsatisfiedLinkError} naming the runtime instead of a confusing failure
 * inside the shell library.
 *
 * <p>No lambdas, no androidx: this build puts only {@code android.jar} on the
 * javac bootclasspath (CODING_STANDARDS.md, "Android / JNI").
 */
public final class ShellNative {
    /** {@code NVA_OK} from include/nva_abi.h. */
    public static final int NVA_OK = 0;

    static {
        System.loadLibrary("nva_runtime");
        System.loadLibrary("nva_shell_jni");
    }

    private ShellNative() {}

    /** The #46 evidence line: proves JNI_OnLoad ran and a real call works. */
    public static native String shellInfo();

    /** The commit embedded at build time ({@code -DSHELL_COMMIT}). */
    public static native String shellCommit();

    /**
     * One single-line JSON snapshot of the runtime's counters, or {@code null}
     * when there is no runtime object. Fields: {@code presents},
     * {@code surface_generation}, {@code attached}, {@code submits}.
     */
    public static native String shellStatsJson();

    public static native int appCreate(String dataDir);

    public static native int appStart();

    public static native int appPause();

    public static native int appResume();

    public static native int appStop();

    public static native int windowAttached(Surface surface, int width, int height, int generation);

    public static native int windowDetached();

    public static native void appDestroy();

    /** Writes one line through the C side's log formatter (#99). */
    public static native void nativeLog(String line);
}
