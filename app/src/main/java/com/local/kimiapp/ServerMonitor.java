package com.local.kimiapp;

import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import org.json.JSONObject;

import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

import okhttp3.OkHttpClient;
import okhttp3.WebSocket;

/**
 * 服务器监听器的公共基类：重连调度、在线健康、通知与悬浮球表情的发布。
 * 具体协议（Kimi Code / DeepSeek Harness）由子类实现 {@link #start()} 与消息解析。
 */
public abstract class ServerMonitor {
    private static final String TAG = "ServerMonitor";
    private static final String HEALTH_PREFS = "kimi_server_health";

    // Reconnect backoff: 2s, 4s, 8s, 16s, 30s, 30s...
    protected static final long RECONNECT_BASE_MS = 2000;
    protected static final long RECONNECT_MAX_MS = 30000;

    /** 监听器与 Service 之间的回调边界，由 KeepAliveService 实现。 */
    public interface MonitorHost {
        void onSummaryChanged();
        void onFaceEvent(String event);
        void postTask(String serverId, String sessionId, String tag, String title, String body);
        void setHealth(String serverId, boolean online);
    }

    protected final MonitorHost host;
    protected final ServerStore.Server server;
    protected final OkHttpClient client;
    private final Handler handler = new Handler(Looper.getMainLooper());

    protected final Map<String, String> titleCache = Collections.synchronizedMap(new HashMap<>());
    protected WebSocket webSocket;
    protected volatile boolean connected;
    protected volatile int activeCount;
    protected volatile boolean stopped;
    protected long reconnectDelay = RECONNECT_BASE_MS;

    private final Set<String> notifiedKeys = Collections.newSetFromMap(
            new LinkedHashMap<String, Boolean>() {
                @Override protected boolean removeEldestEntry(Map.Entry<String, Boolean> eldest) {
                    return size() > 300;
                }
            });

    protected ServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        this.host = host;
        this.server = server;
        this.client = client;
    }

    public boolean isConnected() { return connected; }
    public int getActiveCount() { return activeCount; }
    public String serverId() { return server.id; }

    /** 启动连接流程：由子类实现（拉列表 → 连 WebSocket → 解析事件）。 */
    public abstract void start();

    /** 终止监听（Service 销毁时调用）。 */
    public void shutdown() {
        stopped = true;
        disconnect();
    }

    protected void scheduleReconnect() {
        if (stopped) return;
        handler.removeCallbacks(this::runReconnect);
        handler.postDelayed(this::runReconnect, reconnectDelay);
        reconnectDelay = Math.min(reconnectDelay * 2, RECONNECT_MAX_MS);
        Log.i(TAG, server.name + " reconnect in " + reconnectDelay + "ms");
    }

    private void runReconnect() { start(); }

    protected void disconnect() {
        handler.removeCallbacks(this::runReconnect);
        if (webSocket != null) { webSocket.cancel(); webSocket = null; }
        connected = false;
    }

    protected void setHealth(boolean online) {
        host.setHealth(server.id, online);
    }

    protected void notifySummary() {
        host.onSummaryChanged();
    }

    protected void publishEvent(String event) {
        host.onFaceEvent(event);
    }

    protected void maybeNotify(String sessionId, String status, String title, String body) {
        String key = server.id + ":" + sessionId + ":" + status;
        if (!notifiedKeys.add(key)) {
            Log.d(TAG, server.name + " dedup: " + key);
            return;
        }
        Log.i(TAG, server.name + " notify: " + key);
        host.postTask(server.id, sessionId, key, title, body);
    }

    protected void notifyTurnFinished(String sessionId, String reason, String key) {
        boolean failed = "aborted".equals(reason) || "failed".equals(reason)
                || "error".equals(reason) || "cancelled".equals(reason);
        maybeNotify(sessionId, key,
                failed ? "任务回合失败" : "任务回合完成",
                getTitle(sessionId));
    }

    protected String getTitle(String sessionId) {
        synchronized (titleCache) {
            String t = titleCache.get(sessionId);
            return t != null ? t : "点击查看会话";
        }
    }

    protected boolean isFreshEvent(JSONObject msg) {
        String timestamp = msg.optString("timestamp", "");
        if (timestamp.isEmpty()) return true;
        try {
            long age = System.currentTimeMillis() - java.time.Instant.parse(timestamp).toEpochMilli();
            return age >= -30000 && age <= 2 * 60 * 1000L;
        } catch (Exception ignored) {
            return true;
        }
    }

    protected String eventKey(JSONObject msg, String prefix) {
        String epoch = msg.optString("epoch", "");
        long seq = msg.optLong("seq", -1);
        if (seq >= 0) return prefix + ":" + epoch + ":" + seq;
        return prefix + ":" + msg.optString("id", UUID.randomUUID().toString());
    }
}
