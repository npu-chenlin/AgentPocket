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
import android.content.res.Configuration;
import android.graphics.Color;
import android.graphics.Rect;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.LayerDrawable;
import android.graphics.drawable.RippleDrawable;
import android.content.res.ColorStateList;
import android.net.Uri;
import android.os.Bundle;
import android.os.Build;
import android.provider.Settings;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewConfiguration;
import android.view.animation.OvershootInterpolator;
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
import android.widget.ImageButton;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import android.widget.Button;

import androidx.activity.OnBackPressedCallback;
import androidx.core.view.WindowInsetsControllerCompat;

import com.petterp.floatingx.assist.FxAdsorbDirection;
import com.petterp.floatingx.assist.FxGravity;
import com.petterp.floatingx.assist.helper.FxScopeHelper;
import com.petterp.floatingx.listener.IFxTouchListener;
import com.petterp.floatingx.listener.control.IFxScopeControl;
import com.petterp.floatingx.view.IFxInternalHelper;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

public class MainActivity extends Activity {
    public static volatile boolean isVisible = false;
    public static final String EXTRA_SHOW_CONFIG = "show_connection_config";
    public static final String EXTRA_SERVER_ID = "open_server_id";
    public static final String EXTRA_SESSION_ID = "open_session_id";
    public static final String EXTRA_UPDATE_URL = "update_download_url";
    public static final String EXTRA_UPDATE_NAME = "update_download_name";
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
        activateServerFromIntent(getIntent());
        if (ServerStore.active(this) != null) {
            loadConfiguredUrl(getIntent().getStringExtra(EXTRA_SESSION_ID));
            if (getIntent().getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showServerList();
        } else showConfig(true, null);
        handleUpdateIntent(getIntent());
    }

    @Override public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        if (webView != null) webView.invalidate();
    }

    private void startKeepAliveService() {
        Intent service = new Intent(this, KeepAliveService.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) startForegroundService(service);
        else startService(service);
    }

    private void styleServerHandle(View button) {
        // 主体：带方向光的深蓝渐变
        GradientDrawable body = new GradientDrawable(GradientDrawable.Orientation.TL_BR,
                new int[]{Color.rgb(96, 188, 255), Color.rgb(34, 130, 244), Color.rgb(12, 56, 160)});
        body.setShape(GradientDrawable.OVAL);
        body.setStroke(dp(1), Color.argb(95, 215, 240, 255));
        // 底部内阴影，增加立体感
        GradientDrawable shade = new GradientDrawable(GradientDrawable.Orientation.TOP_BOTTOM,
                new int[]{Color.argb(0, 3, 16, 60), Color.argb(0, 3, 16, 60), Color.argb(72, 3, 16, 64)});
        shade.setShape(GradientDrawable.OVAL);
        // 顶部玻璃高光
        GradientDrawable gloss = new GradientDrawable(GradientDrawable.Orientation.TOP_BOTTOM,
                new int[]{Color.argb(120, 255, 255, 255), Color.argb(28, 255, 255, 255), Color.argb(0, 255, 255, 255)});
        gloss.setShape(GradientDrawable.OVAL);
        LayerDrawable layers = new LayerDrawable(new Drawable[]{body, shade, gloss});
        layers.setLayerInset(2, dp(7), dp(3), dp(7), dp(19));
        button.setBackground(new RippleDrawable(
                ColorStateList.valueOf(Color.argb(70, 255, 255, 255)), layers, null));
    }

    private void installServerHandle(FrameLayout root) {
        FrameLayout handle = new FrameLayout(this);
        // FloatingX 直接使用该参数，缺省会按 WRAP_CONTENT 收缩
        handle.setLayoutParams(new FrameLayout.LayoutParams(dp(56), dp(56)));
        handle.setContentDescription("切换服务器");
        handle.setElevation(dp(8));
        styleServerHandle(handle);
        LinearLayout eyes = new LinearLayout(this);
        eyes.setGravity(Gravity.CENTER);
        final View[] eyeViews = new View[2];
        for (int i = 0; i < 2; i++) {
            View eye = new View(this);
            GradientDrawable eyeShape = new GradientDrawable();
            eyeShape.setColor(Color.WHITE);
            eyeShape.setCornerRadius(dp(4));
            eye.setBackground(eyeShape);
            eyeViews[i] = eye;
            LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(dp(7), dp(10));
            if (i == 1) params.setMargins(dp(8), 0, 0, 0);
            eyes.addView(eye, params);
        }
        handle.addView(eyes, new FrameLayout.LayoutParams(-1, -1));
        eyes.setVisibility(View.INVISIBLE);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            handle.addOnLayoutChangeListener((view, left, top, right, bottom,
                    oldLeft, oldTop, oldRight, oldBottom) -> view.setSystemGestureExclusionRects(
                    Collections.singletonList(new Rect(0, 0, view.getWidth(), view.getHeight()))));
        }

        final IFxScopeControl[] control = new IFxScopeControl[1];
        final boolean[] awake = new boolean[]{false};
        final boolean[] dragged = new boolean[]{false};
        final float[] touchStart = new float[2];
        final long[] touchStartedAt = new long[1];
        final int touchSlop = ViewConfiguration.get(this).getScaledTouchSlop();
        final Runnable[] sleep = new Runnable[1];
        final Runnable[] blink = new Runnable[1];
        final Runnable[] handleClick = new Runnable[1];
        // 睡眠态：略缩小、降透明度，半藏于屏幕边缘
        handle.setAlpha(0.72f);
        handle.setScaleX(0.94f);
        handle.setScaleY(0.94f);

        blink[0] = () -> {
            if (!awake[0]) return;
            for (View eye : eyeViews) {
                eye.setPivotY(eye.getHeight() / 2f);
                eye.animate().cancel();
                eye.animate().scaleY(0.12f).setDuration(80).withEndAction(() ->
                        eye.animate().scaleY(1f).setDuration(120).start()).start();
            }
            handle.postDelayed(blink[0], 2400 + (long) (Math.random() * 1800));
        };

        sleep[0] = () -> {
            if (control[0] == null || !awake[0]) return;
            awake[0] = false;
            handle.removeCallbacks(blink[0]);
            eyes.animate().cancel();
            eyes.animate().alpha(0f).setDuration(140).withEndAction(() -> {
                eyes.setVisibility(View.INVISIBLE);
                eyes.setAlpha(1f);
            }).start();
            handle.animate().cancel();
            handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f).setDuration(180).start();
            boolean left = control[0].getX() + dp(28) < root.getWidth() / 2f;
            control[0].move(left ? -dp(28) : root.getWidth() - dp(28), control[0].getY(), false);
        };

        handleClick[0] = () -> {
            handle.removeCallbacks(sleep[0]);
            if (!awake[0]) {
                awake[0] = true;
                eyes.setVisibility(View.VISIBLE);
                eyes.setAlpha(0f);
                eyes.animate().cancel();
                eyes.animate().alpha(1f).setDuration(160).start();
                handle.animate().cancel();
                handle.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(240)
                        .setInterpolator(new OvershootInterpolator(1.6f)).start();
                handle.postDelayed(blink[0], 1600);
                boolean left = control[0].getX() + dp(28) < root.getWidth() / 2f;
                control[0].move(left ? dp(8) : root.getWidth() - dp(64), control[0].getY(), false);
                handle.postDelayed(sleep[0], 3500);
            } else {
                showServerList();
                handle.postDelayed(sleep[0], 3500);
            }
        };

        FxScopeHelper helper = FxScopeHelper.builder()
                .setLayoutView(handle)
                .setManagerParams(new FrameLayout.LayoutParams(dp(56), dp(56)))
                .setGravity(FxGravity.RIGHT_OR_CENTER)
                .setEnableEdgeAdsorption(true)
                .setEdgeAdsorbDirection(FxAdsorbDirection.LEFT_OR_RIGHT)
                .setEnableScrollOutsideScreen(true)
                .setHalfHidePercent(0.5f)
                .setEnableAnimation(true)
                .setTouchListener(new IFxTouchListener() {
                    @Override public void onDown() { handle.removeCallbacks(sleep[0]); handle.removeCallbacks(blink[0]); }
                    @Override public void onDragIng(MotionEvent event, float x, float y) { }
                    @Override public boolean onTouch(MotionEvent event, IFxInternalHelper helper) {
                        if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                            touchStart[0] = event.getRawX();
                            touchStart[1] = event.getRawY();
                            touchStartedAt[0] = System.currentTimeMillis();
                            dragged[0] = false;
                        } else if (event.getActionMasked() == MotionEvent.ACTION_MOVE) {
                            dragged[0] = Math.hypot(event.getRawX() - touchStart[0],
                                    event.getRawY() - touchStart[1]) >= touchSlop;
                        } else if (event.getActionMasked() == MotionEvent.ACTION_UP) {
                            long duration = System.currentTimeMillis() - touchStartedAt[0];
                            if (!dragged[0] && duration <= 600) handleClick[0].run();
                            else if (dragged[0]) {
                                awake[0] = false;
                                eyes.setVisibility(View.INVISIBLE);
                                handle.animate().cancel();
                                handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f)
                                        .setDuration(180).start();
                            }
                        }
                        return false;
                    }
                    @Override public boolean onInterceptTouchEvent(MotionEvent event, IFxInternalHelper helper) { return false; }
                    @Override public void onUp() {
                        if (!dragged[0] && awake[0]) {
                            handle.postDelayed(sleep[0], 3500);
                            handle.postDelayed(blink[0], 1600);
                        }
                    }
                }).build();
        control[0] = helper.toControl(root);
        control[0].show();
    }

    private void buildUi() {
        FrameLayout root = new FrameLayout(this);
        webView = new WebView(this);
        progress = new ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal);
        progress.setMax(100);
        root.addView(webView, new FrameLayout.LayoutParams(-1, -1));
        root.addView(progress, new FrameLayout.LayoutParams(-1, dp(3), Gravity.TOP));
        setContentView(root);
        installServerHandle(root);

        webView.getSettings().setJavaScriptEnabled(true);
        webView.getSettings().setDomStorageEnabled(true);
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
        if (activateServerFromIntent(intent)) {
            loadConfiguredUrl(intent.getStringExtra(EXTRA_SESSION_ID));
        }
        if (intent.getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showServerList();
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
        @SuppressWarnings("deprecation")
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
                View decor = getWindow().getDecorView();
                WindowInsetsControllerCompat controller = new WindowInsetsControllerCompat(getWindow(), decor);
                controller.setAppearanceLightStatusBars(!dark);
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                    controller.setAppearanceLightNavigationBars(!dark);
                }
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

    private void showServerList() {
        List<ServerStore.Server> servers = ServerStore.load(this);
        String active = ServerStore.activeId(this);
        SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
        Map<String, TextView> statusViews = new HashMap<>();
        LinearLayout list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        list.setPadding(dp(20), dp(4), dp(20), dp(8));
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle("服务器")
                .setView(list).setNegativeButton("关闭", null).create();
        for (ServerStore.Server server : servers) {
            boolean selected = server.id.equals(active);
            LinearLayout card = new LinearLayout(this);
            card.setGravity(Gravity.CENTER_VERTICAL);
            card.setPadding(dp(15), dp(12), dp(14), dp(12));
            GradientDrawable cardBackground = new GradientDrawable();
            cardBackground.setColor(selected ? Color.rgb(238, 245, 255) : Color.rgb(248, 249, 252));
            cardBackground.setCornerRadius(dp(15));
            cardBackground.setStroke(dp(1), selected ? Color.rgb(143, 187, 248) : Color.rgb(226, 229, 236));
            card.setBackground(new RippleDrawable(ColorStateList.valueOf(Color.argb(28, 25, 112, 238)), cardBackground, null));

            TextView status = new TextView(this);
            status.setText("●");
            status.setTextSize(13);
            updateServerHealthDot(status, health, server.id);
            statusViews.put(server.id, status);
            card.addView(status, new LinearLayout.LayoutParams(dp(24), -2));

            LinearLayout text = new LinearLayout(this);
            text.setOrientation(LinearLayout.VERTICAL);
            TextView title = new TextView(this);
            title.setText(server.name);
            title.setTextSize(16);
            title.setTextColor(Color.rgb(28, 34, 45));
            TextView address = new TextView(this);
            address.setText(server.host + ":" + server.port);
            address.setTextSize(13);
            address.setTextColor(Color.rgb(112, 120, 135));
            address.setPadding(0, dp(3), 0, 0);
            text.addView(title); text.addView(address);
            card.addView(text, new LinearLayout.LayoutParams(0, -2, 1));

            if (selected) card.addView(serverBadge("当前", Color.rgb(25, 112, 238), Color.rgb(220, 235, 255)),
                    new LinearLayout.LayoutParams(dp(44), dp(28)));
            ImageButton edit = serverIcon(R.drawable.ic_edit, "编辑 " + server.name, false);
            ImageButton delete = serverIcon(R.drawable.ic_delete, "删除 " + server.name, true);
            LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(dp(38), dp(42));
            iconParams.setMargins(dp(3), 0, 0, 0);
            card.addView(edit, iconParams);
            card.addView(delete, iconParams);
            LinearLayout.LayoutParams cardParams = new LinearLayout.LayoutParams(-1, dp(72));
            cardParams.setMargins(0, 0, 0, dp(10));
            list.addView(card, cardParams);
            card.setOnClickListener(v -> { dialog.dismiss(); switchServer(server.id); });
            edit.setOnClickListener(v -> { dialog.dismiss(); showConfig(false, server, true); });
            delete.setOnClickListener(v -> { dialog.dismiss(); deleteServer(server); });
        }
        LinearLayout actions = new LinearLayout(this);
        actions.setOrientation(LinearLayout.HORIZONTAL);

        Button refresh = new Button(this);
        refresh.setText("刷新页面");
        refresh.setTextSize(14);
        refresh.setTextColor(Color.rgb(78, 88, 105));
        refresh.setAllCaps(false);
        refresh.setElevation(0);
        refresh.setStateListAnimator(null);
        GradientDrawable refreshBackground = new GradientDrawable();
        refreshBackground.setColor(Color.rgb(245, 247, 250));
        refreshBackground.setCornerRadius(dp(14));
        refreshBackground.setStroke(dp(1), Color.rgb(217, 222, 231));
        refresh.setBackground(new RippleDrawable(
                ColorStateList.valueOf(Color.argb(30, 78, 88, 105)), refreshBackground, null));

        Button add = new Button(this);
        add.setText("＋  添加服务器");
        add.setTextSize(14);
        add.setTextColor(Color.rgb(25, 112, 238));
        add.setAllCaps(false);
        GradientDrawable addBackground = new GradientDrawable();
        addBackground.setColor(Color.TRANSPARENT);
        addBackground.setCornerRadius(dp(14));
        addBackground.setStroke(dp(1), Color.rgb(180, 207, 244));
        add.setBackground(new RippleDrawable(ColorStateList.valueOf(Color.argb(30, 25, 112, 238)), addBackground, null));
        LinearLayout.LayoutParams refreshParams = new LinearLayout.LayoutParams(0, dp(50), 1);
        refreshParams.setMargins(0, 0, dp(5), 0);
        actions.addView(refresh, refreshParams);
        LinearLayout.LayoutParams addParams = new LinearLayout.LayoutParams(0, dp(50), 1);
        addParams.setMargins(dp(5), 0, 0, 0);
        actions.addView(add, addParams);
        list.addView(actions, new LinearLayout.LayoutParams(-1, dp(50)));
        refresh.setOnClickListener(v -> {
            dialog.dismiss();
            if (webView != null) webView.reload();
        });
        add.setOnClickListener(v -> { dialog.dismiss(); showConfig(false, null, true); });
        SharedPreferences.OnSharedPreferenceChangeListener healthListener = (preferences, key) -> {
            String serverId = null;
            if (key != null && key.startsWith("online_")) serverId = key.substring("online_".length());
            else if (key != null && key.startsWith("checked_")) serverId = key.substring("checked_".length());
            if (serverId == null || !statusViews.containsKey(serverId)) return;
            String changedServerId = serverId;
            runOnUiThread(() -> updateServerHealthDot(
                    statusViews.get(changedServerId), preferences, changedServerId));
        };
        dialog.setOnDismissListener(ignored ->
                health.unregisterOnSharedPreferenceChangeListener(healthListener));
        showModernDialog(dialog, () -> {
            health.registerOnSharedPreferenceChangeListener(healthListener);
            for (Map.Entry<String, TextView> entry : statusViews.entrySet()) {
                updateServerHealthDot(entry.getValue(), health, entry.getKey());
            }
        });
    }

    private void updateServerHealthDot(TextView status, SharedPreferences health, String serverId) {
        if (status == null) return;
        boolean known = health.contains("checked_" + serverId);
        boolean online = health.getBoolean("online_" + serverId, false);
        status.setTextColor(!known ? Color.rgb(174, 180, 191)
                : online ? Color.rgb(34, 184, 109) : Color.rgb(235, 76, 76));
    }

    private TextView serverBadge(String text, int textColor, int backgroundColor) {
        TextView badge = new TextView(this);
        badge.setText(text);
        badge.setTextSize(12);
        badge.setTextColor(textColor);
        badge.setGravity(Gravity.CENTER);
        GradientDrawable background = new GradientDrawable();
        background.setColor(backgroundColor);
        background.setCornerRadius(dp(10));
        badge.setBackground(background);
        return badge;
    }

    private ImageButton serverIcon(int icon, String description, boolean destructive) {
        ImageButton button = new ImageButton(this);
        button.setImageResource(icon);
        button.setScaleType(android.widget.ImageView.ScaleType.CENTER);
        button.setPadding(dp(9), dp(9), dp(9), dp(9));
        button.setContentDescription(description);
        GradientDrawable background = new GradientDrawable();
        background.setColor(destructive ? Color.rgb(255, 241, 241) : Color.rgb(241, 244, 249));
        background.setStroke(dp(1), destructive ? Color.rgb(247, 199, 199) : Color.rgb(218, 224, 234));
        background.setCornerRadius(dp(11));
        button.setBackground(new RippleDrawable(
                ColorStateList.valueOf(destructive ? Color.argb(35, 217, 67, 67)
                        : Color.argb(35, 83, 98, 122)), background, null));
        return button;
    }

    private void switchServer(String id) {
        List<ServerStore.Server> servers = ServerStore.load(this);
        ServerStore.save(this, servers, id);
        loadConfiguredUrl();
    }

    private void deleteServer(ServerStore.Server target) {
        List<ServerStore.Server> servers = new ArrayList<>(ServerStore.load(this));
        if (servers.size() <= 1) { Toast.makeText(this, "至少保留一台服务器", Toast.LENGTH_SHORT).show(); return; }
        new AlertDialog.Builder(this).setTitle("删除 " + target.name + "？")
                .setMessage(target.host + ":" + target.port)
                .setNegativeButton("取消", null).setPositiveButton("删除", (d, w) -> {
                    boolean wasActive = target.id.equals(ServerStore.activeId(this));
                    servers.removeIf(s -> s.id.equals(target.id));
                    String active = ServerStore.activeId(this);
                    if (wasActive) active = servers.get(0).id;
                    ServerStore.save(this, servers, active);
                    restartListener();
                    if (wasActive) loadConfiguredUrl();
                }).show();
    }

    private boolean activateServerFromIntent(Intent intent) {
        String requested = intent.getStringExtra(EXTRA_SERVER_ID);
        if (requested == null || requested.isEmpty()) return false;
        List<ServerStore.Server> servers = ServerStore.load(this);
        for (ServerStore.Server server : servers) {
            if (server.id.equals(requested)) {
                ServerStore.save(this, servers, requested);
                intent.removeExtra(EXTRA_SERVER_ID);
                return true;
            }
        }
        return false;
    }

    private void showConfig(boolean required, ServerStore.Server editing) {
        showConfig(required, editing, false);
    }

    private void showConfig(boolean required, ServerStore.Server editing, boolean returnToList) {
        LinearLayout box = new LinearLayout(this);
        box.setOrientation(LinearLayout.VERTICAL);
        box.setPadding(dp(24), dp(8), dp(24), dp(8));
        TextView hint = new TextView(this);
        hint.setText("可直接粘贴 Kimi 输出的完整连接地址或整段启动信息，下面内容会自动识别。");
        hint.setTextSize(14);
        hint.setTextColor(Color.rgb(96, 103, 117));
        hint.setLineSpacing(dp(3), 1f);
        hint.setPadding(0, 0, 0, dp(18));
        TextView quickLabel = fieldLabel("快速导入");
        EditText pasted = modernField("粘贴完整地址或启动信息", true);
        pasted.setSingleLine(false);
        pasted.setMinLines(2);
        Button recognize = new Button(this);
        recognize.setText("识别并填充");
        recognize.setTextSize(14);
        recognize.setTextColor(Color.WHITE);
        recognize.setAllCaps(false);
        GradientDrawable recognizeBackground = new GradientDrawable();
        recognizeBackground.setColor(Color.rgb(25, 112, 238));
        recognizeBackground.setCornerRadius(dp(12));
        recognize.setBackground(new RippleDrawable(
                ColorStateList.valueOf(Color.argb(65, 255, 255, 255)), recognizeBackground, null));
        LinearLayout.LayoutParams recognizeParams = new LinearLayout.LayoutParams(-1, dp(46));
        recognizeParams.setMargins(0, 0, 0, dp(2));
        recognize.setLayoutParams(recognizeParams);
        TextView detailLabel = fieldLabel("连接详情");
        detailLabel.setPadding(0, dp(18), 0, dp(7));
        EditText name = modernField("服务器名称（例如：工作站）", false);
        name.setText(editing == null ? "" : editing.name);
        EditText ip = modernField("IP 地址或主机名", false);
        ip.setText(editing == null ? "" : editing.host);
        EditText port = modernField("端口", false);
        port.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);
        port.setText(String.valueOf(editing == null ? 58627 : editing.port));
        EditText token = modernField("Token（可留空）", false);
        token.setInputType(android.text.InputType.TYPE_CLASS_TEXT |
                android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD);
        token.setText(editing == null ? "" : editing.token);
        recognize.setOnClickListener(v -> {
            ParsedConnection parsed = parseConnection(pasted.getText().toString());
            if (parsed == null) {
                Toast.makeText(this, "没有识别到有效的连接地址", Toast.LENGTH_SHORT).show();
                return;
            }
            ip.setText(parsed.host);
            port.setText(String.valueOf(parsed.port));
            token.setText(parsed.token);
            if (name.getText().toString().trim().isEmpty()) name.setText(parsed.host + ":" + parsed.port);
            Toast.makeText(this, "已识别并填充连接信息", Toast.LENGTH_SHORT).show();
        });
        box.addView(hint); box.addView(quickLabel); box.addView(pasted); box.addView(recognize); box.addView(detailLabel);
        box.addView(name); box.addView(ip); box.addView(port); box.addView(token);
        ScrollView scroll = new ScrollView(this);
        scroll.setFillViewport(true);
        scroll.addView(box);

        AlertDialog dialog = new AlertDialog.Builder(this)
            .setTitle(editing == null ? "添加 Kimi 服务器" : "编辑服务器")
            .setView(scroll)
            .setCancelable(!required)
            .setNegativeButton(required ? null : "取消", null)
            .setPositiveButton("保存并连接", null)
            .create();
        if (returnToList) {
            dialog.setOnCancelListener(ignored -> showServerList());
            dialog.setOnDismissListener(ignored -> {
                // 按钮“取消”触发 dismiss 而不是 cancel，单独在显示后绑定。
            });
        }
        Runnable bindSave = () -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
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
            String displayName = name.getText().toString().trim();
            int p;
            try { p = Integer.parseInt(port.getText().toString()); } catch (Exception e) { p = -1; }
            if (host.isEmpty() || host.matches(".*[\\s/:?#].*") || p < 1 || p > 65535) {
                Toast.makeText(this, "请检查 IP 和端口", Toast.LENGTH_SHORT).show(); return;
            }
            if (displayName.isEmpty()) displayName = host + ":" + p;
            List<ServerStore.Server> servers = new ArrayList<>(ServerStore.load(this));
            String id = editing == null ? ServerStore.newId() : editing.id;
            ServerStore.Server saved = new ServerStore.Server(id, displayName, host, p,
                    token.getText().toString().trim());
            if (editing == null) servers.add(saved);
            else for (int i = 0; i < servers.size(); i++) if (servers.get(i).id.equals(id)) servers.set(i, saved);
            ServerStore.save(this, servers, id);
            dialog.dismiss();
            restartListener();
            loadConfiguredUrl();
        });
        Runnable bindActions = () -> {
            bindSave.run();
            if (returnToList) {
                Button cancel = dialog.getButton(AlertDialog.BUTTON_NEGATIVE);
                if (cancel != null) cancel.setOnClickListener(v -> {
                    dialog.setOnDismissListener(null);
                    dialog.dismiss();
                    showServerList();
                });
            }
        };
        showModernDialog(dialog, bindActions);
    }

    private TextView fieldLabel(String text) {
        TextView label = new TextView(this);
        label.setText(text);
        label.setTextSize(13);
        label.setTextColor(Color.rgb(57, 65, 82));
        label.setPadding(0, 0, 0, dp(7));
        return label;
    }

    private EditText modernField(String hint, boolean multiline) {
        EditText field = new EditText(this);
        field.setHint(hint);
        field.setTextSize(15);
        field.setTextColor(Color.rgb(30, 35, 45));
        field.setHintTextColor(Color.rgb(145, 151, 164));
        field.setPadding(dp(14), dp(11), dp(14), dp(11));
        field.setSingleLine(!multiline);
        GradientDrawable background = new GradientDrawable();
        background.setColor(Color.rgb(248, 249, 252));
        background.setCornerRadius(dp(12));
        background.setStroke(dp(1), Color.rgb(222, 226, 234));
        field.setBackground(background);
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, multiline ? dp(82) : dp(50));
        params.setMargins(0, 0, 0, dp(10));
        field.setLayoutParams(params);
        return field;
    }

    private void showModernDialog(AlertDialog dialog, Runnable afterShow) {
        dialog.setOnShowListener(ignored -> {
            GradientDrawable panel = new GradientDrawable();
            panel.setColor(Color.WHITE);
            panel.setCornerRadius(dp(22));
            dialog.getWindow().setBackgroundDrawable(panel);
            Button positive = dialog.getButton(AlertDialog.BUTTON_POSITIVE);
            if (positive != null) positive.setTextColor(Color.rgb(25, 112, 238));
            Button negative = dialog.getButton(AlertDialog.BUTTON_NEGATIVE);
            if (negative != null) negative.setTextColor(Color.rgb(94, 101, 116));
            if (afterShow != null) afterShow.run();
        });
        dialog.show();
    }

    private void restartListener() {
        stopService(new Intent(this, KeepAliveService.class));
        startKeepAliveService();
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
        loadConfiguredUrl(null);
    }

    private void loadConfiguredUrl(String sessionId) {
        ServerStore.Server server = ServerStore.active(this);
        if (server == null) return;
        String url = server.baseUrl() + "/";
        if (sessionId != null && !sessionId.isEmpty()) {
            url += "sessions/" + sessionId;
        }
        String token = server.token;
        if (token != null && !token.isEmpty()) {
            url += "#token=" + Uri.encode(token);
        }
        webView.loadUrl(url);
    }

    @SuppressWarnings("deprecation")
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
