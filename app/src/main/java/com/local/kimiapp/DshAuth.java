package com.local.kimiapp;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * dsh（DeepSeek Harness）Web 认证。
 *
 * dsh 0.2 起页面与 /api 都要浏览器会话 cookie，没带一律 401。签发 cookie 的是 dsh
 * 数据目录里那份**持久签名密钥**（<code>~/.dsh/.credentials.yaml</code>），客户端拿不到，
 * 也不该拿——复刻它的 HMAC 等于把 dsh 的内部签名格式抄一份到自己家里。
 *
 * dsh 自己提供了获取 cookie 的正规入口：启动行里的 launch token。
 * <pre>
 *   GET http://host:port/?token=&lt;launch token&gt;   →   303 + Set-Cookie: dsh-auth-…=v1.…
 * </pre>
 *
 * 两者的寿命不同，这是整个设计的关键：
 * <ul>
 *   <li>launch token 每次启动都重新生成，只在**当前进程**有效；</li>
 *   <li>换来的 cookie 由持久密钥签发，30 天有效，**dsh 重启后依然有效**（实测）。</li>
 * </ul>
 *
 * 所以客户端只需要把 cookie 存下来、当普通 cookie 用。cookie 过期或服务地址变了
 * （cookie 名绑定 authority）时，重新粘贴一次启动行即可。
 */
public final class DshAuth {
    /** dsh 下发的 cookie 形如 dsh-auth-&lt;base64url(sha256(authority))&gt;=v1.&lt;body&gt;.&lt;sig&gt;。 */
    private static final Pattern COOKIE_PAIR = Pattern.compile(
            "(dsh-auth-[A-Za-z0-9_-]+)=(v1\\.[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+)");
    /** 启动行 / URL 里的 launch token。 */
    private static final Pattern LAUNCH_TOKEN = Pattern.compile("[?&]token=([A-Za-z0-9_-]+)");

    private DshAuth() {}

    /** 已存下的 <code>name=value</code>；配置里不是 cookie 时返回 null。 */
    public static String cookie(ServerStore.Server server) {
        if (server == null || !ServerStore.Server.BACKEND_DSH.equals(server.backend)) return null;
        return parseCookie(server.token);
    }

    /**
     * 从存储值里解析出 cookie 对。
     *
     * 容忍整条 Set-Cookie 被存进来（属性在第一个分号之后，丢掉）；名字必须像 dsh 自己
     * 那种、值必须三段式，否则视为没填——早于发请求就拒掉，比发出一个必然 401 的请求好。
     */
    public static String parseCookie(String stored) {
        if (stored == null) return null;
        String raw = stored.trim();
        if (raw.isEmpty()) return null;
        int semicolon = raw.indexOf(';');
        if (semicolon >= 0) raw = raw.substring(0, semicolon).trim();
        Matcher matcher = COOKIE_PAIR.matcher(raw);
        return matcher.matches() ? matcher.group(1) + "=" + matcher.group(2) : null;
    }

    /** 从粘贴的启动行 / URL 里取出 launch token；没有则返回 null。 */
    public static String launchToken(String text) {
        if (text == null) return null;
        Matcher matcher = LAUNCH_TOKEN.matcher(text.trim());
        return matcher.find() ? matcher.group(1) : null;
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

    /**
     * 把 launch token 换成会话 cookie 所需的请求 URL。
     *
     * 交换是一次 303 + Set-Cookie：只要拿到响应头里的 cookie，不必真的跳转。
     */
    public static String exchangeUrl(String baseUrl, String launchToken) {
        return baseUrl + "/?token=" + launchToken;
    }

    /**
     * 从 HTTP 响应头里取出 dsh 的 Set-Cookie（`name=value`）。
     *
     * @param setCookieHeaders 响应里所有 Set-Cookie 头的原值
     * @return 可存储的 cookie 对；没有 dsh 的 cookie 时返回 null
     */
    public static String cookieFromSetCookie(java.util.List<String> setCookieHeaders) {
        if (setCookieHeaders == null) return null;
        for (String header : setCookieHeaders) {
            String parsed = parseCookie(header);
            if (parsed != null) return parsed;
        }
        return null;
    }
}
