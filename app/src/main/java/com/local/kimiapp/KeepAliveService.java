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
import android.os.PowerManager;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.TimeUnit;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;

/**
 * 后台监听服务：为每台已添加的服务器按协议启动一个 {@link ServerMonitor}，
 * 汇总在线状态与忙碌会话数，并把监听器产生的任务事件转成通知与悬浮球表情。
 * 协议差异（Kimi Code / DeepSeek Harness）由各监听器实现，本类只做编排。
 */
public class KeepAliveService extends Service implements ServerMonitor.MonitorHost {
    private static final String SERVICE_CHANNEL = "kimi_background";
    private static final String TASK_CHANNEL = "kimi_tasks";
    private static final String UPDATE_CHANNEL = "kimi_updates";
    private static final String STATE_PREFS = "kimi_native_listener";
    public static final String HEALTH_PREFS = "kimi_server_health";
    private static final String TAG = "KeepAliveService";
    private static final int SERVICE_ID = 1001;
    private static final long UPDATE_CHECK_MS = 6 * 60 * 60 * 1000L;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final List<ServerMonitor> monitors = new ArrayList<>();
    private OkHttpClient client;
    private boolean stopped;
    private PowerManager.WakeLock wakeLock;

    @Override public void onCreate() {
        super.onCreate();
        createChannels();
        startForeground(SERVICE_ID, serviceNotification("正在连接服务器…", null));
        acquireWakeLock();

        client = new OkHttpClient.Builder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.SECONDS)
                .pingInterval(30, TimeUnit.SECONDS)
                .build();

        for (ServerStore.Server server : ServerStore.load(this)) {
            ServerMonitor monitor = createMonitor(server);
            monitors.add(monitor);
            monitor.start();
        }
        updateSummary();
        checkForUpdate();
    }

    private ServerMonitor createMonitor(ServerStore.Server server) {
        if (ServerStore.Server.BACKEND_DSH.equals(server.backend)) {
            return new DshServerMonitor(this, server, client);
        }
        return new KimiServerMonitor(this, server, client);
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        return START_STICKY;
    }

    @Override public IBinder onBind(Intent intent) { return null; }

    private void acquireWakeLock() {
        try {
            PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
            wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "KimiApp:WsKeepAlive");
            wakeLock.acquire();
        } catch (Exception e) {
            Log.w(TAG, "wake lock failed", e);
        }
    }

    private void createChannels() {
        if (Build.VERSION.SDK_INT < 26) return;
        NotificationManager manager = getSystemService(NotificationManager.class);
        NotificationChannel service = new NotificationChannel(SERVICE_CHANNEL, "AgentPocket 后台连接",
                NotificationManager.IMPORTANCE_LOW);
        service.setShowBadge(false);
        manager.createNotificationChannel(service);
        NotificationChannel tasks = new NotificationChannel(TASK_CHANNEL, "AgentPocket 任务通知",
                NotificationManager.IMPORTANCE_HIGH);
        tasks.setDescription("任务完成、等待回答或审批时通知");
        manager.createNotificationChannel(tasks);
        manager.createNotificationChannel(new NotificationChannel(UPDATE_CHANNEL,
                "AgentPocket 应用更新", NotificationManager.IMPORTANCE_DEFAULT));
    }

    private Notification serviceNotification(String text, List<String> sessions) {
        PendingIntent open = PendingIntent.getActivity(this, 1,
                new Intent(this, MainActivity.class)
                        .putExtra(MainActivity.EXTRA_SHOW_SESSIONS, true)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = new Notification.Builder(this, SERVICE_CHANNEL)
                .setSmallIcon(R.mipmap.ic_launcher)
                .setLargeIcon(BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher))
                .setContentTitle("AgentPocket 后台监听").setContentText(text).setOngoing(true)
                .setContentIntent(open);
        if (sessions != null && !sessions.isEmpty()) {
            Notification.InboxStyle style = new Notification.InboxStyle()
                    .setBigContentTitle(text);
            int shown = Math.min(sessions.size(), 6);
            for (int i = 0; i < shown; i++) {
                style.addLine(sessions.get(i));
            }
            if (sessions.size() > shown) {
                style.addLine("… 其余 " + (sessions.size() - shown) + " 个会话");
            }
            builder.setStyle(style);
        }
        return builder.build();
    }

    // ------------------------------------------------------------------
    // ServerMonitor.MonitorHost
    // ------------------------------------------------------------------

    @Override public void onSummaryChanged() { updateSummary(); }

    @Override public void onFaceEvent(String event) { publishEvent(event); }

    @Override public void postTask(String serverId, String sessionId, String tag, String title, String body) {
        for (ServerStore.Server s : ServerStore.load(this)) {
            if (s.id.equals(serverId)) {
                postTask(s, sessionId, tag, title, body);
                return;
            }
        }
    }

    @Override public void setHealth(String serverId, boolean online) {
        getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                .putBoolean("online_" + serverId, online)
                .putLong("checked_" + serverId, System.currentTimeMillis())
                .apply();
    }

    private synchronized void updateSummary() {
        int connected = 0, active = 0;
        List<String> busyTitles = new ArrayList<>();
        JSONArray busySessions = new JSONArray();
        boolean multi = monitors.size() > 1;
        for (ServerMonitor m : monitors) {
            if (m.isConnected()) connected++;
            active += m.getActiveCount();
            String prefix = multi ? "[" + m.serverName() + "] " : "";
            for (String title : m.busySessionTitles()) {
                busyTitles.add(prefix + title);
            }
            for (String[] s : m.busySessions()) {
                try {
                    busySessions.put(new JSONObject()
                            .put("serverId", s[0])
                            .put("serverName", m.serverName())
                            .put("sessionId", s[1])
                            .put("title", s[2])
                            .put("activity", s.length > 3 ? s[3] : ""));
                } catch (Exception ignored) {}
            }
        }
        String text = "已连接 " + connected + "/" + monitors.size()
                + " 台，监听 " + active + " 个会话";
        getSystemService(NotificationManager.class).notify(SERVICE_ID,
                serviceNotification(text, busyTitles));
        getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                .putInt("active_count", active)
                .putString("busy_sessions", busySessions.toString()).apply();
    }

    private void publishEvent(String event) {
        getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                .putString("last_event", event)
                .putLong("last_event_ts", System.currentTimeMillis())
                .apply();
    }

    // ------------------------------------------------------------------
    // 通知
    // ------------------------------------------------------------------

    private void postTask(ServerStore.Server server, String sessionId, String tag, String title, String body) {
        if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            return;
        }
        String serverName = server.name;
        Intent intent = new Intent(this, MainActivity.class)
                .putExtra(MainActivity.EXTRA_SERVER_ID, server.id)
                .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        if (sessionId != null && !sessionId.isEmpty()) {
            intent.putExtra(MainActivity.EXTRA_SESSION_ID, sessionId);
        }
        PendingIntent pending = PendingIntent.getActivity(this, tag.hashCode(),
                intent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        getSystemService(NotificationManager.class).notify(tag, 0,
                new Notification.Builder(this, TASK_CHANNEL)
                        .setSmallIcon(R.mipmap.ic_launcher)
                        .setLargeIcon(BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher))
                        .setContentTitle(title)
                        .setContentText("[" + serverName + "] " + body)
                        .setStyle(new Notification.BigTextStyle().bigText("[" + serverName + "] " + body))
                        .setAutoCancel(true)
                        .setContentIntent(pending)
                        .build());
    }

    // ------------------------------------------------------------------
    // 应用更新检查
    // ------------------------------------------------------------------

    private void checkForUpdate() {
        SharedPreferences state = getSharedPreferences(STATE_PREFS, MODE_PRIVATE);
        long now = System.currentTimeMillis();
        if (now - state.getLong("last_update_check", 0) < UPDATE_CHECK_MS) return;
        Request request = new Request.Builder()
                .url("https://api.github.com/repos/npu-chenlin/AgentPocket/releases/latest")
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "AgentPocket-Android")
                .build();
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
                            postUpdate(version, name, asset.getString("browser_download_url"));
                            break;
                        }
                    }
                } catch (Exception ignored) {}
            }
        });
    }

    private void postUpdate(String version, String name, String url) {
        Intent open = new Intent(this, MainActivity.class)
                .putExtra(MainActivity.EXTRA_UPDATE_URL, url)
                .putExtra(MainActivity.EXTRA_UPDATE_NAME, name)
                .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent pending = PendingIntent.getActivity(this, 9, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        getSystemService(NotificationManager.class).notify("app-update", 0,
                new Notification.Builder(this, UPDATE_CHANNEL)
                        .setSmallIcon(R.mipmap.ic_launcher)
                        .setContentTitle("AgentPocket 有新版本 " + version)
                        .setContentText("点击下载并安装更新")
                        .setAutoCancel(true)
                        .setContentIntent(pending)
                        .build());
    }

    private String currentVersion() {
        try { return getPackageManager().getPackageInfo(getPackageName(), 0).versionName; }
        catch (Exception ignored) { return "0.0.0"; }
    }

    private int compareVersions(String a, String b) {
        String[] aa = a.split("\\."), bb = b.split("\\.");
        for (int i = 0; i < Math.max(aa.length, bb.length); i++) {
            int av = i < aa.length ? number(aa[i]) : 0;
            int bv = i < bb.length ? number(bb[i]) : 0;
            if (av != bv) return Integer.compare(av, bv);
        }
        return 0;
    }

    private int number(String value) {
        try { return Integer.parseInt(value.replaceAll("[^0-9].*$", "")); }
        catch (Exception e) { return 0; }
    }

    @Override public void onDestroy() {
        stopped = true;
        handler.removeCallbacksAndMessages(null);
        for (ServerMonitor m : monitors) m.shutdown();
        if (client != null) client.dispatcher().executorService().shutdown();
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        super.onDestroy();
    }
}
