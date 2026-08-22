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

    // 会话当前活动展示（与桌面端 transcript 相位逻辑一致）：
    // activityBySession: sessionId -> 展示文本（如 "Bash · git push" / "思考中"）
    // toolCommands: sessionId -> (toolCallId -> 命令首行预览)
    // currentTool: sessionId -> [toolCallId, 工具名]
    private final Map<String, String> activityBySession = Collections.synchronizedMap(new HashMap<>());
    private final Map<String, Map<String, String>> toolCommands = Collections.synchronizedMap(new HashMap<>());
    private final Map<String, String[]> currentTool = Collections.synchronizedMap(new HashMap<>());
    private long lastActivityNotify;

    // 忙碌细分状态（全部来自服务器推送，不做周期轮询）：
    // rawBusy        = work_changed 推送的服务器侧 busy
    // mainTurnActive = work_changed 推送的 main_turn_active（主 agent 是否在回合内）
    // bgRunning      = background.task.started/terminated 事件维护的后台任务运行数
    // 有效忙碌 = rawBusy && (主回合活跃 || 还有后台任务)，避免「主 agent 已完成仍转圈」。
    private final Map<String, Boolean> rawBusy = Collections.synchronizedMap(new HashMap<>());
    private final Map<String, Boolean> mainTurnActive = Collections.synchronizedMap(new HashMap<>());
    private final Map<String, Integer> bgRunning = Collections.synchronizedMap(new HashMap<>());

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

    /** 忙碌细分状态：主会话在干活 / 等子结果，还是回合已结束只剩后台任务。 */
    private String sessionState(String sessionId) {
        String pending = pendingBySession.get(sessionId);
        if ("approval".equals(pending)) return "approval";
        if ("question".equals(pending)) return "question";
        if (Boolean.FALSE.equals(mainTurnActive.get(sessionId))) return "background";
        return "working";
    }

    @Override public List<String[]> busySessions() {
        List<String[]> result = new ArrayList<>();
        synchronized (busyBySession) {
            for (Map.Entry<String, Boolean> entry : busyBySession.entrySet()) {
                if (!Boolean.TRUE.equals(entry.getValue())) continue;
                String sessionId = entry.getKey();
                String state = sessionState(sessionId);
                String activity;
                switch (state) {
                    case "approval":
                        activity = "等待审批";
                        break;
                    case "question":
                        activity = "等待回答";
                        break;
                    case "background":
                        int running = bgRunning.containsKey(sessionId) ? bgRunning.get(sessionId) : 0;
                        activity = running > 0
                                ? "主 agent 已完成 · 等 " + running + " 个后台任务"
                                : "主 agent 已完成";
                        break;
                    default:
                        String text = activityBySession.get(sessionId);
                        activity = text != null ? text : "";
                }
                result.add(new String[]{ server.id, sessionId, getTitle(sessionId), activity, state });
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
                    List<String> busyIds = new ArrayList<>();
                    synchronized (titleCache) {
                        titleCache.clear();
                        busyBySession.clear();
                        rawBusy.clear();
                        pendingBySession.clear();
                        activityBySession.clear();
                        toolCommands.clear();
                        currentTool.clear();
                        mainTurnActive.clear();
                        bgRunning.clear();
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
                            if (busy) busyIds.add(id);
                        }
                        activeCount = busyCount();
                    }
                    // 列表只有粗粒度 status，忙碌会话逐个取详情区分主/后台状态
                    for (String id : busyIds) fetchSessionDetail(id);
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
                    // 订阅 transcript 块粒度流，用于展示会话当前活动（与桌面端一致）
                    for (String id : ids) {
                        if (Boolean.TRUE.equals(busyBySession.get(id))) {
                            sendJson(ws, buildSubscribeV2(id));
                        }
                    }
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

                case "background.task.started":
                case "background.task.terminated": {
                    handleBackgroundTaskEvent(type, msg.optString("session_id", ""));
                    break;
                }

                case "transcript.ops": {
                    String sessionId = msg.optString("session_id", "");
                    JSONArray ops = msg.optJSONObject("payload") != null
                            ? msg.optJSONObject("payload").optJSONArray("ops") : null;
                    if (!sessionId.isEmpty() && ops != null) { applyActivityOps(sessionId, ops); refreshActivityThrottled(); }
                    break;
                }

                case "transcript.reset": {
                    String sessionId = msg.optString("session_id", "");
                    JSONObject phase = optPath(msg, "payload", "snapshot", "meta", "agent", "phase");
                    if (!sessionId.isEmpty() && phase != null) { applyPhase(sessionId, phase); refreshActivityThrottled(); }
                    break;
                }

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
                    // 状态跃迁等价于主回合活跃性变化；busy 以 work_changed 推送为准
                    mainTurnActive.put(sessionId, isActive);
                    applyEffectiveBusy(sessionId);
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
                // 推送自带细分字段，直接落状态，不再回查 REST
                rawBusy.put(sessionId, payload.optBoolean("busy", false));
                if (payload.has("main_turn_active")) {
                    mainTurnActive.put(sessionId, payload.optBoolean("main_turn_active", true));
                }
                applyEffectiveBusy(sessionId);

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
                        rawBusy.put(newId, true);
                        mainTurnActive.put(newId, true);
                        applyEffectiveBusy(newId);
                        if (webSocket != null) {
                            try {
                                sendJson(webSocket, buildSubscribe(Collections.singletonList(newId)));
                                sendJson(webSocket, buildSubscribeV2(newId));
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

    /** 启动种子：连接时对忙碌会话取一次详情（含 main_turn_active），之后的状态全靠推送。 */
    private void fetchSessionDetail(String sessionId) {
        HttpUrl url;
        try {
            url = HttpUrl.get(server.baseUrl() + "/api/v1/sessions/" + sessionId);
        } catch (Exception e) { return; }
        client.newCall(authorize(url)).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {}
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (!response.isSuccessful()) return;
                    JSONObject data = new JSONObject(response.body().string()).optJSONObject("data");
                    if (data == null) return;
                    rawBusy.put(sessionId, data.optBoolean("busy", false));
                    boolean mainActive = data.optBoolean("main_turn_active", true);
                    mainTurnActive.put(sessionId, mainActive);
                    pendingBySession.put(sessionId, data.optString("pending_interaction", "none"));
                    if (!mainActive) fetchRunningTaskCount(sessionId);
                    else applyEffectiveBusy(sessionId);
                } catch (Exception ignored) {}
            }
        });
    }

    /** 启动种子：主回合已结束的会话，取一次后台任务数作为事件计数的初值。 */
    private void fetchRunningTaskCount(String sessionId) {
        HttpUrl url;
        try {
            url = HttpUrl.get(server.baseUrl() + "/api/v1/sessions/" + sessionId + "/tasks");
        } catch (Exception e) { return; }
        client.newCall(authorize(url)).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {}
            @Override public void onResponse(Call call, Response response) {
                try (Response ignored = response) {
                    if (!response.isSuccessful()) return;
                    JSONObject root = new JSONObject(response.body().string());
                    JSONArray items = null;
                    JSONObject data = root.optJSONObject("data");
                    if (data != null) items = data.optJSONArray("items");
                    if (items == null) items = root.optJSONArray("data");
                    if (items == null) items = new JSONArray();
                    int running = 0;
                    for (int i = 0; i < items.length(); i++) {
                        JSONObject task = items.optJSONObject(i);
                        if (task != null && "running".equals(task.optString("status"))) running++;
                    }
                    bgRunning.put(sessionId, running);
                    applyEffectiveBusy(sessionId);
                } catch (Exception ignored) {}
            }
        });
    }

    /** 后台任务生命周期事件：增减运行计数后重算有效忙碌。 */
    private void handleBackgroundTaskEvent(String type, String sessionId) {
        if (sessionId.isEmpty()) return;
        int delta = "background.task.started".equals(type) ? 1 : -1;
        synchronized (bgRunning) {
            int current = bgRunning.containsKey(sessionId) ? bgRunning.get(sessionId) : 0;
            bgRunning.put(sessionId, Math.max(0, current + delta));
        }
        applyEffectiveBusy(sessionId);
    }

    /** 有效忙碌 = 服务器报 busy 且（主回合活跃 或 仍有后台任务）；变空闲即触发置顶完成链路。 */
    private void applyEffectiveBusy(String sessionId) {
        boolean raw = Boolean.TRUE.equals(rawBusy.get(sessionId));
        boolean mainActive = !Boolean.FALSE.equals(mainTurnActive.get(sessionId));
        int running;
        synchronized (bgRunning) {
            running = bgRunning.containsKey(sessionId) ? bgRunning.get(sessionId) : 0;
        }
        boolean effective = raw && (mainActive || running > 0);
        boolean was = Boolean.TRUE.equals(busyBySession.get(sessionId));
        busyBySession.put(sessionId, effective);
        if (!effective) clearActivity(sessionId);
        if (effective != was) activeCount = busyCount();
        notifySummary();
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

    // --- transcript 活动展示（与桌面端 protocol/kimi.rs 逻辑一致） ---

    private static JSONObject optPath(JSONObject root, String... keys) {
        JSONObject cur = root;
        for (String key : keys) {
            if (cur == null) return null;
            cur = cur.optJSONObject(key);
        }
        return cur;
    }

    /** 命令预览：取首个非空行并截断到 80 字符，避免把整段脚本塞进界面。 */
    private static String commandPreview(String inputText) {
        String firstLine = "";
        for (String line : inputText.split("\n", -1)) {
            String t = line.trim();
            if (!t.isEmpty()) { firstLine = t; break; }
        }
        final int MAX = 80;
        if (firstLine.length() <= MAX) return firstLine;
        return firstLine.substring(0, MAX) + "…";
    }

    private void applyPhase(String sessionId, JSONObject phase) {
        String kind = phase.optString("kind", "");
        if ("tool_call".equals(kind)) {
            String toolCallId = phase.optString("toolCallId", "");
            String name = phase.optString("name", "工具");
            Map<String, String> cmds = toolCommands.get(sessionId);
            String command = (cmds != null && !toolCallId.isEmpty()) ? cmds.get(toolCallId) : null;
            String display = command != null ? name + " · " + command : name;
            currentTool.put(sessionId, new String[]{ toolCallId, name });
            activityBySession.put(sessionId, display);
        } else if ("streaming".equals(kind) || "running".equals(kind)) {
            currentTool.remove(sessionId);
            activityBySession.put(sessionId, "思考中");
        }
    }

    private void applyActivityOps(String sessionId, JSONArray ops) {
        for (int i = 0; i < ops.length(); i++) {
            JSONObject op = ops.optJSONObject(i);
            if (op == null) continue;
            String opType = op.optString("op", "");
            if ("meta.merge".equals(opType)) {
                JSONObject phase = optPath(op, "meta", "agent", "phase");
                if (phase != null) applyPhase(sessionId, phase);
            } else if ("frame.upsert".equals(opType)) {
                JSONObject frame = op.optJSONObject("frame");
                if (frame == null || !"tool".equals(frame.optString("kind", ""))) continue;
                String toolCallId = frame.optString("toolCallId", "");
                if (toolCallId.isEmpty()) continue;
                String inputText = frame.optString("inputText", "");
                if (!inputText.isEmpty()) {
                    String preview = commandPreview(inputText);
                    if (!preview.isEmpty()) {
                        Map<String, String> cmds = toolCommands.get(sessionId);
                        if (cmds == null) { cmds = Collections.synchronizedMap(new HashMap<>()); toolCommands.put(sessionId, cmds); }
                        cmds.put(toolCallId, preview);
                    }
                }
                // 命令到达时若相位正指向该工具，刷新展示文本。
                String[] cur = currentTool.get(sessionId);
                if (cur != null && cur[0].equals(toolCallId)) {
                    Map<String, String> cmds = toolCommands.get(sessionId);
                    String cmd = cmds != null ? cmds.get(toolCallId) : null;
                    if (cmd != null) activityBySession.put(sessionId, cur[1] + " · " + cmd);
                }
            } else if ("turn.upsert".equals(opType)) {
                // 新轮次开始，丢弃上一轮命令缓存防止无界增长。
                toolCommands.remove(sessionId);
            }
        }
    }

    private void clearActivity(String sessionId) {
        activityBySession.remove(sessionId);
        toolCommands.remove(sessionId);
        currentTool.remove(sessionId);
        rawBusy.remove(sessionId);
        mainTurnActive.remove(sessionId);
        bgRunning.remove(sessionId);
    }

    /** transcript 块粒度事件可能较密，节流 1s 刷新一次摘要/会话列表，避免高频重建通知。 */
    private void refreshActivityThrottled() {
        long now = System.currentTimeMillis();
        if (now - lastActivityNotify >= 1000) {
            lastActivityNotify = now;
            notifySummary();
        }
    }

    private static JSONObject buildSubscribeV2(String sessionId) throws JSONException {
        return new JSONObject()
                .put("type", "subscribe_v2")
                .put("id", UUID.randomUUID().toString())
                .put("payload", new JSONObject()
                        .put("session_id", sessionId)
                        .put("transcript", new JSONObject().put("main", "block")));
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
