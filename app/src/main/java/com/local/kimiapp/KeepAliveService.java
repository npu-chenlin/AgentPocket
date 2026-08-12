package com.local.kimiapp;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.content.Intent;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.IOException;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.TimeUnit;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.HttpUrl;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;

public class KeepAliveService extends Service {
    private static final String SERVICE_CHANNEL = "kimi_background";
    private static final String TASK_CHANNEL = "kimi_tasks";
    private static final String UPDATE_CHANNEL = "kimi_updates";
    private static final String STATE_PREFS = "kimi_native_listener";
    private static final String TAG = "KimiNativeSocket";
    private static final int SERVICE_ID = 1001;
    private static final long POLL_MS = 3000;
    private static final long UPDATE_CHECK_MS = 6 * 60 * 60 * 1000L;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable pollRunnable = this::pollSessions;
    private final Map<String, String> statuses = new HashMap<>();
    private final Map<String, String> sessionTitles = new HashMap<>();
    private final Set<String> subscribed = new HashSet<>();
    private final Set<String> busySessions = new HashSet<>();
    private OkHttpClient client;
    private WebSocket socket;
    private boolean stopped;
    private boolean firstPoll = true;
    private long updatedAfter;
    private int messageId;
    private long lastNotificationAt;

    @Override public void onCreate() {
        super.onCreate();
        createChannels();
        startForeground(SERVICE_ID, serviceNotification("正在连接 Kimi…"));
        client = new OkHttpClient.Builder().readTimeout(0, TimeUnit.MILLISECONDS)
                .pingInterval(20, TimeUnit.SECONDS).build();
        pollSessions();
        checkForUpdate();
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) { return START_STICKY; }
    @Override public IBinder onBind(Intent intent) { return null; }

    private void createChannels() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return;
        NotificationManager manager = getSystemService(NotificationManager.class);
        // 前台服务通道（小米/华为灵动岛会把这个常驻胶囊显示在状态栏）
        NotificationChannel service = new NotificationChannel(SERVICE_CHANNEL,
                "Kimi 后台连接", NotificationManager.IMPORTANCE_LOW);
        service.setShowBadge(false);
        manager.createNotificationChannel(service);
        // 任务通知通道 —— 改为 HIGH，让它横幅弹出（heads-up）
        NotificationChannel task = new NotificationChannel(TASK_CHANNEL,
                "Kimi 任务通知", NotificationManager.IMPORTANCE_HIGH);
        task.setDescription("任务完成、等待回答或审批时通知");
        manager.createNotificationChannel(task);
        NotificationChannel updates = new NotificationChannel(UPDATE_CHANNEL,
                "KimiWeb 应用更新", NotificationManager.IMPORTANCE_DEFAULT);
        updates.setDescription("发现新的 KimiWeb 版本时通知");
        manager.createNotificationChannel(updates);
    }

    private void checkForUpdate() {
        SharedPreferences state = getSharedPreferences(STATE_PREFS, MODE_PRIVATE);
        long now = System.currentTimeMillis();
        if (now - state.getLong("last_update_check", 0) < UPDATE_CHECK_MS) return;
        state.edit().putLong("last_update_check", now).apply();
        Request request = new Request.Builder()
                .url("https://api.github.com/repos/npu-chenlin/KimiCodeWebApp/releases/latest")
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "KimiWeb-Android/" + currentVersion()).build();
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException error) {
                Log.w(TAG, "Update check failed: " + error.getMessage());
            }
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (!response.isSuccessful()) return;
                    JSONObject release = new JSONObject(response.body().string());
                    String version = release.optString("tag_name", "").replaceFirst("^[vV]", "");
                    if (compareVersions(version, currentVersion()) <= 0) return;
                    JSONArray assets = release.optJSONArray("assets");
                    if (assets == null) return;
                    for (int i = 0; i < assets.length(); i++) {
                        JSONObject asset = assets.getJSONObject(i);
                        String name = asset.optString("name", "");
                        if (name.toLowerCase().endsWith(".apk")) {
                            postUpdateNotification(version, name,
                                    asset.getString("browser_download_url"));
                            break;
                        }
                    }
                } catch (Exception error) { Log.w(TAG, "Update response parse failed", error); }
            }
        });
    }

    private String currentVersion() {
        try { return getPackageManager().getPackageInfo(getPackageName(), 0).versionName; }
        catch (Exception ignored) { return "0.0.0"; }
    }

    private int compareVersions(String left, String right) {
        String[] a = left.split("\\."), b = right.split("\\.");
        for (int i = 0; i < Math.max(a.length, b.length); i++) {
            int av = i < a.length ? parseVersionPart(a[i]) : 0;
            int bv = i < b.length ? parseVersionPart(b[i]) : 0;
            if (av != bv) return Integer.compare(av, bv);
        }
        return 0;
    }

    private int parseVersionPart(String value) {
        try { return Integer.parseInt(value.replaceAll("[^0-9].*$", "")); }
        catch (Exception ignored) { return 0; }
    }

    private void postUpdateNotification(String version, String name, String url) {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) return;
        Intent open = new Intent(this, MainActivity.class)
                .putExtra(MainActivity.EXTRA_UPDATE_URL, url)
                .putExtra(MainActivity.EXTRA_UPDATE_NAME, name)
                .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent pending = PendingIntent.getActivity(this, 9, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, UPDATE_CHANNEL) : new Notification.Builder(this);
        getSystemService(NotificationManager.class).notify("app-update", 0,
                builder.setSmallIcon(android.R.drawable.stat_sys_download_done)
                        .setContentTitle("KimiWeb 有新版本 " + version)
                        .setContentText("点击下载并安装更新")
                        .setAutoCancel(true).setContentIntent(pending).build());
    }

    private Notification serviceNotification(String text) {
        PendingIntent pending = PendingIntent.getActivity(this, 1,
                new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent settings = PendingIntent.getActivity(this, 3,
                new Intent(this, MainActivity.class)
                        .putExtra(MainActivity.EXTRA_SHOW_CONFIG, true)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, SERVICE_CHANNEL) : new Notification.Builder(this);
        return builder.setSmallIcon(R.mipmap.ic_launcher)
                .setLargeIcon(largeIcon())
                .setContentTitle("Kimi 后台监听").setContentText(text)
                .setOngoing(true).setContentIntent(pending)
                .addAction(new Notification.Action.Builder(null, "连接设置", settings).build()).build();
    }

    private Bitmap largeIcon() {
        return BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher);
    }

    private String baseUrl() {
        SharedPreferences prefs = getSharedPreferences("kimi_connection", MODE_PRIVATE);
        return "http://" + prefs.getString("ip", "100.95.189.73") + ":" + prefs.getInt("port", 58627);
    }

    private void pollSessions() {
        if (stopped) return;
        HttpUrl.Builder url = HttpUrl.get(baseUrl() + "/api/v2/sessions").newBuilder()
                .addQueryParameter("meta.archived", "false")
                .addQueryParameter("sort", "meta.updated_at_desc")
                .addQueryParameter("page_size", "100");
        if (!firstPoll && updatedAfter > 0)
            url.addQueryParameter("meta.updated_after", String.valueOf(Math.max(0, updatedAfter - 1000)));
        client.newCall(authorizedRequest(url.build())).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException error) {
                Log.w(TAG, "V2 poll failed: " + error.getMessage()); schedulePoll();
            }

            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    JSONArray items = new JSONObject(response.body().string())
                            .getJSONObject("data").getJSONArray("items");
                    processSessions(items);
                } catch (Exception error) { Log.w(TAG, "V2 parse failed", error); }
                finally { schedulePoll(); }
            }
        });
    }

    private Request authorizedRequest(HttpUrl url) {
        Request.Builder builder = new Request.Builder().url(url);
        String token = getSharedPreferences("kimi_connection", MODE_PRIVATE).getString("token", "");
        if (token != null && !token.isEmpty()) builder.header("Authorization", "Bearer " + token);
        return builder.build();
    }

    private synchronized void processSessions(JSONArray items) throws Exception {
        JSONArray live = new JSONArray();
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.getJSONObject(i);
            String id = item.getString("id");
            JSONObject meta = item.getJSONObject("meta");
            String status = item.getJSONObject("activity").getString("status");
            sessionTitles.put(id, title(meta));
            updatedAfter = Math.max(updatedAfter, meta.optLong("updated_at", 0));
            String previous = statuses.put(id, status);

            if ("running".equals(status)) busySessions.add(id);
            if (!firstPoll && !MainActivity.isVisible && previous != null && !previous.equals(status)) {
                if ("running".equals(previous) && "idle".equals(status))
                    postNotification("complete-" + id, "Kimi Code · 回合完成", title(meta));
                else if ("approval".equals(status)) fetchPendingDetail(id, true);
                else if ("question".equals(status)) fetchPendingDetail(id, false);
                else if ("failed".equals(status))
                    postNotification("failed-" + id, "Kimi Code · 回合失败", title(meta));
            }

            boolean active = "running".equals(status) || "approval".equals(status)
                    || "question".equals(status) || "failed".equals(status);
            if (active && !subscribed.contains(id)) live.put(id);
        }
        if (socket == null) openSocket(live); else subscribeMore(live);
        firstPoll = false;
        Log.i(TAG, "V2 poll items=" + items.length() + ", new live=" + live.length());
    }

    private String title(JSONObject meta) {
        String value = meta.optString("title", "");
        return value.isEmpty() || "null".equals(value) ? "点击查看 Kimi 会话" : value;
    }

    private JSONObject cursors(JSONArray ids) {
        JSONObject result = new JSONObject();
        SharedPreferences prefs = getSharedPreferences(STATE_PREFS, MODE_PRIVATE);
        for (int i = 0; i < ids.length(); i++) {
            String id = ids.optString(i), raw = prefs.getString("cursor_" + id, null);
            if (raw != null) try { result.put(id, new JSONObject(raw)); } catch (Exception ignored) {}
        }
        return result;
    }

    private synchronized void openSocket(JSONArray ids) {
        if (socket != null || stopped) return;
        String url = baseUrl().replaceFirst("^http", "ws") + "/api/v1/ws";
        Request.Builder socketRequest = new Request.Builder().url(url);
        String token = getSharedPreferences("kimi_connection", MODE_PRIVATE).getString("token", "");
        if (token != null && !token.isEmpty()) socketRequest.header("Authorization", "Bearer " + token);
        socket = client.newWebSocket(socketRequest.build(), new WebSocketListener() {
            @Override public void onOpen(WebSocket ws, Response response) {
                try {
                    ws.send(new JSONObject().put("type", "client_hello").put("id", "android-hello")
                            .put("payload", new JSONObject()
                                    .put("client_id", "kimi-android-" + UUID.randomUUID())
                                    .put("subscriptions", ids).put("cursors", cursors(ids))).toString());
                    Log.i(TAG, "Native socket opened, requested=" + ids.length());
                } catch (Exception error) { Log.w(TAG, "hello failed", error); }
            }
            @Override public void onMessage(WebSocket ws, String text) { handleFrame(ws, text); }
            @Override public void onFailure(WebSocket ws, Throwable error, Response response) {
                if (socket == ws) socket = null; subscribed.clear();
                Log.w(TAG, "Native socket failed: " + error.getMessage());
                handler.postDelayed(KeepAliveService.this::pollSessions, 3000);
            }
            @Override public void onClosed(WebSocket ws, int code, String reason) {
                if (socket == ws) socket = null; subscribed.clear();
                handler.postDelayed(KeepAliveService.this::pollSessions, 3000);
            }
        });
    }

    private void subscribeMore(JSONArray ids) {
        if (socket == null || ids.length() == 0) return;
        try {
            socket.send(new JSONObject().put("type", "subscribe").put("id", "android-sub-" + (++messageId))
                    .put("payload", new JSONObject().put("session_ids", ids).put("cursors", cursors(ids))).toString());
            Log.i(TAG, "Dynamic subscribe=" + ids.length());
        } catch (Exception error) { Log.w(TAG, "subscribe failed", error); }
    }

    private void handleFrame(WebSocket ws, String text) {
        try {
            JSONObject frame = new JSONObject(text);
            String type = frame.optString("type");
            if ("ping".equals(type)) {
                JSONObject pong = new JSONObject().put("type", "pong");
                if (frame.has("id")) pong.put("id", frame.get("id"));
                ws.send(pong.toString()); return;
            }
            if ("ack".equals(type)) {
                JSONObject p = frame.optJSONObject("payload");
                JSONArray acceptedIds = p == null ? null : p.optJSONArray("accepted_subscriptions");
                int accepted = acceptedIds == null ? 0 : acceptedIds.length();
                if (acceptedIds != null)
                    for (int i = 0; i < acceptedIds.length(); i++) subscribed.add(acceptedIds.optString(i));
                JSONArray resync = p == null ? null : p.optJSONArray("resync_required");
                if (resync != null)
                    for (int i = 0; i < resync.length(); i++) subscribed.remove(resync.optString(i));
                Log.i(TAG, "Subscription ack=" + accepted + ", total=" + subscribed.size());
                getSystemService(NotificationManager.class).notify(SERVICE_ID,
                        serviceNotification("已连接，监听 " + subscribed.size() + " 个会话"));
                return;
            }
            saveCursor(frame);
            JSONObject payload = frame.optJSONObject("payload");
            String event = payload != null && payload.has("type") ? payload.optString("type") : type;
            String session = frame.optString("session_id", "");
            Log.i(TAG, "Event=" + event);
            if ("event.session.work_changed".equals(event) && payload != null) {
                boolean busy = payload.optBoolean("busy", false);
                if (busy) busySessions.add(session);
                else if (busySessions.remove(session) && "completed".equals(payload.optString("last_turn_reason"))
                        && !MainActivity.isVisible) postNotification("complete-" + session,
                        "Kimi Code · 回合完成", sessionTitle(session, "点击查看结果"));
            } else if (!MainActivity.isVisible && event.contains("question") && event.contains("request"))
                fetchPendingDetail(session, false);
            else if (!MainActivity.isVisible && event.contains("approval") && event.contains("request"))
                fetchPendingDetail(session, true);
        } catch (Exception error) { Log.w(TAG, "Bad frame", error); }
    }

    private String sessionTitle(String sessionId, String fallback) {
        String value = sessionTitles.get(sessionId);
        return value == null || value.isEmpty() ? fallback : value;
    }

    private void fetchPendingDetail(String sessionId, boolean approval) {
        if (sessionId == null || sessionId.isEmpty()) return;
        String kind = approval ? "approvals" : "questions";
        HttpUrl url = HttpUrl.get(baseUrl() + "/api/v1/sessions/" + sessionId + "/" + kind)
                .newBuilder().addQueryParameter("status", "pending").build();
        client.newCall(authorizedRequest(url)).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException error) { postPendingFallback(); }
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    JSONArray items = new JSONObject(response.body().string())
                            .getJSONObject("data").getJSONArray("items");
                    if (items.length() == 0) { postPendingFallback(); return; }
                    JSONObject item = items.getJSONObject(0);
                    if (approval) {
                        String id = item.optString("approval_id", sessionId);
                        String tool = item.optString("tool_name", "");
                        postNotification("approval-" + id, "Kimi Code · 等待审批",
                                tool.isEmpty() ? sessionTitle(sessionId, "有工具等待你审批") : tool);
                    } else {
                        String id = item.optString("question_id", sessionId);
                        JSONArray questions = item.optJSONArray("questions");
                        String preview = "";
                        if (questions != null && questions.length() > 0) {
                            JSONObject question = questions.getJSONObject(0);
                            preview = question.optString("question", "");
                            if (preview.isEmpty()) preview = question.optString("header", "");
                            if (preview.isEmpty()) preview = question.optString("body", "");
                        }
                        postNotification("question-" + id, "Kimi Code · 待回答",
                                preview.isEmpty() ? sessionTitle(sessionId, "有提问等待你回答") : preview);
                    }
                } catch (Exception error) { Log.w(TAG, "Pending detail parse failed", error); postPendingFallback(); }
            }
            private void postPendingFallback() {
                postNotification(kind + "-" + sessionId,
                        approval ? "Kimi Code · 等待审批" : "Kimi Code · 待回答",
                        sessionTitle(sessionId, approval ? "有工具等待你审批" : "有提问等待你回答"));
            }
        });
    }

    private void saveCursor(JSONObject frame) {
        String id = frame.optString("session_id", "");
        if (id.isEmpty() || !frame.has("seq")) return;
        try {
            JSONObject cursor = new JSONObject().put("seq", frame.getLong("seq"));
            if (frame.has("epoch")) cursor.put("epoch", frame.getString("epoch"));
            getSharedPreferences(STATE_PREFS, MODE_PRIVATE).edit()
                    .putString("cursor_" + id, cursor.toString()).apply();
        } catch (Exception ignored) {}
    }

    private void postNotification(String tag, String title, String body) {
        long now = System.currentTimeMillis();
        if (now - lastNotificationAt < 500) return;
        lastNotificationAt = now;
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                != PackageManager.PERMISSION_GRANTED) return;
        PendingIntent pending = PendingIntent.getActivity(this, 2,
                new Intent(this, MainActivity.class).setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        Notification.Builder builder = Build.VERSION.SDK_INT >= 26
                ? new Notification.Builder(this, TASK_CHANNEL) : new Notification.Builder(this);
        // 高优先级 + 大图标，让它在小米/华为上横幅弹出（heads-up）
        if (Build.VERSION.SDK_INT >= 26) builder.setPriority(Notification.PRIORITY_HIGH);
        getSystemService(NotificationManager.class).notify(tag, 0,
                builder.setSmallIcon(R.mipmap.ic_launcher)
                        .setLargeIcon(largeIcon())
                        .setContentTitle(title)
                        .setContentText(body)
                        .setStyle(new Notification.BigTextStyle().bigText(body))
                        .setAutoCancel(true)
                        .setContentIntent(pending)
                        .build());
        Log.i(TAG, "Notification posted: " + title);
    }

    private void schedulePoll() {
        handler.removeCallbacks(pollRunnable);
        if (!stopped) handler.postDelayed(pollRunnable, POLL_MS);
    }

    @Override public void onDestroy() {
        stopped = true; handler.removeCallbacksAndMessages(null);
        if (socket != null) socket.close(1000, "service stopped");
        if (client != null) client.dispatcher().executorService().shutdown();
        super.onDestroy();
    }
}
