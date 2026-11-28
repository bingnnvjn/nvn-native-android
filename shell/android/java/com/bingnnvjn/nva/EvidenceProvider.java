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
 * Serves the M0b evidence zip to the system share sheet (#109).
 *
 * <p>A hand-written {@link ContentProvider}, not {@code FileProvider}: the
 * no-Gradle build has no androidx and no {@code res/} for a file-paths table
 * (CODING_STANDARDS.md, "Android / JNI"; the same reason {@link LogProvider}
 * exists).
 *
 * <p>It serves exactly one name, {@value #NAME}, read-only. The manifest
 * declares {@code exported="false"} and {@code grantUriPermissions="true"}, so
 * only a receiver the activity handed a per-URI read grant can open it, and
 * only for that activity's lifetime. No permission is declared or widened.
 */
public final class EvidenceProvider extends ContentProvider {
    /** The authority the manifest declares. */
    public static final String AUTHORITY = "com.bingnnvjn.nva.evidence";

    /** The one file this provider serves: the evidence zip. */
    public static final String NAME = "m0b-bundle.zip";

    /** {@code content://com.bingnnvjn.nva.evidence/m0b-bundle.zip}. */
    public static Uri uri() {
        return new Uri.Builder().scheme("content").authority(AUTHORITY).appendPath(NAME).build();
    }

    @Override
    public boolean onCreate() {
        return true;
    }

    @Override
    public String getType(Uri uri) {
        return isServed(uri) ? "application/zip" : null;
    }

    private File file() {
        return new File(getContext().getFilesDir(), NAME);
    }

    /**
     * The one name this provider serves, checked by every method that reads a
     * URI. Without it a grant would cover any path under the authority.
     */
    private static boolean isServed(Uri uri) {
        return NAME.equals(uri.getLastPathSegment());
    }

    @Override
    public ParcelFileDescriptor openFile(Uri uri, String mode) throws FileNotFoundException {
        if (!"r".equals(mode)) {
            throw new FileNotFoundException("read-only provider, mode=" + mode);
        }
        if (!isServed(uri)) {
            throw new FileNotFoundException("only " + NAME + " is served: " + uri);
        }
        File zip = file();
        if (!zip.isFile()) {
            throw new FileNotFoundException(NAME + " has not been written yet");
        }
        return ParcelFileDescriptor.open(zip, ParcelFileDescriptor.MODE_READ_ONLY);
    }

    @Override
    public Cursor query(Uri uri, String[] projection, String selection, String[] selectionArgs,
                        String sortOrder) {
        if (!isServed(uri)) {
            throw new IllegalArgumentException("only " + NAME + " is served: " + uri);
        }
        String[] columns = projection != null && projection.length > 0
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
