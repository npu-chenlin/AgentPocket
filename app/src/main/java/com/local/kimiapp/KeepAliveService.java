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
import android.graphics.drawable.Icon;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
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

/**
 * WebSocket-based real-time monitor for Kimi Code servers.
 *
 * Replaces HTTP polling with a single persistent WebSocket per server.
 * Traffic: ~10-50 KB/hour idle vs ~1.5-3 MB/hour for 3s polling.
 * Latency: real-time push vs 0-3s poll window.
 *
 * Protocol: ws://host:port/api/v1/ws
 * 1. Send client_hello (with client_id + optional subscriptions)
 * 2. Receive server_hello
 * 3. Send subscribe with known session_ids
 * 4. Receive session_event payloads:
 *    - event.session.status_changed  → idle/running/awaiting_approval/awaiting_question/aborted
 *    - event.session.work_changed    → busy, pending_interaction
 */
public class KeepAliveService extends Service {
    private static final String SERVICE_CHANNEL = "kimi_background";
    private static final String TASK_CHANNEL = "kimi_tasks";
    private static final String UPDATE_CHANNEL = "kimi_updates";
    private static final String STATE_PREFS = "kimi_native_listener";
    public static final String HEALTH_PREFS = "kimi_server_health";
    private static final String TAG = "KimiWsMonitor";
    private static final int SERVICE_ID = 1001;
    private static final long UPDATE_CHECK_MS = 6 * 60 * 60 * 1000L;

    // Reconnect backoff: 2s, 4s, 8s, 16s, 30s, 30s...
    private static final long RECONNECT_BASE_MS = 2000;
    private static final long RECONNECT_MAX_MS = 30000;

    private final Handler handler = new Handler(Looper.getMainLooper());
    private final List<ServerMonitor> monitors = new ArrayList<>();
    private OkHttpClient client;
    private boolean stopped;
    private PowerManager.WakeLock wakeLock;

    @Override public void onCreate() {
        super.onCreate();
        createChannels();
        startForeground(SERVICE_ID, serviceNotification("正在连接服务器…"));
        acquireWakeLock();

        client = new OkHttpClient.Builder()
                .connectTimeout(10, TimeUnit.SECONDS)
                .readTimeout(0, TimeUnit.SECONDS)    // WebSocket needs infinite read
                .pingInterval(30, TimeUnit.SECONDS)  // RFC 6455 keepalive
                .build();

        for (ServerStore.Server server : ServerStore.load(this)) {
            ServerMonitor monitor = new ServerMonitor(server);
            monitors.add(monitor);
            monitor.start();
        }
        updateSummary();
        checkForUpdate();
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
        PendingIntent open = PendingIntent.getActivity(this, 1,
                new Intent(this, MainActivity.class),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        PendingIntent settings = PendingIntent.getActivity(this, 3,
                new Intent(this, MainActivity.class)
                        .putExtra(MainActivity.EXTRA_SHOW_CONFIG, true)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Builder(this, SERVICE_CHANNEL)
                .setSmallIcon(R.mipmap.ic_launcher)
                .setLargeIcon(BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher))
                .setContentTitle("Kimi 后台监听").setContentText(text).setOngoing(true)
                .setContentIntent(open)
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(this, R.mipmap.ic_launcher),
                        "服务器", settings).build())
                .build();
    }

    private synchronized void updateSummary() {
        int connected = 0, active = 0;
        for (ServerMonitor m : monitors) {
            if (m.isConnected()) connected++;
            active += m.getActiveCount();
        }
        String text = "已连接 " + connected + "/" + monitors.size()
                + " 台，监听 " + active + " 个会话";
        getSystemService(NotificationManager.class).notify(SERVICE_ID,
                serviceNotification(text));
    }

    /**
     * Per-server WebSocket monitor.
     *
     * Lifecycle:
     *   IDLE → (start) → SYNC_REST → (got list) → WS_CONNECTING → (onOpen) → WS_HANDSHAKE →
     *   (server_hello) → WS_SUBSCRIBED → session_event flow
     *   Any failure → BACKOFF → WS_CONNECTING
     */
    private final class ServerMonitor {
        final ServerStore.Server server;

        // Cached metadata from REST
        final Map<String, String> titleCache = Collections.synchronizedMap(new HashMap<>());

        // State tracking
        WebSocket webSocket;
        volatile boolean connected;
        volatile int activeCount;

        // Reconnect
        long reconnectDelay = RECONNECT_BASE_MS;
        final Runnable reconnectRunnable = this::start;

        // Notification dedup: "serverId:sessionId:status"
        private final Set<String> notifiedKeys = Collections.newSetFromMap(
                new LinkedHashMap<String, Boolean>() {
                    @Override protected boolean removeEldestEntry(Map.Entry<String, Boolean> eldest) {
                        return size() > 300;
                    }
                });

        ServerMonitor(ServerStore.Server server) { this.server = server; }

        boolean isConnected() { return connected; }
        int getActiveCount() { return activeCount; }

        void start() {
            if (stopped) return;
            connected = false;
            updateSummary();
            // Step 1: fetch session list over REST to populate title cache
            fetchSessionList(() -> {
                if (stopped) return;
                // Step 2: open WebSocket
                connectWebSocket();
            });
        }

        /**
         * Pull /api/v2/sessions once to build the title cache and get current session IDs.
         */
        private void fetchSessionList(Runnable then) {
            HttpUrl url;
            try {
                url = HttpUrl.get(server.baseUrl() + "/api/v2/sessions").newBuilder()
                        .addQueryParameter("meta.archived", "false")
                        .addQueryParameter("page_size", "100")
                        .build();
            } catch (Exception e) { scheduleReconnect(); return; }

            client.newCall(authorize(url)).enqueue(new Callback() {
                @Override public void onFailure(Call call, IOException e) {
                    Log.w(TAG, server.name + " REST fetch failed: " + e.getMessage());
                    scheduleReconnect();
                }
                @Override public void onResponse(Call call, Response response) {
                    try (Response ignored = response) {
                        if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                        JSONArray items = new JSONObject(response.body().string())
                                .getJSONObject("data").getJSONArray("items");
                        synchronized (titleCache) {
                            titleCache.clear();
                            activeCount = 0;
                            for (int i = 0; i < items.length(); i++) {
                                JSONObject item = items.getJSONObject(i);
                                String id = item.getString("id");
                                JSONObject meta = item.optJSONObject("meta");
                                String title = meta != null ? meta.optString("title", "") : "";
                                if (title.isEmpty() || "null".equals(title)) title = "点击查看 Kimi 会话";
                                titleCache.put(id, title);
                                JSONObject activity = item.optJSONObject("activity");
                                String status = activity != null ? activity.optString("status", "idle") : "idle";
                                if (!"idle".equals(status)) activeCount++;
                            }
                        }
                        updateSummary();
                        then.run();
                    } catch (Exception e) {
                        Log.w(TAG, server.name + " REST parse failed", e);
                        scheduleReconnect();
                    }
                }
            });
        }

        private void connectWebSocket() {
            if (stopped) return;
            String wsUrl = server.baseUrl().replace("http://", "ws://").replace("https://", "wss://")
                    + "/api/v1/ws";
            Request.Builder req = new Request.Builder().url(wsUrl);
            if (server.token != null && !server.token.isEmpty())
                req.header("Authorization", "Bearer " + server.token);

            webSocket = client.newWebSocket(req.build(), new WebSocketListener() {
                @Override public void onOpen(WebSocket ws, Response response) {
                    Log.i(TAG, server.name + " WS open");
                    connected = true;
                    reconnectDelay = RECONNECT_BASE_MS;
                    setHealth(true);
                    updateSummary();

                    // Send client_hello with all known session IDs as subscriptions
                    List<String> ids = new ArrayList<>(titleCache.keySet());
                    try {
                        sendJson(ws, buildClientHello(ids));
                    } catch (JSONException e) {
                        Log.e(TAG, server.name + " build hello failed", e);
                    }
                }

                @Override public void onMessage(WebSocket ws, String text) {
                    handleMessage(text);
                }

                @Override public void onClosing(WebSocket ws, int code, String reason) {
                    Log.i(TAG, server.name + " WS closing: " + code + " " + reason);
                    ws.close(code, reason);
                }

                @Override public void onClosed(WebSocket ws, int code, String reason) {
                    connected = false;
                    setHealth(false);
                    updateSummary();
                    scheduleReconnect();
                }

                @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                    Log.w(TAG, server.name + " WS failure: " + t.getMessage());
                    connected = false;
                    setHealth(false);
                    updateSummary();
                    scheduleReconnect();
                }
            });
        }

        private void handleMessage(String text) {
            try {
                JSONObject msg = new JSONObject(text);
                String type = msg.optString("type", "");

                switch (type) {
                    case "server_hello":
                        // Now officially subscribed; some servers accept subscriptions in client_hello,
                        // but we send an explicit subscribe for safety.
                        if (webSocket != null) {
                            List<String> ids = new ArrayList<>(titleCache.keySet());
                            if (!ids.isEmpty()) {
                                sendJson(webSocket, buildSubscribe(ids));
                            }
                        }
                        break;

                    case "subscribe_ack":
                        Log.i(TAG, server.name + " subscribed to " +
                                msg.optJSONObject("payload").optJSONArray("accepted").length() + " sessions");
                        break;

                    case "session_event":
                        handleSessionEvent(msg);
                        break;

                    case "ping":
                        // OkHttp handles RFC ping/pong automatically, but protocol-level ping
                        // messages from the server should be acknowledged with pong if required.
                        // The asyncapi says "pong" is a client->server message type.
                        if (webSocket != null) {
                            sendJson(webSocket, new JSONObject()
                                    .put("type", "pong")
                                    .put("id", UUID.randomUUID().toString()));
                        }
                        break;

                    case "resync_required":
                        Log.i(TAG, server.name + " resync required");
                        fetchSessionList(() -> {
                            if (webSocket != null && !titleCache.isEmpty()) {
                                try {
                                    sendJson(webSocket, buildSubscribe(new ArrayList<>(titleCache.keySet())));
                                } catch (JSONException e) {
                                    Log.e(TAG, server.name + " build subscribe failed", e);
                                }
                            }
                        });
                        break;

                    case "error":
                        Log.w(TAG, server.name + " WS error: " + text);
                        break;

                    default:
                        // acks and others ignored
                        break;
                }
            } catch (JSONException e) {
                Log.w(TAG, server.name + " malformed JSON: " + text.substring(0, Math.min(200, text.length())));
            }
        }

        private void handleSessionEvent(JSONObject msg) throws JSONException {
            JSONObject payload = msg.getJSONObject("payload");
            String eventType = payload.getString("type");
            String sessionId = msg.optString("sessionId", "");

            switch (eventType) {
                case "event.session.status_changed": {
                    String status = payload.getString("status");
                    String previous = payload.optString("previous_status", "");

                    // Update active count
                    boolean wasActive = !"idle".equals(previous) && !previous.isEmpty();
                    boolean isActive = !"idle".equals(status);
                    if (wasActive && !isActive) activeCount = Math.max(0, activeCount - 1);
                    else if (!wasActive && isActive) activeCount++;
                    updateSummary();

                    // Notifications (only when app not visible)
                    if (MainActivity.isVisible) return;

                    if ("running".equals(previous) && "idle".equals(status)) {
                        maybeNotify(sessionId, status, "Kimi Code · 回合完成", getTitle(sessionId));
                    } else if ("awaiting_approval".equals(status)) {
                        maybeNotify(sessionId, status, "Kimi Code · 等待审批", getTitle(sessionId));
                    } else if ("awaiting_question".equals(status)) {
                        maybeNotify(sessionId, status, "Kimi Code · 待回答", getTitle(sessionId));
                    } else if ("aborted".equals(status)) {
                        maybeNotify(sessionId, status, "Kimi Code · 回合失败", getTitle(sessionId));
                    }
                    break;
                }

                case "event.session.work_changed": {
                    // Fallback / supplementary signal
                    String pending = payload.optString("pending_interaction", "none");
                    if (!"none".equals(pending) && !MainActivity.isVisible) {
                        String title = getTitle(sessionId);
                        if ("approval".equals(pending)) {
                            maybeNotify(sessionId, "approval", "Kimi Code · 等待审批", title);
                        } else if ("question".equals(pending)) {
                            maybeNotify(sessionId, "question", "Kimi Code · 待回答", title);
                        }
                    }
                    break;
                }

                case "event.session.created":
                case "prompt.submitted": {
                    // New session or prompt may mean title changed / new session
                    // Refresh title cache lazily
                    refreshTitleFor(sessionId);
                    break;
                }

                default:
                    // Other events (turn.started, turn.ended, etc.) ignored for notifications
                    break;
            }
        }

        private String getTitle(String sessionId) {
            synchronized (titleCache) {
                String t = titleCache.get(sessionId);
                return t != null ? t : "点击查看 Kimi 会话";
            }
        }

        private void refreshTitleFor(String sessionId) {
            HttpUrl url;
            try {
                url = HttpUrl.get(server.baseUrl() + "/api/v1/sessions/" + sessionId).newBuilder().build();
            } catch (Exception e) { return; }
            client.newCall(authorize(url)).enqueue(new Callback() {
                @Override public void onFailure(Call call, IOException e) {}
                @Override public void onResponse(Call call, Response response) {
                    try (Response ignored = response) {
                        if (!response.isSuccessful()) return;
                        JSONObject data = new JSONObject(response.body().string()).optJSONObject("data");
                        if (data == null) return;
                        JSONObject meta = data.optJSONObject("meta");
                        String title = meta != null ? meta.optString("title", "") : "";
                        if (!title.isEmpty() && !"null".equals(title)) {
                            synchronized (titleCache) { titleCache.put(sessionId, title); }
                        }
                    } catch (Exception ignored) {}
                }
            });
        }

        /**
         * Deduplicate by (serverId + sessionId + status). Prevents duplicate notifications
         * when server retransmits or status flaps.
         */
        private void maybeNotify(String sessionId, String status, String title, String body) {
            String key = server.id + ":" + sessionId + ":" + status;
            if (!notifiedKeys.add(key)) return; // already sent this exact state
            KeepAliveService.this.postTask(server, key, title, body);
        }

        void scheduleReconnect() {
            if (stopped) return;
            handler.removeCallbacks(reconnectRunnable);
            handler.postDelayed(reconnectRunnable, reconnectDelay);
            reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
            Log.i(TAG, server.name + " reconnect in " + reconnectDelay + "ms");
        }

        void disconnect() {
            handler.removeCallbacks(reconnectRunnable);
            if (webSocket != null) {
                webSocket.cancel();
                webSocket = null;
            }
            connected = false;
        }

        private Request authorize(HttpUrl url) {
            return KeepAliveService.authorize(url, server.token);
        }

        private void setHealth(boolean online) {
            getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                    .putBoolean("online_" + server.id, online)
                    .putLong("checked_" + server.id, System.currentTimeMillis())
                    .apply();
        }
    }

    // ------------------------------------------------------------------
    // Protocol helpers
    // ------------------------------------------------------------------

    private static JSONObject buildClientHello(List<String> sessionIds) throws JSONException {
        JSONObject payload = new JSONObject()
                .put("client_id", "kimiapp-android");
        if (!sessionIds.isEmpty()) {
            JSONArray arr = new JSONArray();
            for (String id : sessionIds) arr.put(id);
            payload.put("subscriptions", arr);
        }
        return new JSONObject()
                .put("type", "client_hello")
                .put("id", UUID.randomUUID().toString())
                .put("payload", payload);
    }

    private static JSONObject buildSubscribe(List<String> sessionIds) throws JSONException {
        JSONArray arr = new JSONArray();
        for (String id : sessionIds) arr.put(id);
        return new JSONObject()
                .put("type", "subscribe")
                .put("id", UUID.randomUUID().toString())
                .put("payload", new JSONObject().put("session_ids", arr));
    }

    private static void sendJson(WebSocket ws, JSONObject json) {
        ws.send(json.toString());
    }

    private static Request authorize(HttpUrl url, String token) {
        Request.Builder req = new Request.Builder().url(url);
        if (token != null && !token.isEmpty())
            req.header("Authorization", "Bearer " + token);
        return req.build();
    }

    // ------------------------------------------------------------------
    // Notifications & updates
    // ------------------------------------------------------------------

    private void postTask(ServerStore.Server server, String tag, String title, String body) {
        if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)
                        != PackageManager.PERMISSION_GRANTED) {
            return;
        }
        String serverName = server.name;
        PendingIntent pending = PendingIntent.getActivity(this, server.id.hashCode(),
                new Intent(this, MainActivity.class)
                        .putExtra(MainActivity.EXTRA_SERVER_ID, server.id)
                        .setFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP | Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
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

    private void checkForUpdate() {
        SharedPreferences state = getSharedPreferences(STATE_PREFS, MODE_PRIVATE);
        long now = System.currentTimeMillis();
        if (now - state.getLong("last_update_check", 0) < UPDATE_CHECK_MS) return;
        Request request = new Request.Builder()
                .url("https://api.github.com/repos/npu-chenlin/KimiCodeWebApp/releases/latest")
                .header("Accept", "application/vnd.github+json")
                .header("User-Agent", "KimiWeb-Android")
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
                        .setContentTitle("KimiWeb 有新版本 " + version)
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
        for (ServerMonitor m : monitors) m.disconnect();
        if (client != null) client.dispatcher().executorService().shutdown();
        if (wakeLock != null && wakeLock.isHeld()) wakeLock.release();
        super.onDestroy();
    }
}
