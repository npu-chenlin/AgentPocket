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
import java.time.Instant;
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
 * Protocol version: Kimi Code CLI 0.36.1+
 *
 * Key differences from older versions:
 * 1. Auth via Sec-WebSocket-Protocol: "kimi-code.bearer.{token}"
 * 2. client_hello must include subscriptions + cursors
 * 3. Events are pushed directly (no session_event wrapper). Current servers use
 *    event.session.work_changed (busy true/false) and prompt.completed.
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
                .readTimeout(0, TimeUnit.SECONDS)
                .pingInterval(30, TimeUnit.SECONDS)
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
        getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                .putInt("active_count", active).apply();
    }

    private void publishEvent(String event) {
        getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
                .putString("last_event", event)
                .putLong("last_event_ts", System.currentTimeMillis())
                .apply();
    }

    private final class ServerMonitor {
        final ServerStore.Server server;
        final Map<String, String> titleCache = Collections.synchronizedMap(new HashMap<>());
        final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());
        final Map<String, String> pendingBySession = Collections.synchronizedMap(new HashMap<>());
        WebSocket webSocket;
        volatile boolean connected;
        volatile int activeCount;
        long reconnectDelay = RECONNECT_BASE_MS;
        final Runnable reconnectRunnable = this::start;
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
            fetchSessionList(this::connectWebSocket);
        }

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
                    Log.w(TAG, server.name + " REST failed: " + e.getMessage());
                    scheduleReconnect();
                }
                @Override public void onResponse(Call call, Response response) {
                    try (Response ignored = response) {
                        if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                        JSONArray items = new JSONObject(response.body().string())
                                .getJSONObject("data").getJSONArray("items");
                        synchronized (titleCache) {
                            titleCache.clear();
                            busyBySession.clear();
                            pendingBySession.clear();
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
                                boolean busy = !"idle".equals(status);
                                busyBySession.put(id, busy);
                                pendingBySession.put(id, "none");
                                if (busy) activeCount++;
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

            // Auth: Kimi Code 0.36+ uses Sec-WebSocket-Protocol with bearer token
            if (server.token != null && !server.token.isEmpty()) {
                req.header("Sec-WebSocket-Protocol", "kimi-code.bearer." + server.token);
            }

            webSocket = client.newWebSocket(req.build(), new WebSocketListener() {
                @Override public void onOpen(WebSocket ws, Response response) {
                    Log.i(TAG, server.name + " WS open");
                    connected = true;
                    reconnectDelay = RECONNECT_BASE_MS;
                    setHealth(true);
                    updateSummary();

                    // client_hello with subscriptions + cursors (required by 0.36+)
                    List<String> ids = new ArrayList<>(titleCache.keySet());
                    try {
                        sendJson(ws, buildClientHello(ids));
                    } catch (JSONException e) {
                        Log.e(TAG, server.name + " hello failed", e);
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
                Log.d(TAG, server.name + " << " + type);

                switch (type) {
                    case "server_hello":
                        // Subscriptions were already included in client_hello. Sending a second
                        // subscribe here causes an unnecessary acknowledgement and may replay data.
                        Log.i(TAG, server.name + " subscribed to " + titleCache.size() + " sessions");
                        break;

                    case "subscribe_ack":
                        Log.i(TAG, server.name + " subscribe ack");
                        break;

                    case "ack":
                        // hello or other ack
                        break;

                    case "ping":
                        if (webSocket != null) {
                            sendJson(webSocket, new JSONObject()
                                    .put("type", "pong")
                                    .put("payload", new JSONObject().put("nonce", msg.optJSONObject("payload") != null ? msg.optJSONObject("payload").opt("nonce") : UUID.randomUUID().toString())));
                        }
                        break;

                    case "resync_required":
                        Log.i(TAG, server.name + " resync");
                        fetchSessionList(() -> {
                            if (webSocket != null && !titleCache.isEmpty()) {
                                try {
                                    sendJson(webSocket, buildSubscribe(new ArrayList<>(titleCache.keySet())));
                                } catch (JSONException e) {}
                            }
                        });
                        break;

                    case "error":
                        Log.w(TAG, server.name + " error: " + text);
                        break;

                    default:
                        // All other messages are routed based on VY() logic:
                        // event.session.status_changed → protocol → onWireEvent
                        // event.session.created → protocol
                        // prompt.submitted → agent
                        if (type.startsWith("event.session.")) {
                            handleProtocolEvent(msg);
                        } else if (type.equals("prompt.submitted") || type.equals("prompt.completed") || type.equals("prompt.aborted")) {
                            handleAgentEvent(msg);
                        }
                        break;
                }
            } catch (JSONException e) {
                Log.w(TAG, server.name + " bad JSON: " + text.substring(0, Math.min(200, text.length())));
            }
        }

        private void handleProtocolEvent(JSONObject msg) {
            String type = msg.optString("type", "");
            String sessionId = msg.optString("session_id", "");
            JSONObject payload = msg.optJSONObject("payload");
            if (payload == null) payload = new JSONObject();

            Log.d(TAG, server.name + " protocol event: " + type + " session=" + sessionId);

            switch (type) {
                case "event.session.status_changed": {
                    String status = payload.optString("status", "idle");
                    String previous = payload.optString("previous_status", "");

                    boolean wasActive = !"idle".equals(previous) && !previous.isEmpty();
                    boolean isActive = !"idle".equals(status);
                    if (wasActive && !isActive) activeCount = Math.max(0, activeCount - 1);
                    else if (!wasActive && isActive) activeCount++;
                    updateSummary();

                    // Publish face events before the visibility guard — the floating ball
                    // exists only while the app is visible, so events must always be emitted.
                    if ("running".equals(previous) && "idle".equals(status)) {
                        publishEvent("complete");
                        if (MainActivity.isVisible) return;
                        maybeNotify(sessionId, eventKey(msg, "status-complete"),
                                "Kimi Code · 回合完成", getTitle(sessionId));
                    } else if ("awaiting_approval".equals(status)) {
                        publishEvent("approval");
                        if (MainActivity.isVisible) return;
                        maybeNotify(sessionId, eventKey(msg, "status-approval"),
                                "Kimi Code · 等待审批", getTitle(sessionId));
                    } else if ("awaiting_question".equals(status)) {
                        publishEvent("question");
                        if (MainActivity.isVisible) return;
                        maybeNotify(sessionId, eventKey(msg, "status-question"),
                                "Kimi Code · 待回答", getTitle(sessionId));
                    } else if ("aborted".equals(status)) {
                        publishEvent("aborted");
                        if (MainActivity.isVisible) return;
                        maybeNotify(sessionId, eventKey(msg, "status-aborted"),
                                "Kimi Code · 回合失败", getTitle(sessionId));
                    }
                    break;
                }

                case "event.session.work_changed": {
                    boolean busy = payload.optBoolean("busy", false);
                    Boolean previousBusy = busyBySession.put(sessionId, busy);
                    if (previousBusy != null && previousBusy != busy) {
                        if (busy) activeCount++;
                        else activeCount = Math.max(0, activeCount - 1);
                        updateSummary();
                    }

                    String pending = payload.optString("pending_interaction", "none");
                    String previousPending = pendingBySession.put(sessionId, pending);
                    if (!pending.equals(previousPending)) {
                        if ("approval".equals(pending)) {
                            publishEvent("approval");
                            if (!MainActivity.isVisible && isFreshEvent(msg)) {
                                maybeNotify(sessionId, eventKey(msg, "approval"),
                                        "Kimi Code · 等待审批", getTitle(sessionId));
                            }
                        } else if ("question".equals(pending)) {
                            publishEvent("question");
                            if (!MainActivity.isVisible && isFreshEvent(msg)) {
                                maybeNotify(sessionId, eventKey(msg, "question"),
                                        "Kimi Code · 待回答", getTitle(sessionId));
                            }
                        }
                    }
                    break;
                }

                case "event.session.created": {
                    JSONObject sessionObj = payload.optJSONObject("session");
                    if (sessionObj != null) {
                        String newId = sessionObj.optString("id", "");
                        if (!newId.isEmpty()) {
                            JSONObject meta = sessionObj.optJSONObject("meta");
                            String title = meta != null ? meta.optString("title", "") : "";
                            if (title.isEmpty() || "null".equals(title)) title = "点击查看 Kimi 会话";
                            synchronized (titleCache) { titleCache.put(newId, title); }
                            activeCount++;
                            updateSummary();
                            if (webSocket != null) {
                                try {
                                    sendJson(webSocket, buildSubscribe(Collections.singletonList(newId)));
                                } catch (JSONException e) {}
                            }
                        }
                    }
                    break;
                }

                case "event.session.updated":
                case "event.session.deleted":
                    // Refresh title cache
                    break;
            }
        }

        private void handleAgentEvent(JSONObject msg) {
            String type = msg.optString("type", "");
            String sessionId = msg.optString("session_id", "");
            JSONObject payload = msg.optJSONObject("payload");
            if (payload == null) payload = new JSONObject();
            Log.d(TAG, server.name + " agent event: " + type + " session=" + sessionId);

            if ("prompt.submitted".equals(type)) {
                refreshTitleFor(sessionId);
            } else if ("prompt.completed".equals(type)) {
                publishEvent("complete");
                if (!MainActivity.isVisible && isFreshEvent(msg)) {
                    String promptId = payload.optString("promptId", eventKey(msg, "prompt-complete"));
                    notifyTurnFinished(sessionId, payload.optString("reason", "completed"),
                            "prompt-complete:" + promptId);
                }
            } else if ("prompt.aborted".equals(type)) {
                publishEvent("aborted");
                if (!MainActivity.isVisible && isFreshEvent(msg)) {
                    String promptId = payload.optString("promptId", eventKey(msg, "prompt-aborted"));
                    maybeNotify(sessionId, "prompt-aborted:" + promptId,
                            "Kimi Code · 回合失败", getTitle(sessionId));
                }
            }
        }

        private void notifyTurnFinished(String sessionId, String reason, String key) {
            boolean failed = "aborted".equals(reason) || "failed".equals(reason)
                    || "error".equals(reason) || "cancelled".equals(reason);
            maybeNotify(sessionId, key,
                    failed ? "Kimi Code · 回合失败" : "Kimi Code · 回合完成",
                    getTitle(sessionId));
        }

        private boolean isFreshEvent(JSONObject msg) {
            String timestamp = msg.optString("timestamp", "");
            if (timestamp.isEmpty()) return true;
            try {
                long age = System.currentTimeMillis() - Instant.parse(timestamp).toEpochMilli();
                return age >= -30000 && age <= 2 * 60 * 1000L;
            } catch (Exception ignored) {
                return true;
            }
        }

        private String eventKey(JSONObject msg, String prefix) {
            String epoch = msg.optString("epoch", "");
            long seq = msg.optLong("seq", -1);
            if (seq >= 0) return prefix + ":" + epoch + ":" + seq;
            return prefix + ":" + msg.optString("id", UUID.randomUUID().toString());
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

        private void maybeNotify(String sessionId, String status, String title, String body) {
            String key = server.id + ":" + sessionId + ":" + status;
            if (!notifiedKeys.add(key)) {
                Log.d(TAG, server.name + " dedup: " + key);
                return;
            }
            Log.i(TAG, server.name + " notify: " + key);
            KeepAliveService.this.postTask(server, sessionId, key, title, body);
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
            if (webSocket != null) { webSocket.cancel(); webSocket = null; }
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

    /**
     * Build client_hello per Kimi Code 0.36+ protocol.
     * Empty cursors mean "start from now" and avoid replaying the server's event buffer.
     */
    private static JSONObject buildClientHello(List<String> sessionIds) throws JSONException {
        JSONObject payload = new JSONObject().put("client_id", "kimiapp-android");

        if (!sessionIds.isEmpty()) {
            JSONArray subs = new JSONArray();
            for (String id : sessionIds) {
                subs.put(id);
            }
            payload.put("subscriptions", subs);
            payload.put("cursors", new JSONObject());
        } else {
            payload.put("subscriptions", new JSONArray());
            payload.put("cursors", new JSONObject());
        }

        return new JSONObject()
                .put("type", "client_hello")
                .put("id", UUID.randomUUID().toString())
                .put("payload", payload);
    }

    /**
     * Build subscribe message per Kimi Code 0.36+ protocol.
     */
    private static JSONObject buildSubscribe(List<String> sessionIds) throws JSONException {
        JSONArray arr = new JSONArray();
        for (String id : sessionIds) {
            arr.put(id);
        }
        return new JSONObject()
                .put("type", "subscribe")
                .put("id", UUID.randomUUID().toString())
                .put("payload", new JSONObject()
                        .put("session_ids", arr)
                        .put("cursors", new JSONObject()));
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
