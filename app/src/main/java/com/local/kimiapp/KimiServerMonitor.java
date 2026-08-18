package com.local.kimiapp;

import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.HttpUrl;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;

/**
 * Kimi Code web 服务器监听器。
 *
 * Protocol version: Kimi Code CLI 0.36.1+
 *
 * Key differences from older versions:
 * 1. Auth via Sec-WebSocket-Protocol: "kimi-code.bearer.{token}"
 * 2. client_hello must include subscriptions + cursors
 * 3. Events are pushed directly (no session_event wrapper). Current servers use
 *    event.session.work_changed (busy true/false) and prompt.completed.
 */
public class KimiServerMonitor extends ServerMonitor {
    private static final String TAG = "KimiWsMonitor";

    private final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());
    private final Map<String, String> pendingBySession = Collections.synchronizedMap(new HashMap<>());

    public KimiServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        super(host, server, client);
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

    @Override public void start() {
        if (stopped) return;
        connected = false;
        notifySummary();
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
                        }
                        activeCount = busyCount();
                    }
                    notifySummary();
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
                notifySummary();

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
                notifySummary();
                scheduleReconnect();
            }

            @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                Log.w(TAG, server.name + " WS failure: " + t.getMessage());
                connected = false;
                setHealth(false);
                notifySummary();
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
                    Log.i(TAG, server.name + " subscribed to " + titleCache.size() + " sessions");
                    break;

                case "subscribe_ack":
                    Log.i(TAG, server.name + " subscribe ack");
                    break;

                case "ack":
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
                if (!sessionId.isEmpty() && wasActive != isActive) {
                    busyBySession.put(sessionId, isActive);
                    activeCount = busyCount();
                }
                notifySummary();

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
                boolean wasBusy = Boolean.TRUE.equals(busyBySession.get(sessionId));
                busyBySession.put(sessionId, busy);
                if (busy != wasBusy) {
                    activeCount = busyCount();
                    notifySummary();
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
                        busyBySession.put(newId, true);
                        activeCount = busyCount();
                        notifySummary();
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

    private Request authorize(HttpUrl url) {
        Request.Builder req = new Request.Builder().url(url);
        if (server.token != null && !server.token.isEmpty())
            req.header("Authorization", "Bearer " + server.token);
        return req.build();
    }

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
}
