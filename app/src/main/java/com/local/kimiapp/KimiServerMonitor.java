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
import java.util.concurrent.atomic.AtomicLong;

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
    /** 每次建立连接递增；旧 WebSocket 的回调不得影响当前连接。 */
    private final AtomicLong socketGeneration = new AtomicLong();

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
        final long generation = socketGeneration.incrementAndGet();
        if (webSocket != null) webSocket.cancel();
        String wsUrl = server.baseUrl().replace("http://", "ws://").replace("https://", "wss://")
                + "/api/v1/ws";
        Request.Builder req = new Request.Builder().url(wsUrl);

        // Auth: Kimi Code 0.36+ uses Sec-WebSocket-Protocol with bearer token
        if (server.token != null && !server.token.isEmpty()) {
            req.header("Sec-WebSocket-Protocol", "kimi-code.bearer." + server.token);
        }

        webSocket = client.newWebSocket(req.build(), new WebSocketListener() {
            @Override public void onOpen(WebSocket ws, Response response) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.i(TAG, server.name + " WS open");
                connected = true;
                reconnectDelay = RECONNECT_BASE_MS;
                setHealth(true);
                notifySummary();

                // client_hello with subscriptions + cursors (required by 0.36+)
                // 只用基础订阅：相位/工具/任务事件都由它推送，不再订阅 transcript（省流量）
                List<String> ids = new ArrayList<>(titleCache.keySet());
                try {
                    sendJson(ws, buildClientHello(ids));
                } catch (JSONException e) {
                    Log.e(TAG, server.name + " hello failed", e);
                }
            }

            @Override public void onMessage(WebSocket ws, String text) {
                if (isCurrentSocket(ws, generation)) handleMessage(ws, generation, text);
            }

            @Override public void onClosing(WebSocket ws, int code, String reason) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.i(TAG, server.name + " WS closing: " + code + " " + reason);
                ws.close(code, reason);
            }

            @Override public void onClosed(WebSocket ws, int code, String reason) {
                if (!isCurrentSocket(ws, generation)) return;
                connected = false;
                setHealth(false);
                notifySummary();
                scheduleReconnect();
            }

            @Override public void onFailure(WebSocket ws, Throwable t, Response response) {
                if (!isCurrentSocket(ws, generation)) return;
                Log.w(TAG, server.name + " WS failure: " + t.getMessage());
                connected = false;
                setHealth(false);
                notifySummary();
                scheduleReconnect();
            }
        });
    }

    private boolean isCurrentSocket(WebSocket ws, long generation) {
        return !stopped && generation == socketGeneration.get() && webSocket == ws;
    }

    private void handleMessage(WebSocket ws, long generation, String text) {
        try {
            JSONObject msg = new JSONObject(text);
            String type = msg.optString("type", "");
            Log.d(TAG, server.name + " << " + type);

            // 只关心主 agent：子代理的回合完成/审批/相位等事件一律忽略，
            // 避免子代理触发通知、污染忙碌状态与活动行。
            JSONObject eventPayload = msg.optJSONObject("payload");
            String agentId = eventPayload != null ? eventPayload.optString("agentId", "") : "";
            if (!agentId.isEmpty() && !"main".equals(agentId)) return;

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
                    if (isCurrentSocket(ws, generation)) {
                        sendJson(ws, new JSONObject()
                                .put("type", "pong")
                                .put("payload", new JSONObject().put("nonce", msg.optJSONObject("payload") != null ? msg.optJSONObject("payload").opt("nonce") : UUID.randomUUID().toString())));
                    }
                    break;

                case "resync_required":
                    Log.i(TAG, server.name + " resync");
                    fetchSessionList(() -> {
                        if (isCurrentSocket(ws, generation) && !titleCache.isEmpty()) {
                            try {
                                sendJson(ws, buildSubscribe(new ArrayList<>(titleCache.keySet())));
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

                case "agent.status.updated": {
                    String sessionId = msg.optString("session_id", "");
                    JSONObject phase = msg.optJSONObject("payload") != null
                            ? msg.optJSONObject("payload").optJSONObject("phase") : null;
                    if (!sessionId.isEmpty() && phase != null) applyAgentPhase(sessionId, phase);
                    break;
                }

                case "tool.call.started": {
                    handleToolCallStarted(msg.optString("session_id", ""), msg.optJSONObject("payload"));
                    break;
                }

                case "turn.ended": {
                    // 轮次结束清命令缓存，防止无界增长
                    toolCommands.remove(msg.optString("session_id", ""));
                    break;
                }

                default:
                    if (type.startsWith("event.session.")) {
                        handleProtocolEvent(msg, ws, generation);
                    } else if (type.equals("prompt.submitted") || type.equals("prompt.completed") || type.equals("prompt.aborted")) {
                        handleAgentEvent(msg);
                    }
                    break;
            }
        } catch (JSONException e) {
            Log.w(TAG, server.name + " bad JSON: " + text.substring(0, Math.min(200, text.length())));
        }
    }

    private void handleProtocolEvent(JSONObject msg, WebSocket ws, long generation) {
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

                // 通知不在这里发：完成/失败走 prompt.*，审批/回答走 work_changed 的
                // pending 跃迁——单一通路，避免一次事件两条提醒。悬浮球表情照常切换。
                if ("running".equals(previous) && "idle".equals(status)) {
                    publishEvent("complete");
                } else if ("awaiting_approval".equals(status)) {
                    publishEvent("approval");
                } else if ("awaiting_question".equals(status)) {
                    publishEvent("question");
                } else if ("aborted".equals(status)) {
                    publishEvent("aborted");
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
                        if (isCurrentSocket(ws, generation)) {
                            try {
                                sendJson(ws, buildSubscribe(Collections.singletonList(newId)));
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

    // --- 活动展示：全部来自基础订阅推送的 agent 事件（相位 + 工具开始） ---

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

    /** 相位事件（基础订阅推送）：更新活动文本，区分思考/输出/工具。 */
    private void applyAgentPhase(String sessionId, JSONObject phase) {
        String kind = phase.optString("kind", "");
        if ("tool_call".equals(kind)) {
            String toolCallId = phase.optString("toolCallId", "");
            String name = phase.optString("name", "工具");
            currentTool.put(sessionId, new String[]{ toolCallId, name });
            Map<String, String> cmds = toolCommands.get(sessionId);
            String command = (cmds != null && !toolCallId.isEmpty()) ? cmds.get(toolCallId) : null;
            activityBySession.put(sessionId, toolDisplay(displayName(name), command));
        } else if ("streaming".equals(kind) || "running".equals(kind)) {
            currentTool.remove(sessionId);
            String stream = phase.optString("stream", "");
            activityBySession.put(sessionId, "assistant".equals(stream) ? "输出中" : "思考中");
        } else if ("ended".equals(kind)) {
            currentTool.remove(sessionId);
        }
        refreshActivityThrottled();
    }

    /** 工具开始事件（基础订阅推送）：命令预览入缓存；子代理按工具名识别。 */
    private void handleToolCallStarted(String sessionId, JSONObject payload) {
        if (payload == null || sessionId.isEmpty()) return;
        String toolCallId = payload.optString("toolCallId", "");
        if (toolCallId.isEmpty()) return;
        String name = payload.optString("name", "");
        JSONObject display = payload.optJSONObject("display");
        String command = display != null ? display.optString("command", "") : "";
        JSONObject args = payload.optJSONObject("args");
        if (command.isEmpty() && args != null) {
            command = args.optString("command", "");
            if (command.isEmpty()) command = args.optString("description", "");
        }
        String preview = commandPreview(command);
        if (!preview.isEmpty()) {
            Map<String, String> cmds = toolCommands.get(sessionId);
            if (cmds == null) { cmds = Collections.synchronizedMap(new HashMap<>()); toolCommands.put(sessionId, cmds); }
            cmds.put(toolCallId, preview);
        }
        // 相位已指向该工具时刷新展示（相位事件可能先到）
        String[] cur = currentTool.get(sessionId);
        if (cur != null && cur[0].equals(toolCallId)) {
            activityBySession.put(sessionId, toolDisplay(displayName(name.isEmpty() ? cur[1] : name), preview));
        }
        refreshActivityThrottled();
    }

    private static boolean isSubagentTool(String name) {
        return "Agent".equals(name) || "AgentSwarm".equals(name) || "Task".equals(name);
    }

    private static String displayName(String name) {
        return isSubagentTool(name) ? "子代理" : name;
    }

    private static String toolDisplay(String name, String command) {
        return command == null || command.isEmpty() ? name : name + " · " + command;
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
