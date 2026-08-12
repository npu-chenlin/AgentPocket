package com.local.kimiapp;

import android.app.DownloadManager;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;

import androidx.core.content.FileProvider;

import java.io.File;

public class UpdateDownloadReceiver extends BroadcastReceiver {
    static final String PREFS = "kimi_updates";

    @Override public void onReceive(Context context, Intent intent) {
        long completed = intent.getLongExtra(DownloadManager.EXTRA_DOWNLOAD_ID, -1);
        long expected = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getLong("download_id", -2);
        if (completed != expected) return;
        DownloadManager manager = context.getSystemService(DownloadManager.class);
        try (Cursor cursor = manager.query(new DownloadManager.Query().setFilterById(completed))) {
            if (!cursor.moveToFirst() || cursor.getInt(cursor.getColumnIndexOrThrow(
                    DownloadManager.COLUMN_STATUS)) != DownloadManager.STATUS_SUCCESSFUL) return;
        }
        String name = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
                .getString("download_name", "KimiWeb-update.apk");
        File apk = new File(context.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS), name);
        Uri uri = FileProvider.getUriForFile(context, context.getPackageName() + ".files", apk);
        Intent install = new Intent(Intent.ACTION_VIEW).setDataAndType(uri,
                "application/vnd.android.package-archive")
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
        context.startActivity(install);
    }
}
