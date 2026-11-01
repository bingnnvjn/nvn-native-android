/*
 * Minimal JNI library for the product shell (#46).
 *
 * This is deliberately the smallest thing that satisfies docs/GATES.md §3:
 * the library exports JNI_OnLoad, and the activity makes one real JNI call so
 * the export is a live entry point rather than a symbol nobody loads. No
 * runtime, no guest, no Vulkan here.
 *
 * JNI_OnLoad is loaded by the Android VM when MainActivity's static block runs
 * System.loadLibrary("nva_shell_jni") (docs/ARCHITECTURE.md §3.7).
 */
#include <jni.h>

#ifdef __ANDROID__
#include <android/log.h>
#define SHELL_LOG(...) \
    __android_log_print(ANDROID_LOG_INFO, "nva-shell", __VA_ARGS__)
#else
#define SHELL_LOG(...) ((void)0)
#endif

#define SHELL_VERSION "0.1.0-m0a"

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
