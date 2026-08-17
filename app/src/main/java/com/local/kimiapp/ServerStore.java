package com.local.kimiapp;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

public final class ServerStore {
    private static final String PREFS = "kimi_servers";
    private static final String LEGACY = "kimi_connection";

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
        SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String raw = prefs.getString("items", "");
        List<Server> result = new ArrayList<>();
        if (!raw.isEmpty()) try {
            JSONArray items = new JSONArray(raw);
            for (int i = 0; i < items.length(); i++) {
                JSONObject item = items.getJSONObject(i);
                result.add(new Server(item.getString("id"), item.optString("name", "Kimi"),
                        item.getString("host"), item.getInt("port"), item.optString("token", ""),
                        item.optString("backend", Server.BACKEND_KIMI)));
            }
        } catch (Exception ignored) {}
        if (result.isEmpty()) {
            SharedPreferences legacy = context.getSharedPreferences(LEGACY, Context.MODE_PRIVATE);
            if (legacy.contains("ip")) {
                result.add(new Server(UUID.randomUUID().toString(), "默认服务器",
                        legacy.getString("ip", "100.95.189.73"), legacy.getInt("port", 58627),
                        legacy.getString("token", "")));
                save(context, result, result.get(0).id);
            }
        }
        return result;
    }

    public static synchronized void save(Context context, List<Server> servers, String activeId) {
        JSONArray items = new JSONArray();
        for (Server server : servers) try { items.put(server.json()); } catch (Exception ignored) {}
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
                .putString("items", items.toString()).putString("active", activeId).apply();
    }

    public static String activeId(Context context) {
        List<Server> servers = load(context);
        String id = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString("active", "");
        for (Server server : servers) if (server.id.equals(id)) return id;
        return servers.isEmpty() ? "" : servers.get(0).id;
    }

    public static Server active(Context context) {
        String id = activeId(context);
        for (Server server : load(context)) if (server.id.equals(id)) return server;
        return null;
    }

    public static String newId() { return UUID.randomUUID().toString(); }
}
