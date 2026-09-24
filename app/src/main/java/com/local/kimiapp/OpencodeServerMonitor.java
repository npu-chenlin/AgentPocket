package com.local.kimiapp;

import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicBoolean;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.HttpUrl;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;

/**
 * OpenCode web 服务器监听器（适配 opencode v2，实测协议）。
 *
 * 1. 鉴权：HTTP Basic。用户名为 opencode（服务端固定，忽略 OPENCODE_SERVER_USERNAME），
 *    密码来自 OPENCODE_SERVER_PASSWORD。token 字段承载 user:pass，只写密码时补默认用户。
 * 2. 会话列表：GET /api/session → {"data":[{id,title,...}],"cursor":{...}}。
 * 3. 忙闲：v2 没有可轮询的状态端点，只在事件流里体现。
 * 4. 事件流：GET /api/event（SSE），帧为
 *    {"id","created","type","data":{...},"location":{"directory":...}}。
 *    一次回合对应 session.execution.started → session.execution.succeeded/failed；
 *    durable 事件 type 可能带 `.N` 尾缀。
 * 5. 全部 REST 接口在 /api/ 前缀下。
 */
public class OpencodeServerMonitor extends ServerMonitor {
    private static final String TAG = "OpencodeMonitor";

    private final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());
    private final AtomicBoolean connecting = new AtomicBoolean();
    private String eventUrl;

    public OpencodeServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        super(host, server, client);
    }

    @Override public void start() {
        if (stopped) return;
        connected = false;
        eventUrl = server.baseUrl() + "/api/event";
        // v2 的忙闲只来自 SSE 事件，不再有状态轮询。
        fetchSessionBaseline(this::readEventStream);
    }

    /**
     * 挂上 HTTP Basic 认证头。opencode v2 要求 Basic；token 字段承载 user:pass，
     * 只写密码时补默认用户名 opencode。
     */
    private Request authorize(Request.Builder builder) {
        String raw = server.token == null ? "" : server.token.trim();
        if (!raw.isEmpty()) {
            int colon = raw.indexOf(':');
            String user = colon < 0 ? "opencode" : raw.substring(0, colon).trim();
            String pass = colon < 0 ? raw : raw.substring(colon + 1);
            if (user.isEmpty()) user = "opencode";
            builder.header("Authorization", MainActivity.basicHeader(user + ":" + pass));
        }
        return builder.build();
    }

    /** 基线：拉一次会话列表填充标题缓存；忙闲只靠 SSE 事件。 */
    private void fetchSessionBaseline(Runnable then) {
        Request request;
        try {
            request = authorize(new Request.Builder().url(server.baseUrl() + "/api/session?limit=200")
                    .header("Accept", "application/json"));
        } catch (Exception e) {
            scheduleReconnect();
            return;
        }
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {
                Log.w(TAG, server.name + " session list failed: " + e.getMessage());
                scheduleReconnect();
            }
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (response.code() == 401 || response.code() == 403) {
                        throw new IOException("认证失败（HTTP " + response.code()
                                + "）：token 应填 opencode 登录密码或 用户名:密码");
                    }
                    if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                    // v2 信封：{"data":[...],"cursor":{...}}；v1 是裸数组。
                    String body = response.body().string();
                    JSONArray items;
                    try {
                        items = new JSONObject(body).optJSONArray("data");
                    } catch (Exception notEnvelope) {
                        items = new JSONArray(body);
                    }
                    if (items == null) throw new IOException("session list missing data[]");
                    synchronized (titleCache) {
                        titleCache.clear();
                        for (int i = 0; i < items.length(); i++) {
                            JSONObject item = items.optJSONObject(i);
                            if (item == null) continue;
                            String id = item.optString("id", "");
                            if (id.isEmpty()) continue;
                            String title = item.optString("title", "");
                            if (title.isEmpty() || "null".equals(title)) title = "OpenCode 会话";
                            titleCache.put(id, title);
                        }
                    }
                    notifySummary();
                    then.run();
                } catch (Exception e) {
                    Log.w(TAG, server.name + " session list parse failed", e);
                    scheduleReconnect();
                }
            }
        });
    }

    /** 连接 SSE 事件流：流式读取直到 EOF/失败，随后走基类重连。 */
    private void readEventStream() {
        if (stopped) return;
        if (!connecting.compareAndSet(false, true)) return;
        Request request = authorize(new Request.Builder().url(eventUrl)
                .header("Accept", "text/event-stream"));
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {
                connecting.set(false);
                Log.w(TAG, server.name + " SSE failed: " + e.getMessage());
                if (call.isCanceled()) return;
                connected = false;
                setHealth(false);
                notifySummary();
                scheduleReconnect();
            }
            @Override public void onResponse(Call call, Response response) {
                if (stopped) { response.close(); connecting.set(false); return; }
                if (!response.isSuccessful()) {
                    response.close();
                    connecting.set(false);
                    Log.w(TAG, server.name + " SSE HTTP " + response.code());
                    connected = false;
                    setHealth(false);
                    notifySummary();
                    scheduleReconnect();
                    return;
                }
                Log.i(TAG, server.name + " SSE open");
                connected = true;
                reconnectDelay = RECONNECT_BASE_MS;
                setHealth(true);
                notifySummary();
                // OkHttp enqueue 回调返回后 body stream 仍可读；在新线程逐行消费，
                // EOF（服务端关闭/断网）会退出并触发重连。
                new Thread(() -> {
                    try (BufferedReader reader = new BufferedReader(new InputStreamReader(
                            response.body().byteStream(), StandardCharsets.UTF_8))) {
                        String line;
                        while (!stopped && (line = reader.readLine()) != null) {
                            handleLine(line);
                        }
                    } catch (IOException e) {
                        Log.w(TAG, server.name + " SSE stream ended", e);
                    } finally {
                        connecting.set(false);
                        if (!stopped) {
                            connected = false;
                            setHealth(false);
                            notifySummary();
                            scheduleReconnect();
                        }
                    }
                }, "opencode-sse-" + server.id).start();
            }
        });
    }

    private void handleLine(String line) {
        String trimmed = line.trim();
        if (!trimmed.startsWith("data:")) return;
        String payload = trimmed.substring(5).trim();
        JSONObject json;
        try {
            json = new JSONObject(payload);
        } catch (Exception e) {
            return;
        }
        // v2 /api/event：负载在 data 内层；更早的包装用 payload；v1 直接平铺。
        JSONObject inner = json.optJSONObject("data");
        if (inner == null) inner = json.optJSONObject("payload");
        if (inner == null) inner = json;
        String type = inner.optString("type", json.optString("type", ""));
        // durable 事件带 .N 尾缀（session.created.5），归一化。
        int dot = type.lastIndexOf('.');
        if (dot > 0) {
            String suffix = type.substring(dot + 1);
            if (suffix.matches("\\d+")) type = type.substring(0, dot);
        }
        // v2 的会话字段直接放在 data 内；v1 走 properties。
        JSONObject props = inner.optJSONObject("properties");
        if (props == null) props = inner;
        String sessionId = props.optString("sessionID", props.optString("session_id", ""));
        Log.d(TAG, server.name + " << " + type + " session=" + sessionId);
        switch (type) {
            case "session.created":
            case "session.updated": {
                JSONObject info = props.optJSONObject("info");
                if (info != null) {
                    String id = info.optString("id", "");
                    String title = info.optString("title", "");
                    if (!id.isEmpty()) {
                        if (title.isEmpty() || "null".equals(title)) title = "OpenCode 会话";
                        synchronized (titleCache) { titleCache.put(id, title); }
                    }
                } else if (!sessionId.isEmpty()) {
                    String title = inner.optString("title", "OpenCode 会话");
                    if (title.isEmpty() || "null".equals(title)) title = "OpenCode 会话";
                    synchronized (titleCache) { titleCache.put(sessionId, title); }
                }
                break;
            }
            case "session.deleted": {
                if (!sessionId.isEmpty()) {
                    synchronized (titleCache) { titleCache.remove(sessionId); }
                    setBusy(sessionId, false);
                }
                break;
            }
            // v2：一次执行回合的开始与结束决定忙闲。
            case "session.execution.started": {
                if (!sessionId.isEmpty()) setBusy(sessionId, true);
                break;
            }
            case "session.execution.succeeded":
            case "session.execution.failed": {
                if (!sessionId.isEmpty()) setBusy(sessionId, false);
                break;
            }
            // v1 兼容。
            case "session.status": {
                JSONObject status = props.optJSONObject("status");
                boolean busy = status != null && !"idle".equals(status.optString("type", ""));
                if (!sessionId.isEmpty()) setBusy(sessionId, busy);
                break;
            }
            case "session.idle": {
                if (!sessionId.isEmpty()) setBusy(sessionId, false);
                break;
            }
            case "session.error": {
                if (!sessionId.isEmpty()) {
                    setBusy(sessionId, false);
                    publishEvent("aborted");
                    if (!MainActivity.isVisible) {
                        maybeNotify(sessionId, "opencode-session-error:" + sessionId,
                                "OpenCode · 会话出错", getTitle(sessionId));
                    }
                }
                break;
            }
            default:
                break;
        }
    }

    /** 按会话维护忙碌集合：busy->idle 跃迁经基类去重后由 host 发布完成通知。 */
    private void setBusy(String sessionId, boolean busy) {
        Boolean prev = busyBySession.put(sessionId, busy);
        if (prev != null && prev == busy) return;
        activeCount = busyCount();
        notifySummary();
        if (!busy && Boolean.TRUE.equals(prev)) {
            publishEvent("complete");
            if (!MainActivity.isVisible) {
                notifyTurnFinished(sessionId, "completed", "sse-idle:" + sessionId);
            }
        }
    }

    @Override public List<String> busySessionTitles() {
        List<String> titles = new ArrayList<>();
        synchronized (busyBySession) {
            for (Map.Entry<String, Boolean> entry : busyBySession.entrySet()) {
                if (Boolean.TRUE.equals(entry.getValue())) titles.add(getTitle(entry.getKey()));
            }
        }
        return titles;
    }

    @Override public List<String[]> busySessions() {
        List<String[]> result = new ArrayList<>();
        synchronized (busyBySession) {
            for (Map.Entry<String, Boolean> entry : busyBySession.entrySet()) {
                if (Boolean.TRUE.equals(entry.getValue())) {
                    result.add(new String[]{ server.id, entry.getKey(), getTitle(entry.getKey()), "", "working" });
                }
            }
        }
        return result;
    }

    /** 忙碌会话数：与桌面端一致，按忙碌集合大小去重统计。 */
    private int busyCount() {
        int count = 0;
        synchronized (busyBySession) {
            for (Boolean busy : busyBySession.values()) {
                if (Boolean.TRUE.equals(busy)) count++;
            }
        }
        return count;
    }
}