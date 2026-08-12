package com.local.kimiapp;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.DownloadManager;
import android.content.Intent;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.graphics.Color;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.provider.Settings;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.PermissionRequest;
import android.webkit.JavascriptInterface;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;
import android.widget.Toast;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class MainActivity extends Activity {
    public static volatile boolean isVisible = false;
    public static final String EXTRA_SHOW_CONFIG = "show_connection_config";
    public static final String EXTRA_UPDATE_URL = "update_download_url";
    public static final String EXTRA_UPDATE_NAME = "update_download_name";
    private static final String PREFS = "kimi_connection";
    private static final int FILE_CHOOSER = 42;
    private static final String NOTIFICATION_CHANNEL = "kimi_tasks";
    private WebView webView;
    private ProgressBar progress;
    private ValueCallback<Uri[]> fileCallback;
    private PermissionRequest pendingPermission;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        setupNotifications();
        startKeepAliveService();
        buildUi();
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        if (prefs.contains("ip")) {
            loadConfiguredUrl();
            if (getIntent().getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showConfig(false);
        } else showConfig(true);
        handleUpdateIntent(getIntent());
    }

    private void startKeepAliveService() {
        Intent service = new Intent(this, KeepAliveService.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service);
        else startService(service);
    }

    private void buildUi() {
        FrameLayout root = new FrameLayout(this);
        webView = new WebView(this);
        progress = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progress.setMax(100);
        root.addView(webView, new FrameLayout.LayoutParams(-1, -1));
        root.addView(progress, new FrameLayout.LayoutParams(-1, dp(3), Gravity.TOP));
        setContentView(root);

        webView.getSettings().setJavaScriptEnabled(true);
        webView.getSettings().setDomStorageEnabled(true);
        webView.getSettings().setDatabaseEnabled(true);
        webView.getSettings().setMediaPlaybackRequiresUserGesture(false);
        webView.getSettings().setAllowFileAccess(true);
        webView.getSettings().setBuiltInZoomControls(false);
        webView.addJavascriptInterface(new ThemeBridge(), "AndroidTheme");
        webView.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri uri = request.getUrl();
                if ("http".equals(uri.getScheme()) || "https".equals(uri.getScheme())) return false;
                try { startActivity(new Intent(Intent.ACTION_VIEW, uri)); } catch (Exception ignored) {}
                return true;
            }

            @Override public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                installThemeSync();
            }
        });
        webView.setWebChromeClient(new WebChromeClient() {
            @Override public void onProgressChanged(WebView view, int value) {
                progress.setProgress(value);
                progress.setVisibility(value == 100 ? View.GONE : View.VISIBLE);
            }
            @Override public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (fileCallback != null) fileCallback.onReceiveValue(null);
                fileCallback = callback;
                try { startActivityForResult(params.createIntent(), FILE_CHOOSER); }
                catch (Exception e) { fileCallback = null; Toast.makeText(MainActivity.this, "无法打开文件选择器", Toast.LENGTH_SHORT).show(); }
                return true;
            }
            @Override public void onPermissionRequest(PermissionRequest request) {
                runOnUiThread(() -> requestWebPermission(request));
            }
        });

    }

    @Override protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        if (intent.getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showConfig(false);
        handleUpdateIntent(intent);
    }

    private void handleUpdateIntent(Intent intent) {
        String url = intent.getStringExtra(EXTRA_UPDATE_URL);
        if (url == null || url.isEmpty()) return;
        String name = intent.getStringExtra(EXTRA_UPDATE_NAME);
        if (name == null || !name.endsWith(".apk")) name = "KimiWeb-update.apk";
        DownloadManager.Request request = new DownloadManager.Request(Uri.parse(url))
                .setTitle("正在下载 KimiWeb 更新")
                .setDescription(name)
                .setMimeType("application/vnd.android.package-archive")
                .setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE_NOTIFY_COMPLETED)
                .setDestinationInExternalFilesDir(this, android.os.Environment.DIRECTORY_DOWNLOADS, name);
        long id = getSystemService(DownloadManager.class).enqueue(request);
        getSharedPreferences(UpdateDownloadReceiver.PREFS, MODE_PRIVATE).edit()
                .putLong("download_id", id).putString("download_name", name).apply();
        intent.removeExtra(EXTRA_UPDATE_URL);
        Toast.makeText(this, "更新开始下载，完成后将打开安装页面", Toast.LENGTH_LONG).show();
    }


    private void setupNotifications() {
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            NotificationChannel channel = new NotificationChannel(NOTIFICATION_CHANNEL,
                    "Kimi 任务通知", NotificationManager.IMPORTANCE_DEFAULT);
            channel.setDescription("任务完成、等待回答或审批时通知");
            manager.createNotificationChannel(channel);
        }
        if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 8);
        }
    }

    private void installThemeSync() {
        String script = "(function(){" +
                "if(window.__androidThemeSync)return;window.__androidThemeSync=true;" +
                "function sync(){" +
                "var e=document.body||document.documentElement,c=getComputedStyle(e).backgroundColor;" +
                "if(!c||c==='rgba(0, 0, 0, 0)'||c==='transparent')c=getComputedStyle(document.documentElement).backgroundColor;" +
                "var m=c&&c.match(/[\\d.]+/g),dark=matchMedia('(prefers-color-scheme: dark)').matches;" +
                "if(m&&m.length>=3){var r=+m[0],g=+m[1],b=+m[2];dark=(r*299+g*587+b*114)<128000;c='rgb('+r+','+g+','+b+')';}" +
                "AndroidTheme.update(c||'',dark);}" +
                "new MutationObserver(sync).observe(document.documentElement,{attributes:true,subtree:true,attributeFilter:['class','style','data-theme']});" +
                "matchMedia('(prefers-color-scheme: dark)').addEventListener('change',sync);sync();" +
                "})();";
        webView.evaluateJavascript(script, null);
    }

    private class ThemeBridge {
        @JavascriptInterface public void update(String color, boolean dark) {
            runOnUiThread(() -> {
                int background = dark ? Color.rgb(18, 18, 18) : Color.WHITE;
                try {
                    if (color != null && color.startsWith("rgb(")) {
                        String[] values = color.substring(4, color.length() - 1).split(",");
                        background = Color.rgb(Integer.parseInt(values[0].trim()),
                                Integer.parseInt(values[1].trim()), Integer.parseInt(values[2].trim()));
                    }
                } catch (Exception ignored) {}
                // 状态栏和导航栏背景色跟随网页主题
                getWindow().setStatusBarColor(background);
                getWindow().setNavigationBarColor(background);
                // 图标颜色：暗底用白色图标，亮底用黑色图标
                int flags = getWindow().getDecorView().getSystemUiVisibility();
                if (dark) {
                    flags &= ~View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        flags &= ~View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                    }
                } else {
                    flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                        flags |= View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                    }
                }
                getWindow().getDecorView().setSystemUiVisibility(flags);
            });
        }
    }

    private void requestWebPermission(PermissionRequest request) {
        pendingPermission = request;
        if (android.os.Build.VERSION.SDK_INT >= 23 &&
            (checkSelfPermission(Manifest.permission.CAMERA) != PackageManager.PERMISSION_GRANTED ||
             checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED)) {
            requestPermissions(new String[]{Manifest.permission.CAMERA, Manifest.permission.RECORD_AUDIO}, 7);
        } else request.grant(request.getResources());
    }

    @Override public void onRequestPermissionsResult(int code, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(code, permissions, results);
        if (code == 7 && pendingPermission != null) {
            boolean granted = true;
            for (int result : results) granted &= result == PackageManager.PERMISSION_GRANTED;
            if (granted) pendingPermission.grant(pendingPermission.getResources()); else pendingPermission.deny();
            pendingPermission = null;
        }
    }

    private void showConfig(boolean required) {
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(dp(22), dp(6), dp(22), 0);
        TextView hint = new TextView(this);
        hint.setText("可直接粘贴 Kimi 输出的完整连接地址或整段启动信息，下面内容会自动识别。");
        hint.setTextColor(Color.DKGRAY);
        hint.setPadding(0, 0, 0, dp(12));
        EditText pasted = new EditText(this);
        pasted.setHint("例如 http://100.95.189.73:58627/#token=…");
        pasted.setSingleLine(false);
        pasted.setMinLines(2);
        EditText ip = new EditText(this);
        ip.setHint("IP 地址或主机名");
        ip.setSingleLine(true);
        ip.setText(prefs.getString("ip", "100.95.189.73"));
        EditText port = new EditText(this);
        port.setHint("端口");
        port.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);
        port.setSingleLine(true);
        port.setText(String.valueOf(prefs.getInt("port", 58627)));
        EditText token = new EditText(this);
        token.setHint("Token（可留空）");
        token.setSingleLine(true);
        token.setInputType(android.text.InputType.TYPE_CLASS_TEXT |
                android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD);
        token.setText(prefs.getString("token", ""));
        box.addView(hint); box.addView(pasted); box.addView(ip); box.addView(port); box.addView(token);

        AlertDialog dialog = new AlertDialog.Builder(this)
            .setTitle("连接 Kimi")
            .setView(box)
            .setCancelable(!required)
            .setNegativeButton(required ? null : "取消", null)
            .setPositiveButton("保存并连接", null)
            .create();
        dialog.setOnShowListener(x -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
            if (!pasted.getText().toString().trim().isEmpty()) {
                ParsedConnection parsed = parseConnection(pasted.getText().toString());
                if (parsed == null) {
                    Toast.makeText(this, "没有识别到有效的连接地址", Toast.LENGTH_SHORT).show(); return;
                }
                ip.setText(parsed.host);
                port.setText(String.valueOf(parsed.port));
                if (!parsed.token.isEmpty()) token.setText(parsed.token);
            }
            String host = ip.getText().toString().trim();
            int p;
            try { p = Integer.parseInt(port.getText().toString()); } catch (Exception e) { p = -1; }
            if (host.isEmpty() || host.matches(".*[\\s/:?#].*") || p < 1 || p > 65535) {
                Toast.makeText(this, "请检查 IP 和端口", Toast.LENGTH_SHORT).show(); return;
            }
            prefs.edit().putString("ip", host).putInt("port", p)
                    .putString("token", token.getText().toString().trim()).apply();
            dialog.dismiss();
            stopService(new Intent(this, KeepAliveService.class));
            startKeepAliveService();
            loadConfiguredUrl();
        }));
        dialog.show();
    }

    private static class ParsedConnection {
        final String host, token;
        final int port;
        ParsedConnection(String host, int port, String token) {
            this.host = host; this.port = port; this.token = token;
        }
    }

    private ParsedConnection parseConnection(String text) {
        try {
            Matcher urlMatch = Pattern.compile("https?://[^\\s]+", Pattern.CASE_INSENSITIVE).matcher(text);
            String raw = urlMatch.find() ? urlMatch.group() : text.trim();
            raw = raw.replaceAll("[),;，。]+$", "");
            Uri uri = Uri.parse(raw);
            String host = uri.getHost();
            int port = uri.getPort();
            if (host == null || host.isEmpty()) return null;
            if (port < 1) port = "https".equalsIgnoreCase(uri.getScheme()) ? 443 : 80;
            String foundToken = "";
            String fragment = uri.getFragment();
            if (fragment != null) foundToken = Uri.parse("http://local/?" + fragment).getQueryParameter("token");
            if (foundToken == null || foundToken.isEmpty()) foundToken = uri.getQueryParameter("token");
            if (foundToken == null || foundToken.isEmpty()) {
                Matcher tokenMatch = Pattern.compile("(?i)(?:auth[-_ ]?token|token)\\s*[:=]\\s*([A-Za-z0-9._~-]+)").matcher(text);
                foundToken = tokenMatch.find() ? tokenMatch.group(1) : "";
            }
            return new ParsedConnection(host, port, foundToken == null ? "" : foundToken);
        } catch (Exception ignored) { return null; }
    }

    private void loadConfiguredUrl() {
        SharedPreferences prefs = getSharedPreferences(PREFS, MODE_PRIVATE);
        String url = "http://" + prefs.getString("ip", "100.95.189.73") + ":" + prefs.getInt("port", 58627) + "/";
        String token = prefs.getString("token", "");
        if (token != null && !token.isEmpty()) url += "#token=" + Uri.encode(token);
        webView.loadUrl(url);
    }

    @Override public void onBackPressed() {
        if (webView.canGoBack()) webView.goBack(); else super.onBackPressed();
    }

    @Override protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        if (requestCode == FILE_CHOOSER && fileCallback != null) {
            fileCallback.onReceiveValue(WebChromeClient.FileChooserParams.parseResult(resultCode, data));
            fileCallback = null;
        }
    }

    @Override protected void onDestroy() {
        if (webView != null) { webView.stopLoading(); webView.destroy(); }
        super.onDestroy();
    }

    @Override protected void onStart() {
        super.onStart();
        isVisible = true;
    }

    @Override protected void onStop() {
        isVisible = false;
        super.onStop();
    }

    private int dp(int value) { return Math.round(value * getResources().getDisplayMetrics().density); }
}
