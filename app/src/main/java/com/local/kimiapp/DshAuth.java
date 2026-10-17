package com.local.kimiapp;

import android.util.Base64;

import java.nio.charset.StandardCharsets;

import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/**
 * dsh（DeepSeek Harness）Web 认证。
 *
 * dsh 0.2 起 Web UI 与 /api 都要求浏览器会话 cookie；没带 cookie 一律 401
 * （正文 <code>dsh web authentication required; reopen the URL printed by dsh web.</code>）。
 *
 * 真正管用的凭据是 dsh 数据目录里那把**持久**签名密钥
 * （<code>~/.dsh/.credentials.yaml</code> 的 <code>client-connection/browser-session</code>），
 * 那份文件只在缺失时才生成、从不轮换。每次启动都变的 launch token 只打印在服务
 * 进程日志里，客户端拿不到，因此没有可用性。
 *
 * 所以做法与 Kimi 的访问令牌一致：把密钥填进服务连接的 token 字段，客户端自己签发
 * cookie。密钥落在客户端配置里——客户端本来就是可信边界。
 *
 * 与 dsh 的 <code>BrowserAuth.encodeCookie</code> 逐字段对齐：
 * <pre>
 *   名字  dsh-auth-&lt;base64url(sha256(authority))&gt;
 *   值    v1.&lt;base64url(json)&gt;.&lt;base64url(hmac-sha256(secret, body))&gt;
 *   载荷  {"version":1,"authority":...,"issuedAt":...,"expiresAt":...}，毫秒
 * </pre>
 * 键序不能变——HMAC 覆盖的是那个 JSON 字节串本身。authority 是 <code>host:port</code>，
 * cookie 按它绑定，所以同一台 dsh 在不同地址下登录不通用。
 */
public final class DshAuth {
    /** cookie 有效期（天）。与 dsh 自身 cookieMaxAgeDays 的默认值一致。 */
    private static final int COOKIE_MAX_AGE_DAYS = 30;

    private DshAuth() {}

    /** dsh 的 cookie 绑定 host:port。 */
    public static String authority(ServerStore.Server server) {
        return server.host + ":" + server.port;
    }

    /** URL-safe base64（无填充），与 dsh 的 encodeBase64Url 一致。 */
    private static String base64Url(byte[] bytes) {
        return Base64.encodeToString(bytes, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
    }

    /**
     * 严格解码密钥：dsh 只写 32 字节的 base64url。
     * 长度 %4==1 在 base64 里不可能出现，出现即视为填错。
     */
    private static byte[] decodeSecret(String secret) {
        String value = secret == null ? "" : secret.trim();
        if (value.isEmpty() || value.length() % 4 == 1) return null;
        if (!value.matches("[A-Za-z0-9_-]+")) return null;
        try {
            byte[] bytes = Base64.decode(value, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
            // 回编一次确认没有多余字符被静默丢弃。
            return base64Url(bytes).equals(value) ? bytes : null;
        } catch (IllegalArgumentException e) {
            return null;
        }
    }

    /** cookie 名：dsh-auth-&lt;base64url(sha256(authority))&gt;。 */
    public static String cookieName(String authority) {
        try {
            java.security.MessageDigest digest = java.security.MessageDigest.getInstance("SHA-256");
            return "dsh-auth-" + base64Url(digest.digest(authority.getBytes(StandardCharsets.UTF_8)));
        } catch (java.security.NoSuchAlgorithmException e) {
            // SHA-256 是 JRE 必备算法，走不到这里。
            throw new IllegalStateException(e);
        }
    }

    /**
     * 用密钥签发一个会话 cookie（<code>name=value</code>），密钥不合法时返回 null。
     */
    public static String mintCookie(String secret, String authority, long nowMs) {
        byte[] key = decodeSecret(secret);
        if (key == null) return null;

        long expiresAt = nowMs + COOKIE_MAX_AGE_DAYS * 24L * 60 * 60 * 1000;
        // 键序固定：签的是这个字符串本身。
        String payload = "{\"version\":1,\"authority\":" + jsonString(authority)
                + ",\"issuedAt\":" + nowMs + ",\"expiresAt\":" + expiresAt + "}";
        String body = base64Url(payload.getBytes(StandardCharsets.UTF_8));
        try {
            Mac mac = Mac.getInstance("HmacSHA256");
            mac.init(new SecretKeySpec(key, "HmacSHA256"));
            String signature = base64Url(mac.doFinal(body.getBytes(StandardCharsets.UTF_8)));
            return cookieName(authority) + "=v1." + body + "." + signature;
        } catch (Exception e) {
            return null;
        }
    }

    /** 这个服务当前可用的 cookie；拿不到密钥时返回 null，请求照常发出并如实报 401。 */
    public static String cookie(ServerStore.Server server) {
        if (!ServerStore.Server.BACKEND_DSH.equals(server.backend)) return null;
        return mintCookie(server.token, authority(server), System.currentTimeMillis());
    }

    /** 取 cookie 名（`=` 之前那半），用于 WebView 的 CookieManager。 */
    public static String cookieNameOf(String cookie) {
        int separator = cookie.indexOf('=');
        return separator <= 0 ? "" : cookie.substring(0, separator);
    }

    /** 取 cookie 值（`=` 之后那半）。 */
    public static String cookieValueOf(String cookie) {
        int separator = cookie.indexOf('=');
        return separator <= 0 ? "" : cookie.substring(separator + 1);
    }

    /** 最小 JSON 字符串转义——authority 只可能是 host:port，但没理由不挡住引号。 */
    private static String jsonString(String value) {
        StringBuilder out = new StringBuilder(value.length() + 2);
        out.append('"');
        for (int i = 0; i < value.length(); i++) {
            char c = value.charAt(i);
            if (c == '"' || c == '\\') out.append('\\').append(c);
            else if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
            else out.append(c);
        }
        return out.append('"').toString();
    }
}
