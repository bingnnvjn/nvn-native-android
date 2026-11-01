package com.bingnnvjn.nva;

import android.app.Activity;
import android.os.Build;
import android.os.Bundle;
import android.widget.TextView;

/**
 * Product shell activity (#46).
 *
 * It proves exactly three things and nothing more: the APK installs, the
 * manifest is the product one, and the JNI library's {@code JNI_OnLoad} runs.
 * Window, input, audio, SAF and save handling are L6 work and stay out of this
 * ticket (docs/ARCHITECTURE.md §3.7, #37 Out of Scope).
 *
 * No lambdas here: the no-Gradle build puts {@code android.jar} on the javac
 * bootclasspath, which has no {@code LambdaMetafactory}
 * (CODING_STANDARDS.md, "Android / JNI").
 */
public class MainActivity extends Activity {
    static {
        System.loadLibrary("nva_shell_jni");
    }

    /** Called once at startup so JNI_OnLoad and one real JNI call both run. */
    private native String shellInfo();

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        String info;
        try {
            info = shellInfo();
        } catch (Throwable t) {
            info = "native call failed: " + t;
        }

        TextView view = new TextView(this);
        view.setText(info + "\n" + Build.MODEL + " / Android " + Build.VERSION.RELEASE);
        view.setTextIsSelectable(true);
        setContentView(view);
    }
}
