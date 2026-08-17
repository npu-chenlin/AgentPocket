package com.local.kimiapp;

import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.IOException;
import java.util.Collections;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;
import okhttp3.WebSocket;
import okhttp3.WebSocketListener;

/**
 * DeepSeek Harness (dsh) web 服务器监听器。
 *
 * 协议要点（实测）：
 * 1. HTTP RPC：POST /api/&lt;method&gt;，信封
 *    {"type":"client-request","rpcId":"...","method":"...","payload":{...}}
 *    响应 {"type":"server-response","rpcId":"...","result":{"ok":true,"value":{...}}}
 * 2. 会话列表：session.list → value.items[]（sessionId / running / projections.values.title）
 * 3. WebSocket：/api/events.mux 为纯下行流，客户端不发送任何消息（上行会被 1008 关闭）。
 *    帧为 server-request，method 有 session/event、session/subscribed、
 *    session/projection、approval/requested、question/requested 等。
 * 4. 无 token 鉴权：信任围栏按 Host 头（本机 IP 字面量）判定。
 */
public class DshServerMonitor extends ServerMonitor {
    private static final String TAG = "DshMonitor";
    private static final MediaType JSON = MediaType.parse("application/json; charset=utf-8");

    private final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());

    public DshServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        super(host, server, client);
    }

    @Override public void start() {
        if (stopped) return;
        connected = false;
        notifySummary();
        fetchSessionList(this::connectWebSocket);
    }

    // ------------------------------------------------------------------
    // 会话列表（HTTP RPC）
    // ------------------------------------------------------------------

    private void fetchSessionList(Runnable then) {
        Request request = new Request.Builder()
                .url(server.baseUrl() + "/api/session.list")
                .post(RequestBody.create(rpc("session.list", "{}"), JSON))
                .build();
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {
                Log.w(TAG, server.name + " RPC failed: " + e.getMessage());
                scheduleReconnect();
            }
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
                    JSONObject result = new JSONObject(response.body().string()).optJSONObject("result");
                    if (result == null || !result.optBoolean("ok", false)) throw new IOException("RPC not ok");
                    JSONArray items = result.optJSONObject("value").optJSONArray("items");
                    synchronized (titleCache) {
                        titleCache.clear();
                        busyBySession.clear();
                        activeCount = 0;
                        for (int i = 0; i < items.length(); i++) {
                            JSONObject item = items.getJSONObject(i);
                            String id = item.getString("sessionId");
                            boolean running = item.optBoolean("running", false);
                            busyBySession.put(id, running);
                            if (running) activeCount++;
                            JSONObject projections = item.optJSONObject("projections");
                            JSONObject values = projections != null ? projections.optJSONObject("values") : null;
                            String title = values != null ? values.optString("title", "") : "";
                            if (title.isEmpty() || "null".equals(title)) title = "";
                            if (!title.isEmpty()) titleCache.put(id, title);
                        }
                    }
                    notifySummary();
                    then.run();
                } catch (Exception e) {
                    Log.w(TAG, server.name + " RPC parse failed", e);
                    scheduleReconnect();
                }
            }
        });
    }

    // ------------------------------------------------------------------
    // WebSocket（/api/events.mux，纯下行）
    // ------------------------------------------------------------------

    private void connectWebSocket() {
        if (stopped) return;
        String wsUrl = server.baseUrl().replace("http://", "ws://").replace("https://", "wss://")
                + "/api/events.mux";

        webSocket = client.newWebSocket(new Request.Builder().url(wsUrl).build(), new WebSocketListener() {
            @Override public void onOpen(WebSocket ws, Response response) {
                Log.i(TAG, server.name + " mux open");
                connected = true;
                reconnectDelay = RECONNECT_BASE_MS;
                setHealth(true);
                notifySummary();
            }

            @Override public void onMessage(WebSocket ws, String text) {
                handleFrame(text);
            }

            @Override public void onClosed(WebSocket ws, int code, String reason) {
                Log.i(TAG, server.name + " mux closed: " + code + " " + reason);
                connected = false;
                setHealth(false);
                notifySummary();
                scheduleReconnect();
            }

            @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                Log.w(TAG, server.name + " mux failure: " + t.getMessage());
                connected = false;
                setHealth(false);
                notifySummary();
                scheduleReconnect();
            }
        });
    }

    private void handleFrame(String text) {
        try {
            JSONObject msg = new JSONObject(text);
            // 只处理 server-request 下行帧
            if (!"server-request".equals(msg.optString("type", ""))) return;
            String method = msg.optString("method", "");
            JSONObject payload = msg.optJSONObject("payload");
            if (payload == null) return;
            String sessionId = payload.optString("sessionId", "");
            Log.d(TAG, server.name + " << " + method + " session=" + sessionId);

            switch (method) {
                case "session/subscribed":
                    // baseline，无业务含义
                    break;

                case "session/event":
                    handleSessionEvent(sessionId, payload.optJSONObject("event"));
                    break;

                case "session/projection":
                    if ("title".equals(payload.optString("key", ""))) {
                        String title = payload.opt("value") instanceof String
                                ? payload.optString("value", "") : "";
                        if (!title.isEmpty() && !"null".equals(title)) {
                            synchronized (titleCache) { titleCache.put(sessionId, title); }
                        }
                    }
                    break;

                case "approval/requested": {
                    String toolName = payload.optString("toolName", "");
                    publishEvent("approval");
                    if (!MainActivity.isVisible && isFreshEvent(msg)) {
                        String body = toolName.isEmpty() ? getTitle(sessionId)
                                : "请求调用 " + toolName + " · " + getTitle(sessionId);
                        maybeNotify(sessionId, "approval:" + payload.optString("approvalId", UUID.randomUUID().toString()),
                                "DeepSeek Harness · 等待审批", body);
                    }
                    break;
                }

                case "question/requested": {
                    publishEvent("question");
                    if (!MainActivity.isVisible && isFreshEvent(msg)) {
                        String body = getTitle(sessionId);
                        JSONArray questions = payload.optJSONArray("questions");
                        if (questions != null && questions.length() > 0) {
                            String q = questions.optJSONObject(0).optString("question", "");
                            if (!q.isEmpty()) body = q;
                        }
                        maybeNotify(sessionId, "question:" + msg.optString("rpcId", UUID.randomUUID().toString()),
                                "DeepSeek Harness · 待回答", body);
                    }
                    break;
                }

                case "session/queue":
                case "approval/resolved":
                case "question/resolved":
                    // 队列快照与结果帧：通知已由 requested 帧发出
                    break;

                default:
                    break;
            }
        } catch (JSONException e) {
            Log.w(TAG, server.name + " bad frame: " + text.substring(0, Math.min(200, text.length())));
        }
    }

    /** 解析 SessionEvent（session/event 帧的 event 字段），只关心回合开始/结束与标题。 */
    private void handleSessionEvent(String sessionId, JSONObject event) {
        if (event == null) return;
        String type = event.optString("type", "");
        JSONObject data = event.optJSONObject("data");
        switch (type) {
            case "turn/start":
                setBusy(sessionId, true);
                break;

            case "turn/end": {
                setBusy(sessionId, false);
                String kind = data != null && data.optJSONObject("reason") != null
                        ? data.optJSONObject("reason").optString("kind", "") : "";
                long seq = event.optLong("seq", -1);
                String key = "turn-end:" + seq;
                if ("completed".equals(kind)) {
                    publishEvent("complete");
                    if (!MainActivity.isVisible) {
                        notifyTurnFinished(sessionId, "completed", key);
                    }
                } else if ("error".equals(kind) || "aborted".equals(kind)
                        || "blocked".equals(kind) || "max-tokens".equals(kind)
                        || "interrupted".equals(kind)) {
                    publishEvent("aborted");
                    if (!MainActivity.isVisible) {
                        maybeNotify(sessionId, key, "DeepSeek Harness · 回合失败", getTitle(sessionId));
                    }
                }
                break;
            }

            case "session/title": {
                String title = data != null ? data.optString("title", "") : "";
                if (!title.isEmpty() && !"null".equals(title)) {
                    synchronized (titleCache) { titleCache.put(sessionId, title); }
                }
                break;
            }

            default:
                break;
        }
    }

    /** 按会话维护忙碌计数：turn/start 置忙，turn/end 置闲。 */
    private void setBusy(String sessionId, boolean busy) {
        Boolean prev = busyBySession.put(sessionId, busy);
        if (prev != null && prev == busy) return;
        if (busy) activeCount++;
        else activeCount = Math.max(0, activeCount - 1);
        notifySummary();
    }

    private static String rpc(String method, String payloadJson) {
        return "{\"type\":\"client-request\",\"rpcId\":\"" + UUID.randomUUID()
                + "\",\"method\":\"" + method + "\",\"payload\":" + payloadJson + "}";
    }
}
