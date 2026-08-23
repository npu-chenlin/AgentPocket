package com.local.kimiapp;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * 服务器配置存储。采用与桌面端共享的统一交换格式
 * {"schema":1,"activeId":"...","servers":[...]}；
 * 加载时兼容旧纯数组格式，下次保存自动迁移。
 */
public final class ServerStore {
    private static final String PREFS = "kimi_servers";
    private static final int SCHEMA = 1;

    public static final class Server {
        public static final String BACKEND_KIMI = "kimi";
        public static final String BACKEND_DSH = "dsh";
        public String id, name, host, token;
        public int port;
        /** 后端协议类型：BACKEND_KIMI（Kimi Code web）或 BACKEND_DSH（DeepSeek Harness web）。 */
        public String backend;
        public Server(String id, String name, String host, int port, String token) {
            this(id, name, host, port, token, BACKEND_KIMI);
        }
        public Server(String id, String name, String host, int port, String token, String backend) {
            this.id = id; this.name = name; this.host = host; this.port = port; this.token = token;
            this.backend = backend == null || backend.isEmpty() ? BACKEND_KIMI : backend;
        }
        public String baseUrl() { return "http://" + host + ":" + port; }
        JSONObject json() throws Exception { return new JSONObject().put("id", id).put("name", name)
                .put("host", host).put("port", port).put("token", token).put("backend", backend); }
    }

    private ServerStore() {}

    public static synchronized List<Server> load(Context context) {
        return readState(context).servers;
    }

    public static synchronized void save(Context context, List<Server> servers, String activeId) {
        writeState(context, servers, activeId);
    }

    /**
     * 在同一把锁内完成读取、合并和保存，避免同步回调与用户编辑互相覆盖。
     * 返回实际处理的有效条目数。
     */
    public static synchronized int merge(Context context, JSONArray items) {
        State state = readState(context);
        List<Server> servers = new ArrayList<>(state.servers);
        int count = 0;
        for (int i = 0; i < items.length(); i++) {
            JSONObject item = items.optJSONObject(i);
            if (item == null) continue;
            String host = item.optString("host", "");
            int port = item.optInt("port", 0);
            if (host.isEmpty() || port < 1 || port > 65535) continue;
            String id = item.optString("id", "");
            if (id.isEmpty()) id = newId();
            String name = item.optString("name", "");
            if (name.isEmpty()) name = host + ":" + port;
            String backend = item.optString("backend", "");
            if (backend.isEmpty()) backend = Server.BACKEND_KIMI;
            Server incoming = new Server(id, name, host, port,
                    item.optString("token", ""), backend);
            int existing = -1;
            for (int j = 0; j < servers.size(); j++) {
                if (servers.get(j).id.equals(id)) { existing = j; break; }
            }
            if (existing >= 0) servers.set(existing, incoming);
            else servers.add(incoming);
            count++;
        }
        if (count > 0) {
            String activeId = state.activeId;
            if (activeId == null || activeId.isEmpty()) activeId = servers.get(0).id;
            writeState(context, servers, activeId);
        }
        return count;
    }

    private static void writeState(Context context, List<Server> servers, String activeId) {
        JSONArray items = new JSONArray();
        for (Server server : servers) try { items.put(server.json()); } catch (Exception ignored) {}
        JSONObject doc = new JSONObject();
        try {
            doc.put("schema", SCHEMA)
                    .put("activeId", activeId == null ? "" : activeId)
                    .put("servers", items);
        } catch (Exception ignored) {}
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putString("items", doc.toString()).apply();
    }

    public static String activeId(Context context) {
        State state = readState(context);
        for (Server server : state.servers) if (server.id.equals(state.activeId)) return state.activeId;
        return state.servers.isEmpty() ? "" : state.servers.get(0).id;
    }

    public static Server active(Context context) {
        String id = activeId(context);
        for (Server server : load(context)) if (server.id.equals(id)) return server;
        return null;
    }

    public static String newId() { return UUID.randomUUID().toString(); }

    private static final class State {
        final List<Server> servers;
        final String activeId;
        State(List<Server> servers, String activeId) { this.servers = servers; this.activeId = activeId; }
    }

    /** 解析存储内容：先按统一对象格式，失败回退旧纯数组格式（迁移期兼容）。 */
    private static State readState(Context context) {
        SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String raw = prefs.getString("items", "");
        List<Server> servers = new ArrayList<>();
        String activeId = "";
        if (!raw.isEmpty()) try {
            JSONObject doc = new JSONObject(raw);
            activeId = doc.optString("activeId", "");
            JSONArray items = doc.optJSONArray("servers");
            if (items != null) servers = parseItems(items);
        } catch (Exception objectFailed) {
            try {
                servers = parseItems(new JSONArray(raw));
                activeId = prefs.getString("active", "");
            } catch (Exception ignored) {}
        }
        return new State(servers, activeId);
    }

    private static List<Server> parseItems(JSONArray items) {
        List<Server> result = new ArrayList<>();
        for (int i = 0; i < items.length(); i++) try {
            JSONObject item = items.getJSONObject(i);
            result.add(new Server(item.getString("id"), item.optString("name", "Kimi"),
                    item.getString("host"), item.getInt("port"), item.optString("token", ""),
                    item.optString("backend", Server.BACKEND_KIMI)));
        } catch (Exception ignored) {}
        return result;
    }
}
