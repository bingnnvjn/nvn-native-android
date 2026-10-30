/*
 * JNI bridge for the #30 ABI slice APK.
 *
 * guest.c is compiled into this shared object with -Dmain=abi_slice_guest_main.
 * The JNI thread redirects stdout/stderr to a file, runs the guest, and returns
 * the log to Java. The abort test forks a child so SIGABRT does not kill the
 * app process.
 */
#include <jni.h>
#include <android/log.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>
#include <errno.h>

#define TAG "abi-slice"

extern int abi_slice_guest_main(int argc, char **argv);
extern void abi_slice_guest_reset(void);

static int redirect_to_file(const char *path, int *saved_out, int *saved_err)
{
    *saved_out = dup(STDOUT_FILENO);
    *saved_err = dup(STDERR_FILENO);
    if (*saved_out < 0 || *saved_err < 0) {
        return -1;
    }
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    if (fd < 0) {
        return -1;
    }
    if (dup2(fd, STDOUT_FILENO) < 0 || dup2(fd, STDERR_FILENO) < 0) {
        close(fd);
        return -1;
    }
    close(fd);
    return 0;
}

static void restore_stdio(int saved_out, int saved_err)
{
    fflush(NULL);
    if (saved_out >= 0) {
        dup2(saved_out, STDOUT_FILENO);
        close(saved_out);
    }
    if (saved_err >= 0) {
        dup2(saved_err, STDERR_FILENO);
        close(saved_err);
    }
}

static jstring read_log(JNIEnv *env, const char *path)
{
    FILE *f = fopen(path, "rb");
    if (!f) {
        char msg[256];
        snprintf(msg, sizeof msg, "cannot read %s: %s", path, strerror(errno));
        return (*env)->NewStringUTF(env, msg);
    }
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (n < 0) {
        n = 0;
    }
    char *buf = (char *)malloc((size_t)n + 1);
    if (!buf) {
        fclose(f);
        return (*env)->NewStringUTF(env, "out of memory");
    }
    size_t got = fread(buf, 1, (size_t)n, f);
    fclose(f);
    buf[got] = '\0';
    jstring out = (*env)->NewStringUTF(env, buf);
    free(buf);
    return out;
}

static void append_line(const char *path, const char *line)
{
    FILE *f = fopen(path, "a");
    if (f) {
        fputs(line, f);
        fputc('\n', f);
        fclose(f);
    }
}

JNIEXPORT jstring JNICALL
Java_com_bingnnvjn_abislice_MainActivity_runNative(JNIEnv *env, jobject thiz,
                                                   jstring jmode,
                                                   jstring jcache)
{
    (void)thiz;
    if (!jmode || !jcache) {
        return (*env)->NewStringUTF(env, "missing mode/cacheDir");
    }
    const char *mode = (*env)->GetStringUTFChars(env, jmode, NULL);
    const char *cache = (*env)->GetStringUTFChars(env, jcache, NULL);
    if (!mode || !cache) {
        return (*env)->NewStringUTF(env, "GetStringUTFChars failed");
    }

    char path[1024];
    snprintf(path, sizeof path, "%s/abi-slice-%s.log", cache, mode);

    int so = -1, se = -1;
    if (redirect_to_file(path, &so, &se) != 0) {
        (*env)->ReleaseStringUTFChars(env, jmode, mode);
        (*env)->ReleaseStringUTFChars(env, jcache, cache);
        return (*env)->NewStringUTF(env, "cannot open log file");
    }

    abi_slice_guest_reset();
    char *argv[] = { (char *)"abi_guest", (char *)mode, NULL };
    int rc = abi_slice_guest_main(2, argv);
    fflush(NULL);
    restore_stdio(so, se);

    char line[128];
    snprintf(line, sizeof line, "EXIT %d", rc);
    append_line(path, line);
    __android_log_print(ANDROID_LOG_INFO, TAG, "mode=%s exit=%d", mode, rc);

    (*env)->ReleaseStringUTFChars(env, jmode, mode);
    (*env)->ReleaseStringUTFChars(env, jcache, cache);
    return read_log(env, path);
}

JNIEXPORT jstring JNICALL
Java_com_bingnnvjn_abislice_MainActivity_runAbort(JNIEnv *env, jobject thiz,
                                                  jstring jcache)
{
    (void)thiz;
    if (!jcache) {
        return (*env)->NewStringUTF(env, "missing cacheDir");
    }
    const char *cache = (*env)->GetStringUTFChars(env, jcache, NULL);
    if (!cache) {
        return (*env)->NewStringUTF(env, "GetStringUTFChars failed");
    }

    char path[1024];
    snprintf(path, sizeof path, "%s/abi-slice-abort.log", cache);

    int so = -1, se = -1;
    if (redirect_to_file(path, &so, &se) != 0) {
        (*env)->ReleaseStringUTFChars(env, jcache, cache);
        return (*env)->NewStringUTF(env, "cannot open abort log");
    }

    fflush(NULL);
    pid_t pid = fork();
    if (pid == 0) {
        char *argv[] = { (char *)"abi_guest", (char *)"unimplemented", NULL };
        int rc = abi_slice_guest_main(2, argv);
        _exit(rc);
    }

    int status = 0;
    if (pid > 0) {
        (void)waitpid(pid, &status, 0);
    }
    restore_stdio(so, se);

    char line[256];
    if (pid < 0) {
        snprintf(line, sizeof line, "ABORT fork failed: %s", strerror(errno));
    } else if (WIFSIGNALED(status)) {
        snprintf(line, sizeof line, "ABORT signaled=%d signal=%d (SIGABRT=6)",
                 WIFSIGNALED(status) ? 1 : 0, WTERMSIG(status));
    } else {
        snprintf(line, sizeof line, "ABORT exited=%d (expected SIGABRT=6)",
                 WIFEXITED(status) ? WEXITSTATUS(status) : -1);
    }
    append_line(path, line);
    __android_log_print(ANDROID_LOG_INFO, TAG, "%s", line);

    (*env)->ReleaseStringUTFChars(env, jcache, cache);
    return read_log(env, path);
}
