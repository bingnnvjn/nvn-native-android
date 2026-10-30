package com.bingnnvjn.abislice;

import android.app.Activity;
import android.os.Build;
import android.os.Bundle;
import android.text.TextUtils;
import android.util.Log;
import android.view.ViewGroup;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

/**
 * #30 ABI slice APK shell.
 *
 * The native side is the same C guest as the CI runs. It links libnvn_host.so
 * and calls the NVN-shaped C ABI. The APK only adds a screen and buttons, so
 * the phone can run the spike without adb.
 */
public class MainActivity extends Activity {
    static {
        System.loadLibrary("nvn_host");
        System.loadLibrary("abi_slice_jni");
    }

    private native String runNative(String mode, String cacheDir);
    private native String runAbort(String cacheDir);

    private TextView output;

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);

        LinearLayout root = new LinearLayout(this);
        root.setOrientation(LinearLayout.VERTICAL);

        TextView info = new TextView(this);
        info.setText("ABI slice\n"
                + Build.MODEL + " / Android " + Build.VERSION.RELEASE + "\n"
                + TextUtils.join(", ", Build.SUPPORTED_ABIS));
        info.setTextIsSelectable(true);
        root.addView(info);

        LinearLayout buttons = new LinearLayout(this);
        Button all = new Button(this);
        all.setText("Run all");
        Button gate = new Button(this);
        gate.setText("Run gate");
        Button abort = new Button(this);
        abort.setText("Abort test");
        buttons.addView(all);
        buttons.addView(gate);
        buttons.addView(abort);
        root.addView(buttons);

        output = new TextView(this);
        output.setTextIsSelectable(true);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(output);
        root.addView(scroll, new LinearLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, 0, 1.0f));

        setContentView(root);

        all.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { runMode("all", false); }
        });
        gate.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { runMode("gate", false); }
        });
        abort.setOnClickListener(new android.view.View.OnClickListener() {
            @Override public void onClick(android.view.View v) { runMode("unimplemented", true); }
        });

        /* Phone-only path: run the five assertions as soon as the app opens. */
        runMode("all", false);
    }

    private void runMode(String mode, boolean abort) {
        output.setText("Running " + mode + " ...");
        new Thread(new Runnable() {
            @Override public void run() {
                String text;
                try {
                    String cache = getCacheDir().getAbsolutePath();
                    text = abort ? runAbort(cache) : runNative(mode, cache);
                } catch (Throwable t) {
                    text = "native call failed:\n" + Log.getStackTraceString(t);
                }
                for (String line : text.split("\n")) {
                    Log.i("abi-slice", line);
                }
                final String shown = text;
                runOnUiThread(new Runnable() {
                    @Override public void run() { output.setText(shown); }
                });
            }
        }, "abi-slice").start();
    }
}
