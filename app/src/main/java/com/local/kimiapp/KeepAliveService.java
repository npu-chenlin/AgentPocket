package com.local.kimiapp;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.BitmapFactory;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.IOException;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.TimeUnit;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.HttpUrl;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;

public class KeepAliveService extends Service {
    private static final String SERVICE_CHANNEL = "kimi_background";
    private static final String TASK_CHANNEL = "kimi_tasks";
    private static final String UPDATE_CHANNEL = "kimi_updates";
    private static final String STATE_PREFS = "kimi_native_listener";
    public static final String HEALTH_PREFS = "kimi_server_health";
    private static final String TAG = "KimiMultiMonitor";
    private static final int SERVICE_ID = 1001;
    private static final long POLL_MS = 3000;
    private static final long UPDATE_CHECK_MS = 6 * 60 * 60 * 1000L;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final List<ServerMonitor> monitors = new ArrayList<>();
    private OkHttpClient client;
    private boolean stopped;
    private long lastNotificationAt;

    @Override public void onCreate() {
        super.onCreate();
        createChannels();
        startForeground(SERVICE_ID, serviceNotification("正在连接服务器…"));
        client = new OkHttpClient.Builder().connectTimeout(8, TimeUnit.SECONDS)
                .readTimeout(12, TimeUnit.SECONDS).build();
        for (ServerStore.Server server : ServerStore.load(this)) {
            ServerMonitor monitor = new ServerMonitor(server);
            monitors.add(monitor);
            monitor.poll();
        }
        updateSummary();
        checkForUpdate();
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) { return START_STICKY; }
    @Override public IBinder onBind(Intent intent) { return null; }

    private void createChannels() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationManager manager = getSystemService(NotificationManager.class);
        NotificationChannel service = new NotificationChannel(SERVICE_CHANNEL, "Kimi 后台连接",
                NotificationManager.IMPORTANCE_LOW);
        service.setShowBadge(false);
        manager.createNotificationChannel(service);
        NotificationChannel tasks = new NotificationChannel(TASK_CHANNEL, "Kimi 任务通知",
                NotificationManager.IMPORTANCE_HIGH);
        tasks.setDescription("任务完成、等待回答或审批时通知");
        manager.createNotificationChannel(tasks);
        manager.createNotificationChannel(new NotificationChannel(UPDATE_CHANNEL,
                "KimiWeb 应用更新", NotificationManager.IMPORTANCE_DEFAULT));
    }

    private Notification serviceNotification(String text) {
        PendingIntent open = PendingIntent.getActivity(this, 1, new Intent(this, MainActivity.class),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent settings = PendingIntent.getActivity(this, 3,
                new Intent(this, MainActivity.class).putExtra(MainActivity.EXTRA_SHOW_CONFIG, true)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, SERVICE_CHANNEL) : new Notification.Builder(this);
        return builder.setSmallIcon(R.mipmap.ic_launcher)
                .setLargeIcon(BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher))
                .setContentTitle("Kimi 后台监听").setContentText(text).setOngoing(true)
                .setContentIntent(open).addAction(new Notification.Action.Builder(null, "服务器", settings).build())
                .build();
    }

    private synchronized void updateSummary() {
        int connected = 0, active = 0;
        for (ServerMonitor monitor : monitors) {
            if (monitor.connected) connected++;
            active += monitor.activeCount;
        }
        getSystemService(NotificationManager.class).notify(SERVICE_ID,
                serviceNotification("已连接 " + connected + "/" + monitors.size() + " 台，监听 " + active + " 个会话"));
    }

    private final class ServerMonitor {
        final ServerStore.Server server;
        final Map<String, String> statuses = new HashMap<>();
        final Map<String, String> titles = new HashMap<>();
        boolean firstPoll = true, connected;
        int activeCount;
        long updatedAfter;

        ServerMonitor(ServerStore.Server server) { this.server = server; }

        void poll() {
            if (stopped) return;
            HttpUrl.Builder url;
            try {
                url = HttpUrl.get(server.baseUrl() + "/api/v2/sessions").newBuilder()
                        .addQueryParameter("meta.archived", "false")
                        .addQueryParameter("sort", "meta.updated_at_desc")
                        .addQueryParameter("page_size", "100");
            } catch (Exception error) { schedule(); return; }
            if (!firstPoll && updatedAfter > 0)
                url.addQueryParameter("meta.updated_after", String.valueOf(Math.max(0, updatedAfter - 1000)));
            client.newCall(authorize(url.build())).enqueue(new Callback() {
                @Override public void onFailure(Call call, IOException error) {
                    setConnected(false); updateSummary(); schedule();
                    Log.w(TAG, server.name + " poll failed: " + error.getMessage());
                }
                @Override public void onResponse(Call call, Response response) {
                    try (Response ignored = response) {
                        if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                        JSONArray items = new JSONObject(response.body().string())
                                .getJSONObject("data").getJSONArray("items");
                        process(items); setConnected(true);
                    } catch (Exception error) {
                        setConnected(false); Log.w(TAG, server.name + " parse failed", error);
                    } finally { firstPoll = false; updateSummary(); schedule(); }
                }
            });
        }

        void setConnected(boolean value) {
            connected = value;
            getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                    .putBoolean("online_" + server.id, value)
                    .putLong("checked_" + server.id, System.currentTimeMillis()).apply();
        }

        Request authorize(HttpUrl url) {
            Request.Builder request = new Request.Builder().url(url);
            if (server.token != null && !server.token.isEmpty())
                request.header("Authorization", "Bearer " + server.token);
            return request.build();
        }

        synchronized void process(JSONArray items) throws Exception {
            Set<String> active = new HashSet<>();
            for (int i = 0; i < items.length(); i++) {
                JSONObject item = items.getJSONObject(i);
                String id = item.getString("id");
                JSONObject meta = item.getJSONObject("meta");
                String status = item.getJSONObject("activity").getString("status");
                String title = meta.optString("title", "");
                if (title.isEmpty() || "null".equals(title)) title = "点击查看 Kimi 会话";
                titles.put(id, title);
                updatedAfter = Math.max(updatedAfter, meta.optLong("updated_at", 0));
                String previous = statuses.put(id, status);
                if (!"idle".equals(status)) active.add(id);
                if (!firstPoll && !MainActivity.isVisible && previous != null && !previous.equals(status)) {
                    if ("running".equals(previous) && "idle".equals(status))
                        postTask(server, "complete-" + server.id + "-" + id, "Kimi Code · 回合完成", title);
                    else if ("approval".equals(status)) fetchDetail(id, true, title);
                    else if ("question".equals(status)) fetchDetail(id, false, title);
                    else if ("failed".equals(status))
                        postTask(server, "failed-" + server.id + "-" + id, "Kimi Code · 回合失败", title);
                }
            }
            activeCount = active.size();
        }

        void fetchDetail(String sessionId, boolean approval, String fallback) {
            String kind = approval ? "approvals" : "questions";
            HttpUrl url = HttpUrl.get(server.baseUrl() + "/api/v1/sessions/" + sessionId + "/" + kind)
                    .newBuilder().addQueryParameter("status", "pending").build();
            client.newCall(authorize(url)).enqueue(new Callback() {
                @Override public void onFailure(Call call, IOException error) { postFallback(); }
                @Override public void onResponse(Call call, Response response) {
                    try (Response ignored = response) {
                        JSONArray items = new JSONObject(response.body().string()).getJSONObject("data").getJSONArray("items");
                        if (items.length() == 0) { postFallback(); return; }
                        JSONObject item = items.getJSONObject(0);
                        if (approval) {
                            String tool = item.optString("tool_name", "");
                            postTask(server, "approval-" + server.id + "-" + item.optString("approval_id", sessionId),
                                    "Kimi Code · 等待审批", tool.isEmpty() ? fallback : tool);
                        } else {
                            JSONArray questions = item.optJSONArray("questions");
                            String preview = questions != null && questions.length() > 0
                                    ? questions.getJSONObject(0).optString("question", "") : "";
                            postTask(server, "question-" + server.id + "-" + item.optString("question_id", sessionId),
                                    "Kimi Code · 待回答", preview.isEmpty() ? fallback : preview);
                        }
                    } catch (Exception error) { postFallback(); }
                }
                void postFallback() {
                    postTask(server, kind + "-" + server.id + "-" + sessionId,
                            approval ? "Kimi Code · 等待审批" : "Kimi Code · 待回答", fallback);
                }
            });
        }

        void schedule() { if (!stopped) handler.postDelayed(this::poll, POLL_MS); }
    }

    private void postTask(ServerStore.Server server, String tag, String title, String body) {
        long now = System.currentTimeMillis();
        if (now - lastNotificationAt < 500) return;
        lastNotificationAt = now;
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) return;
        String serverName = server.name;
        PendingIntent pending = PendingIntent.getActivity(this, server.id.hashCode(),
                new Intent(this, MainActivity.class).putExtra(MainActivity.EXTRA_SERVER_ID, server.id)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, TASK_CHANNEL) : new Notification.Builder(this);
        getSystemService(NotificationManager.class).notify(tag, 0,
                builder.setSmallIcon(R.mipmap.ic_launcher)
                        .setLargeIcon(BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher))
                        .setContentTitle(title).setContentText("[" + serverName + "] " + body)
                        .setStyle(new Notification.BigTextStyle().bigText("[" + serverName + "] " + body))
                        .setAutoCancel(true).setContentIntent(pending).build());
    }

    private void checkForUpdate() {
        SharedPreferences state = getSharedPreferences(STATE_PREFS, MODE_PRIVATE);
        long now = System.currentTimeMillis();
        if (now - state.getLong("last_update_check", 0) < UPDATE_CHECK_MS) return;
        Request request = new Request.Builder()
                .url("https://api.github.com/repos/npu-chenlin/KimiCodeWebApp/releases/latest")
                .header("Accept", "application/vnd.github+json").header("User-Agent", "KimiWeb-Android").build();
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException error) {}
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (!response.isSuccessful()) return;
                    state.edit().putLong("last_update_check", System.currentTimeMillis()).apply();
                    JSONObject release = new JSONObject(response.body().string());
                    String version = release.optString("tag_name", "").replaceFirst("^[vV]", "");
                    if (compareVersions(version, currentVersion()) <= 0) return;
                    JSONArray assets = release.optJSONArray("assets");
                    for (int i = 0; assets != null && i < assets.length(); i++) {
                        JSONObject asset = assets.getJSONObject(i);
                        String name = asset.optString("name", "");
                        if (name.toLowerCase().endsWith(".apk")) {
                            postUpdate(version, name, asset.getString("browser_download_url")); break;
                        }
                    }
                } catch (Exception ignored) {}
            }
        });
    }

    private String currentVersion() {
        try { return getPackageManager().getPackageInfo(getPackageName(), 0).versionName; }
        catch (Exception ignored) { return "0.0.0"; }
    }
    private int compareVersions(String a, String b) {
        String[] aa = a.split("\\."), bb = b.split("\\.");
        for (int i = 0; i < Math.max(aa.length, bb.length); i++) {
            int av = i < aa.length ? number(aa[i]) : 0, bv = i < bb.length ? number(bb[i]) : 0;
            if (av != bv) return Integer.compare(av, bv);
        }
        return 0;
    }
    private int number(String value) { try { return Integer.parseInt(value.replaceAll("[^0-9].*$", "")); } catch (Exception e) { return 0; } }

    private void postUpdate(String version, String name, String url) {
        Intent open = new Intent(this, MainActivity.class).putExtra(MainActivity.EXTRA_UPDATE_URL, url)
                .putExtra(MainActivity.EXTRA_UPDATE_NAME, name).setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent pending = PendingIntent.getActivity(this, 9, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, UPDATE_CHANNEL) : new Notification.Builder(this);
        getSystemService(NotificationManager.class).notify("app-update", 0,
                builder.setSmallIcon(android.R.drawable.stat_sys_download_done)
                        .setContentTitle("KimiWeb 有新版本 " + version).setContentText("点击下载并安装更新")
                        .setAutoCancel(true).setContentIntent(pending).build());
    }

    @Override public void onDestroy() {
        stopped = true;
        handler.removeCallbacksAndMessages(null);
        if (client != null) client.dispatcher().executorService().shutdown();
        super.onDestroy();
    }
}
