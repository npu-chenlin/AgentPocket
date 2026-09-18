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
 * OpenCode web 服务器监听器。
 *
 * 协议要点（实测）：
 * 1. 会话列表：GET /session → JSON 数组（id / title / directory）。
 * 2. 忙碌状态：GET /session/status?directory=&lt;dir&gt; → {sessionId: {type: busy|idle|retry}}，
 *    status 必须带与前端一致的 directory 参数才会返回结果。
 * 3. 事件流：GET /global/event（SSE，text/event-stream），帧为
 *    {"directory","project","payload":{"id","type","properties"}}；
 *    v1 /event 则是 {"id","type","properties"}。会话事件类型
 *    session.created/updated/deleted/status/idle 与翻转（带 `.N` 尾缀）。
 * 4. 无内置鉴权：随 `opencode serve --port N` 部署时的 Bearer token（开 --dangerous-bypass-auth 则留空）。
 */
public class OpencodeServerMonitor extends ServerMonitor {
    private static final String TAG = "OpencodeMonitor";
    private static final long STATUS_POLL_MS = 10_000;

    private final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());
    private final AtomicBoolean connecting = new AtomicBoolean();
    private final AtomicBoolean polling = new AtomicBoolean();
    private String eventUrl;

    public OpencodeServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        super(host, server, client);
    }

    @Override public void start() {
        if (stopped) return;
        connected = false;
        eventUrl = server.baseUrl() + "/global/event";
        fetchSessionBaseline(() -> {
            readEventStream();
            // 轮询只随首个成功基线启动一次；后续重连复用现有轮询，避免线程累积。
            if (polling.compareAndSet(false, true)) scheduleStatusPoll();
        });
    }

    /** 轮询兜底：/session/status 不保证经 SSE 推送，（尤其断线重连后）用它校准状态。 */
    private void scheduleStatusPoll() {
        if (stopped) return;
        new Thread(() -> {
            try {
                Thread.sleep(STATUS_POLL_MS);
            } catch (InterruptedException ignored) {
                return;
            }
            if (stopped) return;
            pollStatus();
            scheduleStatusPoll();
        }, "opencode-poll-" + server.id).start();
    }

    private void pollStatus() {
        if (stopped) return;
        Response response = null;
        try {
            HttpUrl url = HttpUrl.get(server.baseUrl() + "/session/status").newBuilder()
                    .addQueryParameter("directory", directoryParam())
                    .build();
            Request request = authorize(new Request.Builder().url(url)
                    .header("Accept", "application/json"));
            response = client.newCall(request).execute();
            if (!response.isSuccessful()) return;
            JSONObject status = new JSONObject(response.body().string());
            // 用 status map 全量校准：map 中按其值更新，不在 map 中（已完成/不在监控目录）设为 idle。
            // 统一走 setBusy，让「busy -> idle」跃迁检测与 SSE 路径共用一套去重通知。
            List<String> ids;
            synchronized (titleCache) {
                ids = new ArrayList<>(titleCache.keySet());
            }
            for (String id : ids) {
                JSONObject s = status.optJSONObject(id);
                setBusy(id, s != null && !"idle".equals(s.optString("type", "")));
            }
        } catch (Exception ignored) {
        } finally {
            if (response != null) response.close();
        }
    }

    private String directoryParam() {
        String token = server.token == null ? "" : server.token.trim();
        if (token.startsWith("dir=")) return token.substring(4);
        if (token.startsWith("/")) return token;
        return "/";
    }

    private Request authorize(Request.Builder builder) {
        String token = server.token == null ? "" : server.token.trim();
        if (!token.isEmpty() && !token.startsWith("dir=") && !token.startsWith("/")) {
            builder.header("Authorization", "Bearer " + token);
        }
        return builder.build();
    }

    /** 基线：拉一次会话列表填充标题缓存；列表没有忙碌信息，状态靠轮询 + SSE。 */
    private void fetchSessionBaseline(Runnable then) {
        Request request;
        try {
            request = authorize(new Request.Builder().url(server.baseUrl() + "/session")
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
                    if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                    JSONArray items = new JSONArray(response.body().string());
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
        // v2 /global/event：type 在 payload 内层；v1 /event type 在顶层。
        JSONObject inner = json.optJSONObject("payload");
        if (inner == null) inner = json;
        String type = inner.optString("type", "");
        // durable 事件带 .N 尾缀（session.created.5），归一化。
        int dot = type.lastIndexOf('.');
        if (dot > 0) {
            String suffix = type.substring(dot + 1);
            if (suffix.matches("\\d+")) type = type.substring(0, dot);
        }
        JSONObject props = inner.optJSONObject("properties");
        if (props == null) props = new JSONObject();
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