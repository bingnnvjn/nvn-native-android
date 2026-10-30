package com.bingnnvjn.abislice;

/** Linux JNI smoke harness. Same native names as the APK; no Android imports. */
public class MainActivity {
    static {
        System.loadLibrary("nvn_host");
        System.loadLibrary("abi_slice_jni");
    }

    private native String runNative(String mode, String cacheDir);
    private native String runAbort(String cacheDir);

    public static void main(String[] args) {
        if (args.length != 1) {
            System.err.println("usage: MainActivity <cacheDir>");
            System.exit(2);
        }
        MainActivity m = new MainActivity();
        System.out.println("=== ALL ===");
        System.out.println(m.runNative("all", args[0]));
        System.out.println("=== GATE ===");
        System.out.println(m.runNative("gate", args[0]));
        System.out.println("=== ABORT ===");
        System.out.println(m.runAbort(args[0]));
    }
}
