package com.local.kimiapp;

import android.content.Context;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.IOException;
import java.util.List;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;

/**
 * 与桌面端一次性同步服务通信的客户端。
 * 二维码内容格式：agentpocket://sync?host=<ip>&port=<port>&token=<uuid>
 * GET/POST http://<host>:<port>/config，请求头 X-Sync-Token 鉴权。
 */
public final class SyncClient {

    /** 扫码/深链解析出的同步目标。 */
    public static final class SyncLink {
        public final String host;
        public final int port;
        public final String token;

        private SyncLink(String host, int port, String token) {
            this.host = host;
            this.port = port;
            this.token = token;
        }

        String baseUrl() { return "http://" + host + ":" + port; }
    }

    public interface FetchCallback {
        void onMerged(int count);

        void onError(String message);
    }

    public interface UploadCallback {
        void onSent();

        void onError(String message);
    }

    private static final MediaType JSON = MediaType.parse("application/json; charset=utf-8");
    private static final Handler mainHandler = new Handler(Looper.getMainLooper());

    private SyncClient() {}

    /** 解析扫码/深链文本，非法返回 null。 */
    public static SyncLink parseLink(String text) {
        if (text == null) return null;
        Uri uri;
        try {
            uri = Uri.parse(text.trim());
        } catch (Exception e) {
            return null;
        }
        if (!"agentpocket".equals(uri.getScheme()) || !"sync".equals(uri.getHost())) return null;
        String host = uri.getQueryParameter("host");
        String portText = uri.getQueryParameter("port");
        String token = uri.getQueryParameter("token");
        if (host == null || host.isEmpty() || portText == null || token == null || token.isEmpty()) return null;
        int port;
        try {
            port = Integer.parseInt(portText);
        } catch (NumberFormatException e) {
            return null;
        }
        if (port < 1 || port > 65535) return null;
        return new SyncLink(host, port, token);
    }

    /** 从电脑端拉取配置并合并进本地存储，结果通过回调通知（主线程）。 */
    public static void fetch(OkHttpClient client, Context context, SyncLink link, FetchCallback callback) {
        Request request = new Request.Builder()
                .url(link.baseUrl() + "/config")
                .header("X-Sync-Token", link.token)
                .get()
                .build();
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {
                postError(callback, "同步失败：" + e.getMessage());
            }

            @Override public void onResponse(Call call, Response response) {
                try (Response res = response) {
                    if (res.code() == 403) {
                        postError(callback, "同步凭证无效，请重新扫码");
                        return;
                    }
                    if (!res.isSuccessful()) {
                        postError(callback, "同步失败：HTTP " + res.code());
                        return;
                    }
                    String body = res.body() == null ? "" : res.body().string();
                    JSONArray items;
                    try {
                        JSONObject doc = new JSONObject(body);
                        items = doc.optJSONArray("servers");
                        if (items == null) throw new Exception("missing servers");
                    } catch (Exception e) {
                        postError(callback, "电脑配置格式无法识别");
                        return;
                    }
                    int count = merge(context, items);
                    if (count == 0) {
                        postError(callback, "电脑配置中没有可用服务连接");
                    } else {
                        mainHandler.post(() -> callback.onMerged(count));
                    }
                } catch (Exception e) {
                    postError(callback, "电脑配置格式无法识别");
                }
            }
        });
    }

    /** 把本地服务器列表上传到电脑端（电脑端弹确认导入框）。 */
    public static void upload(OkHttpClient client, Context context, SyncLink link, UploadCallback callback) {
        List<ServerStore.Server> servers = ServerStore.load(context);
        if (servers.isEmpty()) {
            mainHandler.post(() -> callback.onError("还没有服务连接可发送"));
            return;
        }
        JSONArray items = new JSONArray();
        for (ServerStore.Server server : servers) try { items.put(server.json()); } catch (Exception ignored) {}
        JSONObject doc = new JSONObject();
        try {
            doc.put("schema", 1)
                    .put("activeId", ServerStore.activeId(context))
                    .put("servers", items);
        } catch (Exception ignored) {}
        Request request = new Request.Builder()
                .url(link.baseUrl() + "/config")
                .header("X-Sync-Token", link.token)
                .post(RequestBody.create(doc.toString(), JSON))
                .build();
        client.newCall(request).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) {
                mainHandler.post(() -> callback.onError("同步失败：" + e.getMessage()));
            }

            @Override public void onResponse(Call call, Response response) {
                try (Response res = response) {
                    if (res.code() == 403) {
                        mainHandler.post(() -> callback.onError("同步凭证无效，请重新扫码"));
                    } else if (res.isSuccessful()) {
                        mainHandler.post(() -> callback.onSent());
                    } else {
                        mainHandler.post(() -> callback.onError("同步失败：HTTP " + res.code()));
                    }
                }
            }
        });
    }

    /**
     * 合并电脑端下发的服务器列表：同 id 覆盖、否则追加。
     * 跳过 host 为空或端口非法的条目，返回实际合并条数。
     */
    private static int merge(Context context, JSONArray items) {
        return ServerStore.merge(context, items);
    }

    /** 切回主线程上报错误。 */
    private static void postError(FetchCallback callback, String message) {
        mainHandler.post(() -> callback.onError(message));
    }
}
