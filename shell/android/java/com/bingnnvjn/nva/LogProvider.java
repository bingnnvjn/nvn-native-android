package com.bingnnvjn.nva;

import android.content.ContentProvider;
import android.content.ContentValues;
import android.database.Cursor;
import android.database.MatrixCursor;
import android.net.Uri;
import android.os.ParcelFileDescriptor;
import android.provider.OpenableColumns;

import java.io.File;
import java.io.FileNotFoundException;

/**
 * Serves the shell's device log to the system share sheet (#99).
 *
 * <p>This is a hand-written {@link ContentProvider}, not {@code FileProvider}.
 * The no-Gradle build puts only {@code android.jar} on the javac bootclasspath,
 * and {@code androidx.core.content.FileProvider} is not in it
 * (CODING_STANDARDS.md, "Android / JNI"). A provider also needs no resource
 * pipeline: {@code FileProvider} would want an {@code @xml/file_paths} table,
 * and this build has no {@code res/} at all.
 *
 * <p>It is read-only on purpose. The log has exactly one writer — the C side —
 * and a second one would interleave half lines with it
 * (shell/android/jni/nva_shell_jni.c).
 *
 * <p>The provider is not a permission. Nothing in the manifest widens: the
 * {@code <provider>} entry declares {@code exported="false"} and
 * {@code grantUriPermissions="true"}, so only an app the shell has handed a
 * read grant for this one URI can open the file, and only for as long as the
 * receiving activity lives.
 */
public final class LogProvider extends ContentProvider {
    /** The authority the manifest declares. */
    public static final String AUTHORITY = "com.bingnnvjn.nva.logs";

    /** The one file this provider serves; the C side writes it. */
    public static final String NAME = "nva-shell.log";

    /** {@code content://com.bingnnvjn.nva.logs/nva-shell.log}. */
    public static Uri uri() {
        return new Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(NAME).build();
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        return "text/plain";
    }

    /**
     * The file itself. {@code getFilesDir()} is the same directory the activity
     * hands to {@code appCreate}, so the provider and the writer cannot drift
     * onto two different paths.
     */
    private File file() {
        return new File(getContext().getFilesDir(), NAME);
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("read-only provider, mode=" + mode);
        }
        if (!NAME.equals(uri.getLastPathSegment())) {
            throw new FileNotFoundException("only " + NAME + " is served: " + uri);
        }
        File log = file();
        if (!log.isFile()) {
            throw new FileNotFoundException(NAME + " has not been written yet");
        }
        return ParcelFileDescriptor.open(log, ParcelFileDescriptor.MODE_READ_ONLY);
    }

    /**
     * {@link OpenableColumns} is what the share sheet reads to label the
     * attachment and size it; without it the entry shows up unnamed.
     */
    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs,
                        String sortOrder) {
        String[] columns = projection != null
                ? projection
                : new String[] {OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE};
        MatrixCursor cursor = new MatrixCursor(columns, 1);
        MatrixCursor.RowBuilder row = cursor.newRow();
        for (String column : columns) {
            if (OpenableColumns.DISPLAY_NAME.equals(column)) {
                row.add(NAME);
            } else if (OpenableColumns.SIZE.equals(column)) {
                row.add(file().length());
            } else {
                row.add(null);
            }
        }
        return cursor;
    }

    @Override
    public Uri insert(Uri uri, ContentValues values) {
        throw new UnsupportedOperationException("read-only provider");
    }

    @Override
    public int update(Uri uri, ContentValues values, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("read-only provider");
    }

    @Override
    public int delete(Uri uri, String selection, String[] selectionArgs) {
        throw new UnsupportedOperationException("read-only provider");
    }
}
