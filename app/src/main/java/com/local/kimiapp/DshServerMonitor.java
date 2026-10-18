package com.local.kimiapp;

import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.atomic.AtomicLong;
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
 * DeepSeek Harness (dsh) web 服务器监听器 —— 0.2 协议。
 *
 * 0.1 那套"一条全局下游流，帧里带 session/event、approval/requested"的模型在
 * 0.2 已经没有了。现在分三条：
 *
 * 1. HTTP RPC：POST /api/&lt;method&gt;，信封
 *    {"type":"client-request","rpcId":"...","method":"...","payload":{"args":{...}}}
 *    session/list 的参数名是 <code>_request</code>，响应
 *    {"type":"server-response","rpcId":"...","result":{"ok":true,"value":{...}}}
 * 2. WebSocket /api/remote.mux：连接后先发 open 帧订阅，之后收 item 帧。
 *    - <code>$events</code>：会话增删、running 变化、agent 失败、审批/提问请求
 *    - <code>session/follow</code>：单会话的 turn/start、turn/end
 * 3. 认证：/api 与页面都要求浏览器会话 cookie；由 DshAuth 解析配置里存的那条 cookie。
 *
 * 审批与提问是 waterfall 帧，**必须回一条**：所有客户端都不回，服务端会一直
 * 挂着等，真人那边的审批就被卡住了。这里只回 next，把决定权留给 Web UI。
 */
public class DshServerMonitor extends ServerMonitor {
    private static final String TAG = "DshMonitor";
    private static final MediaType JSON = MediaType.parse("application/json; charset=utf-8");
    /** 与桌面端一致：会话再多也不该把连接数拉爆。 */
    private static final int MAX_FOLLOWS = 16;

    private final Map<String, Boolean> busyBySession = Collections.synchronizedMap(new HashMap<>());
    /** 子代理会话：不单独订阅，它们由父会话体现。 */
    private final Set<String> subagentSessions = Collections.synchronizedSet(new HashSet<>());
    /** 每个会话一条 follow 连接。 */
    private final Map<String, WebSocket> follows = Collections.synchronizedMap(new HashMap<>());
    /** $events 连接拿到的 clientId，应答 waterfall 时要带上。 */
    private volatile String eventsClientId;
    /** 每次建立连接递增；旧 WebSocket 的回调不得影响当前连接。 */
    private final AtomicLong socketGeneration = new AtomicLong();

    public DshServerMonitor(MonitorHost host, ServerStore.Server server, OkHttpClient client) {
        super(host, server, client);
    }

    @Override public void start() {
        if (stopped) return;
        connected = false;
        notifySummary();
        new Thread(() -> {
            List<String> followable = fetchSessionList();
            if (stopped) return;
            connectEvents();
            for (String sessionId : followable) followSession(sessionId);
        }, "dsh-monitor").start();
    }

    // ------------------------------------------------------------------
    // 会话列表（HTTP RPC）
    // ------------------------------------------------------------------

    /** 拉一次会话列表；返回需要 follow 的非子代理会话 id。 */
    private List<String> fetchSessionList() {
        List<String> followable = new ArrayList<>();
        String cookie = DshAuth.cookie(server);
        if (cookie == null) {
            Log.w(TAG, server.name + " 配置里没有可用的 dsh cookie");
            scheduleReconnect();
            return followable;
        }
        // session/list 的参数名是 _request，不是 request。
        Request request = new Request.Builder()
                .url(server.baseUrl() + "/api/session/list")
                .header("Cookie", cookie)
                .post(RequestBody.create(rpc("session/list", "{\"_request\":{}}"), JSON))
                .build();
        try (Response response = client.newCall(request).execute()) {
            if (response.code() == 401) {
                // cookie 过期或换了访问地址（cookie 名绑定 authority），症状就是 401。
                throw new IOException("dsh 认证失败：重新粘贴一次启动行以取得新 cookie");
            }
            if (!response.isSuccessful()) throw new IOException("HTTP " + response.code());
            JSONObject result = new JSONObject(response.body().string()).optJSONObject("result");
            if (result == null || !result.optBoolean("ok", false)) throw new IOException("RPC not ok");
            JSONArray items = result.optJSONObject("value").optJSONArray("items");
            synchronized (titleCache) {
                titleCache.clear();
            }
            busyBySession.clear();
            subagentSessions.clear();
            for (int i = 0; i < items.length(); i++) {
                JSONObject item = items.getJSONObject(i);
                String id = item.getString("sessionId");
                boolean subagent = "subagent".equals(item.optString("origin", ""));
                if (subagent) subagentSessions.add(id);
                busyBySession.put(id, item.optBoolean("running", false));
                String title = titleOf(item);
                if (!title.isEmpty()) {
                    synchronized (titleCache) {
                        titleCache.put(id, title);
                    }
                }
                if (!subagent) followable.add(id);
            }
            activeCount = busyCount();
            notifySummary();
        } catch (Exception e) {
            Log.w(TAG, server.name + " session/list 失败", e);
            scheduleReconnect();
        }
        return followable;
    }

    /** 会话标题：projections.values.title 非空才算，服务端会给字面量 "null"。 */
    private static String titleOf(JSONObject item) {
        JSONObject projections = item.optJSONObject("projections");
        JSONObject values = projections != null ? projections.optJSONObject("values") : null;
        String title = values != null ? values.optString("title", "") : "";
        return title.isEmpty() || "null".equals(title) ? "" : title;
    }

    // ------------------------------------------------------------------
    // WebSocket（/api/remote.mux）
    // ------------------------------------------------------------------

    private String muxUrl() {
        return server.baseUrl().replace("http://", "ws://").replace("https://", "wss://")
                + "/api/remote.mux";
    }

    /** 全局事件流：会话集合与 running 变化、审批 / 提问。 */
    private void connectEvents() {
        if (stopped) return;
        final long generation = socketGeneration.incrementAndGet();
        if (webSocket != null) webSocket.cancel();
        closeFollows();

        String cookie = DshAuth.cookie(server);
        if (cookie == null) {
            scheduleReconnect();
            return;
        }
        Request.Builder builder = new Request.Builder().url(muxUrl()).header("Cookie", cookie);
        webSocket = client.newWebSocket(builder.build(), new WebSocketListener() {
            @Override public void onOpen(WebSocket ws, Response response) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.i(TAG, server.name + " $events open");
                ws.send(openFrame("events", "$events", new JSONObject()));
            }

            @Override public void onMessage(WebSocket ws, String text) {
                if (isCurrentSocket(ws, generation)) handleEventsFrame(ws, text);
            }

            @Override public void onClosed(WebSocket ws, int code, String reason) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.i(TAG, server.name + " $events closed: " + code + " " + reason);
                onEventsDown();
            }

            @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.w(TAG, server.name + " $events failure: " + t.getMessage());
                onEventsDown();
            }
        });
    }

    private void onEventsDown() {
        connected = false;
        setHealth(false);
        notifySummary();
        closeFollows();
        scheduleReconnect();
    }

    private void closeFollows() {
        synchronized (follows) {
            for (WebSocket socket : follows.values()) socket.cancel();
            follows.clear();
        }
    }

    /** 单会话事件订阅：turn/start、turn/end。 */
    private void followSession(final String sessionId) {
        if (stopped || subagentSessions.contains(sessionId)) return;
        synchronized (follows) {
            if (follows.containsKey(sessionId) || follows.size() >= MAX_FOLLOWS) return;
        }
        String cookie = DshAuth.cookie(server);
        if (cookie == null) return;

        Request request = new Request.Builder().url(muxUrl()).header("Cookie", cookie).build();
        WebSocket socket = client.newWebSocket(request, new WebSocketListener() {
            @Override public void onOpen(WebSocket ws, Response response) {
                JSONObject address = new JSONObject();
                try {
                    address.put("kind", "session").put("sessionId", sessionId);
                } catch (JSONException ignored) {}
                JSONObject request = new JSONObject();
                try {
                    request.put("address", address);
                } catch (JSONException ignored) {}
                ws.send(openFrame("follow-" + sessionId, "session/follow", request));
            }

            @Override public void onMessage(WebSocket ws, String text) {
                handleFollowFrame(sessionId, text);
            }

            @Override public void onClosed(WebSocket ws, int code, String reason) {
                follows.remove(sessionId);
            }

            @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                follows.remove(sessionId);
            }
        });
        synchronized (follows) {
            follows.put(sessionId, socket);
        }
    }

    private boolean isCurrentSocket(WebSocket ws, long generation) {
        return !stopped && generation == socketGeneration.get() && webSocket == ws;
    }

    // ------------------------------------------------------------------
    // 帧解析
    // ------------------------------------------------------------------

    private void handleEventsFrame(WebSocket ws, String text) {
        try {
            JSONObject frame = new JSONObject(text);
            if (!"item".equals(frame.optString("type", ""))) return;
            JSONObject value = frame.optJSONObject("value");
            if (value == null) return;
            String type = value.optString("type", "");

            if ("ready".equals(type)) {
                eventsClientId = value.optString("clientId", "");
                connected = true;
                reconnectDelay = RECONNECT_BASE_MS;
                setHealth(true);
                notifySummary();
                return;
            }
            if ("waterfall".equals(type)) {
                handleWaterfall(ws, value);
                return;
            }
            if (!"emit".equals(type)) return;

            String event = value.optString("event", "");
            JSONArray args = value.optJSONArray("args");
            if (args == null) return;
            switch (event) {
                case "api-session/status": {
                    // args = [sessionId, running]
                    String sessionId = args.optString(0, "");
                    if (sessionId.isEmpty()) return;
                    setBusy(sessionId, args.optBoolean(1, false));
                    break;
                }
                case "api-session/added": {
                    // 新会话会出现两次（session/created 与 agent/created 各一次），按 id upsert。
                    JSONObject summary = args.optJSONObject(0);
                    if (summary == null) return;
                    String id = summary.optString("sessionId", "");
                    if (id.isEmpty()) return;
                    boolean subagent = "subagent".equals(summary.optString("origin", ""));
                    if (subagent) subagentSessions.add(id);
                    String title = titleOf(summary);
                    if (!title.isEmpty()) {
                        synchronized (titleCache) {
                            titleCache.put(id, title);
                        }
                    }
                    if (summary.optBoolean("running", false)) setBusy(id, true);
                    // 会话集合变了才有必要跟随，这里补一条（列表刷新是另一条路）。
                    if (!subagent && !follows.containsKey(id)) followSession(id);
                    break;
                }
                case "api-session/removed": {
                    String sessionId = args.optString(0, "");
                    if (sessionId.isEmpty()) return;
                    setBusy(sessionId, false);
                    synchronized (titleCache) {
                        titleCache.remove(sessionId);
                    }
                    subagentSessions.remove(sessionId);
                    WebSocket socket = follows.remove(sessionId);
                    if (socket != null) socket.cancel();
                    break;
                }
                case "api-session/error": {
                    // 回合之外的 agent 失败：没有 turn/end 可依，只有这条。
                    String sessionId = args.optString(0, "");
                    if (sessionId.isEmpty()) return;
                    setBusy(sessionId, false);
                    publishEvent("aborted");
                    if (!MainActivity.isVisible) {
                        maybeNotify(sessionId, "agent-error:" + sessionId,
                                "DeepSeek Harness · 任务失败", args.optString(1, getTitle(sessionId)));
                    }
                    break;
                }
                default:
                    break;
            }
        } catch (JSONException e) {
            Log.w(TAG, server.name + " $events 坏帧: " + text.substring(0, Math.min(200, text.length())));
        }
    }

    /**
     * 审批 / 提问请求。
     *
     * 帧里那个字段叫 agentId，但 dsh 的 agent 就是以 session id 为键创建的，所以它就是会话 id。
     * 必须应答：只回 next 不认领，交回 Web UI 处理。
     */
    private void handleWaterfall(WebSocket ws, JSONObject value) {
        String event = value.optString("event", "");
        String eventId = value.optString("eventId", "");
        String sessionId = value.optString("agentId", "");
        boolean approval = "approval/request".equals(event);

        if (approval) {
            publishEvent("approval");
            if (!MainActivity.isVisible) {
                JSONObject request = value.optJSONObject("request");
                String toolName = request != null ? request.optString("toolName", "") : "";
                String body = toolName.isEmpty() ? getTitle(sessionId)
                        : "请求调用 " + toolName + " · " + getTitle(sessionId);
                maybeNotify(sessionId, "approval:" + eventId,
                        "DeepSeek Harness · 等待审批", body);
            }
        } else {
            publishEvent("question");
            if (!MainActivity.isVisible) {
                maybeNotify(sessionId, "question:" + eventId,
                        "DeepSeek Harness · 待回答", getTitle(sessionId));
            }
        }

        String clientId = eventsClientId;
        if (clientId != null && !clientId.isEmpty()) {
            ws.send(resultFrame(clientId, eventId, "next"));
        }
    }

    /** 单会话 follow 帧。 */
    private void handleFollowFrame(String sessionId, String text) {
        try {
            JSONObject frame = new JSONObject(text);
            if (!"item".equals(frame.optString("type", ""))) return;
            JSONObject value = frame.optJSONObject("value");
            if (value == null) return;
            String type = value.optString("type", "");
            if ("snapshot".equals(type)) {
                // 打开时回放一段窗口，里面的 turn/end 是历史，不该通知。
                return;
            }
            if (!"event".equals(type)) return;
            JSONObject event = value.optJSONObject("event");
            if (event != null) handleSessionEvent(sessionId, event);
        } catch (JSONException e) {
            Log.w(TAG, server.name + " follow 坏帧: " + text.substring(0, Math.min(200, text.length())));
        }
    }

    /** 解析 SessionEvent（follow 帧的 event 字段），只关心回合开始/结束与标题。 */
    private void handleSessionEvent(String sessionId, JSONObject event) {
        String type = event.optString("type", "");
        JSONObject data = event.optJSONObject("data");
        switch (type) {
            case "turn/start":
                setBusy(sessionId, true);
                break;

            case "turn/end": {
                setBusy(sessionId, false);
                JSONObject reason = data != null ? data.optJSONObject("reason") : null;
                String kind = reason != null ? reason.optString("kind", "") : "";
                long seq = event.optLong("seq", -1);
                String key = "turn-end:" + seq;
                if ("completed".equals(kind)) {
                    publishEvent("complete");
                    if (!MainActivity.isVisible) {
                        notifyTurnFinished(sessionId, "completed", key);
                    }
                } else if ("error".equals(kind) || "aborted".equals(kind)
                        || "blocked".equals(kind) || "max-tokens".equals(kind)
                        || "interrupted".equals(kind) || "forked".equals(kind)) {
                    publishEvent("aborted");
                    if (!MainActivity.isVisible) {
                        // 失败时把模型的报错带出来——"任务失败"四个字对排查没有帮助。
                        String body = "error".equals(kind) && reason != null
                                && reason.optJSONObject("error") != null
                                ? reason.optJSONObject("error").optString("message", getTitle(sessionId))
                                : getTitle(sessionId);
                        maybeNotify(sessionId, key, "DeepSeek Harness · 回合失败", body);
                    }
                }
                break;
            }

            case "session/title": {
                String title = data != null ? data.optString("title", "") : "";
                if (!title.isEmpty() && !"null".equals(title)) {
                    synchronized (titleCache) {
                        titleCache.put(sessionId, title);
                    }
                }
                break;
            }

            default:
                break;
        }
    }

    /** 按会话维护忙碌集合：turn/start 置忙，turn/end 置闲。 */
    private void setBusy(String sessionId, boolean busy) {
        Boolean prev = busyBySession.put(sessionId, busy);
        if (prev != null && prev == busy) return;
        activeCount = busyCount();
        notifySummary();
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
                    result.add(new String[]{ server.id, entry.getKey(), getTitle(entry.getKey()) });
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

    @Override public void shutdown() {
        closeFollows();
        super.shutdown();
    }

    // ------------------------------------------------------------------
    // 帧构造
    // ------------------------------------------------------------------

    /** mux 上的第 0 帧必须是 open，之后服务端才会推 item。 */
    private static String openFrame(String streamId, String endpoint, JSONObject args) {
        try {
            return new JSONObject()
                    .put("type", "open")
                    .put("streamId", streamId)
                    .put("endpoint", endpoint)
                    .put("payload", new JSONObject().put("args", args))
                    .toString();
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }

    /** waterfall 的应答：kind=next 表示不认领。 */
    private static String resultFrame(String clientId, String eventId, String outcome) {
        try {
            JSONObject args = new JSONObject()
                    .put("clientId", clientId)
                    .put("eventId", eventId)
                    .put("outcome", new JSONObject().put("kind", outcome));
            return new JSONObject()
                    .put("type", "client-request")
                    .put("rpcId", UUID.randomUUID().toString())
                    .put("method", "$events/result")
                    .put("payload", new JSONObject().put("args", args))
                    .toString();
        } catch (JSONException e) {
            throw new IllegalStateException(e);
        }
    }

    private static String rpc(String method, String argsJson) {
        return "{\"type\":\"client-request\",\"rpcId\":\"" + UUID.randomUUID()
                + "\",\"method\":\"" + method + "\",\"payload\":{\"args\":" + argsJson + "}}";
    }
}
