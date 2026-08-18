package com.local.kimiapp;

import android.Manifest;
import android.app.Activity;
import android.app.AlertDialog;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.DownloadManager;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import android.content.res.Configuration;
import android.graphics.Bitmap;
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
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.text.TextUtils;
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
import android.widget.ImageView;
import android.widget.ProgressBar;
import android.widget.RadioButton;
import android.widget.RadioGroup;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;
import android.widget.Button;

import androidx.core.view.WindowInsetsControllerCompat;
import androidx.recyclerview.widget.ItemTouchHelper;
import androidx.recyclerview.widget.LinearLayoutManager;
import androidx.recyclerview.widget.RecyclerView;

import com.petterp.floatingx.assist.FxAdsorbDirection;
import com.petterp.floatingx.assist.FxGravity;
import com.petterp.floatingx.assist.helper.FxScopeHelper;
import com.petterp.floatingx.listener.IFxTouchListener;
import com.petterp.floatingx.listener.control.IFxScopeControl;
import com.petterp.floatingx.view.IFxInternalHelper;

import com.local.kimiapp.face.GrokFaceState;
import com.local.kimiapp.face.GrokFaceView;

import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.TimeUnit;

import java.util.regex.Matcher;
import java.util.regex.Pattern;

import okhttp3.Call;
import okhttp3.Callback;
import okhttp3.MediaType;
import okhttp3.OkHttpClient;
import okhttp3.Request;
import okhttp3.RequestBody;
import okhttp3.Response;

import org.json.JSONArray;
import org.json.JSONObject;

public class MainActivity extends Activity {
    public static volatile boolean isVisible = false;
    public static final String EXTRA_SHOW_CONFIG = "show_connection_config";
    public static final String EXTRA_SHOW_SESSIONS = "show_busy_sessions";
    public static final String EXTRA_SERVER_ID = "open_server_id";
    public static final String EXTRA_SESSION_ID = "open_session_id";
    public static final String EXTRA_UPDATE_URL = "update_download_url";
    public static final String EXTRA_UPDATE_NAME = "update_download_name";
    private static final int FILE_CHOOSER = 42;
    private static final String NOTIFICATION_CHANNEL = "kimi_tasks";
    private WebView webView;
    private ProgressBar progress;
    private GrokFaceView faceView;
    private GrokFaceState lastFaceState;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private final Runnable faceFallback = this::applyBaseFaceState;
    private ValueCallback<Uri[]> fileCallback;
    private PermissionRequest pendingPermission;
    /** 悬浮球控制器（配置变化/折叠屏展开时需要重新贴边）。 */
    private IFxScopeControl floatingControl;
    /** 后端类型探测用短超时 client，避免阻塞页面加载。 */
    private final OkHttpClient probeClient = new OkHttpClient.Builder()
            .connectTimeout(5, TimeUnit.SECONDS)
            .readTimeout(5, TimeUnit.SECONDS)
            .build();

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        setupNotifications();
        startKeepAliveService();
        buildUi();
        registerFaceEventListeners();
        activateServerFromIntent(getIntent());
        if (ServerStore.active(this) != null) {
            loadConfiguredUrl(getIntent().getStringExtra(EXTRA_SESSION_ID));
            if (getIntent().getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showServerList();
            if (getIntent().getBooleanExtra(EXTRA_SHOW_SESSIONS, false)) showBusySessions();
        } else showConfig(true, null);
        handleUpdateIntent(getIntent());
    }

    @Override public void onConfigurationChanged(Configuration newConfig) {
        super.onConfigurationChanged(newConfig);
        if (webView != null) webView.invalidate();
        reattachFloatingBall();
    }

    /**
     * 折叠屏展开/旋转导致屏幕宽度变化时，把悬浮球重新贴到新的屏幕边缘。
     * onConfigurationChanged 时布局尚未更新，先记录旧位置，等布局完成后移动。
     */
    private void reattachFloatingBall() {
        if (floatingControl == null) return;
        final IFxScopeControl ball = floatingControl;
        final float oldX = ball.getX();
        final float oldY = ball.getY();
        final int ballWidth = ball.getView().getWidth();
        final int oldWidth = ball.getManagerView().getWidth();
        if (oldWidth <= 0 || ballWidth <= 0) return;
        ball.getManagerView().post(() -> {
            int newWidth = ball.getManagerView().getWidth();
            if (newWidth <= 0 || newWidth == oldWidth) return;
            boolean leftSide = (oldX + ballWidth / 2f) < oldWidth / 2f;
            float offset = leftSide ? oldX : (oldWidth - oldX - ballWidth);
            float newX = leftSide ? offset : newWidth - ballWidth - offset;
            ball.move(newX, oldY, true);
        });
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
        handle.setLayoutParams(new FrameLayout.LayoutParams(dp(56), dp(56)));
        handle.setContentDescription("切换服务器");
        handle.setElevation(dp(8));
        styleServerHandle(handle);
        faceView = new GrokFaceView(this);
        handle.addView(faceView, new FrameLayout.LayoutParams(-1, -1));
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            handle.addOnLayoutChangeListener((view, left, top, right, bottom,
                    oldLeft, oldTop, oldRight, oldBottom) -> view.setSystemGestureExclusionRects(
                    Collections.singletonList(new Rect(0, 0, view.getWidth(), view.getHeight()))));
        }

        final boolean[] awake = new boolean[]{false};
        final boolean[] dragged = new boolean[]{false};
        final float[] touchStart = new float[2];
        final long[] touchStartedAt = new long[1];
        final int touchSlop = ViewConfiguration.get(this).getScaledTouchSlop();
        final Runnable[] sleep = new Runnable[1];
        final Runnable[] handleClick = new Runnable[1];
        // 睡眠态：略缩小、降透明度，半藏于屏幕边缘
        handle.setAlpha(0.72f);
        handle.setScaleX(0.94f);
        handle.setScaleY(0.94f);

        sleep[0] = () -> {
            if (floatingControl == null || !awake[0]) return;
            awake[0] = false;
            applyFaceState(GrokFaceState.SLEEPING);
            handle.animate().cancel();
            handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f).setDuration(180).start();
            boolean left = floatingControl.getX() + dp(28) < root.getWidth() / 2f;
            floatingControl.move(left ? -dp(28) : root.getWidth() - dp(28), floatingControl.getY(), false);
        };

        handleClick[0] = () -> {
            handle.removeCallbacks(sleep[0]);
            if (!awake[0]) {
                awake[0] = true;
                faceView.wakeTurn();
                faceView.blinkNow();
                applyBaseFaceState();
                handle.animate().cancel();
                handle.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(240)
                        .setInterpolator(new OvershootInterpolator(1.6f)).start();
                boolean left = floatingControl.getX() + dp(28) < root.getWidth() / 2f;
                floatingControl.move(left ? dp(8) : root.getWidth() - dp(64), floatingControl.getY(), false);
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
                    @Override public void onDown() { handle.removeCallbacks(sleep[0]); }
                    @Override public void onDragIng(MotionEvent event, float x, float y) { }
                    @Override public boolean onTouch(MotionEvent event, IFxInternalHelper helper) {
                        if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                            touchStart[0] = event.getRawX();
                            touchStart[1] = event.getRawY();
                            touchStartedAt[0] = System.currentTimeMillis();
                            dragged[0] = false;
                        } else if (event.getActionMasked() == MotionEvent.ACTION_MOVE) {
                            boolean wasDragged = dragged[0];
                            dragged[0] = Math.hypot(event.getRawX() - touchStart[0],
                                    event.getRawY() - touchStart[1]) >= touchSlop;
                            if (dragged[0]) {
                                if (!wasDragged) {
                                    mainHandler.removeCallbacks(faceFallback);
                                    applyFaceState(GrokFaceState.DRAGGING);
                                    faceView.setGaze(0f, 0f);
                                }
                                faceView.setGaze(clampUnit((event.getRawX() - touchStart[0]) / dp(70)),
                                        clampUnit((event.getRawY() - touchStart[1]) / dp(70)));
                            }
                        } else if (event.getActionMasked() == MotionEvent.ACTION_UP) {
                            long duration = System.currentTimeMillis() - touchStartedAt[0];
                            if (!dragged[0] && duration <= 600) handleClick[0].run();
                            else if (dragged[0]) {
                                awake[0] = false;
                                faceView.setGaze(0f, 0f);
                                applyFaceState(GrokFaceState.SLEEPING);
                                handle.animate().cancel();
                                handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f)
                                        .setDuration(180).start();
                            }
                        }
                        return false;
                    }
                    @Override public boolean onInterceptTouchEvent(MotionEvent event, IFxInternalHelper helper) { return false; }
                    @Override public void onUp() {
                        if (!dragged[0] && awake[0]) handle.postDelayed(sleep[0], 3500);
                    }
                }).build();
        floatingControl = helper.toControl(root);
        floatingControl.show();
    }

    private float clampUnit(float v) {
        return Math.max(-1f, Math.min(1f, v));
    }

    private void registerFaceEventListeners() {
        SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
        health.registerOnSharedPreferenceChangeListener(facePrefsListener);
        applyBaseFaceState();
    }

    private final SharedPreferences.OnSharedPreferenceChangeListener facePrefsListener =
            (prefs, key) -> {
                if (key == null || faceView == null) return;
                if (key.equals("active_count")) {
                    applyBaseFaceState();
                } else if (key.equals("last_event")) {
                    long ts = prefs.getLong("last_event_ts", 0L);
                    if (System.currentTimeMillis() - ts > 8000L) return;   // 过期事件忽略
                    String ev = prefs.getString("last_event", "");
                    GrokFaceState s = null;
                    if ("complete".equals(ev)) {
                        s = GrokFaceState.CELEBRATE;
                        faceView.spinOnce();
                    } else if ("approval".equals(ev) || "question".equals(ev)) {
                        s = GrokFaceState.THINKING;
                    } else if ("aborted".equals(ev)) {
                        s = GrokFaceState.SAD;
                    }
                    if (s != null) {
                        applyFaceState(s);
                        mainHandler.removeCallbacks(faceFallback);
                        mainHandler.postDelayed(faceFallback, 3000L);      // 3s 后回落
                    }
                }
            };

    private void applyFaceState(GrokFaceState state) {
        if (faceView == null || state == lastFaceState) return;
        lastFaceState = state;
        faceView.setFaceState(state);
    }

    private void applyBaseFaceState() {
        if (faceView == null) return;
        GrokFaceState s;
        if (webView != null && progress != null && progress.getVisibility() == View.VISIBLE) {
            s = GrokFaceState.LOADING;
        } else {
            SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
            int active = health.getInt("active_count", 0);
            if (active > 0) {
                s = GrokFaceState.WORKING;
            } else {
                boolean known = false;
                boolean online = false;
                for (ServerStore.Server server : ServerStore.load(this)) {
                    if (!health.contains("checked_" + server.id)) continue;
                    known = true;
                    if (health.getBoolean("online_" + server.id, false)) online = true;
                }
                s = known && !online ? GrokFaceState.DROWSY : GrokFaceState.IDLE;
            }
        }
        applyFaceState(s);
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

            @Override public void onPageStarted(WebView view, String url, Bitmap favicon) {
                super.onPageStarted(view, url, favicon);
                injectUuidPolyfill(view);
            }

            @Override public void onPageFinished(WebView view, String url) {
                super.onPageFinished(view, url);
                installThemeSync();
            }
        });
        webView.setWebChromeClient(new WebChromeClient() {
            @Override public void onProgressChanged(WebView view, int value) {
                progress.setProgress(value);
                boolean loading = value < 100;
                progress.setVisibility(loading ? View.VISIBLE : View.GONE);
                if (loading) applyFaceState(GrokFaceState.LOADING);
                else applyBaseFaceState();
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
            String sessionId = intent.getStringExtra(EXTRA_SESSION_ID);
            // 页面已在目标服务器/会话上时不再重新加载，避免从通知栏打开每次都白屏刷新
            if (!isOnTargetPage(sessionId)) {
                loadConfiguredUrl(sessionId);
            }
        }
        if (intent.getBooleanExtra(EXTRA_SHOW_CONFIG, false)) showServerList();
        if (intent.getBooleanExtra(EXTRA_SHOW_SESSIONS, false)) {
            intent.removeExtra(EXTRA_SHOW_SESSIONS);
            showBusySessions();
        }
        handleUpdateIntent(intent);
    }

    /** 当前 WebView 页面是否已指向目标服务器（含目标会话，dsh 无会话深链）。 */
    private boolean isOnTargetPage(String sessionId) {
        if (webView == null) return false;
        String current = webView.getUrl();
        if (current == null) return false;
        ServerStore.Server server = ServerStore.active(this);
        if (server == null) return false;
        String expected = server.baseUrl() + "/";
        if (sessionId != null && !sessionId.isEmpty()
                && ServerStore.Server.BACKEND_KIMI.equals(server.backend)) {
            expected += "sessions/" + sessionId;
        }
        return current.equals(expected) || current.startsWith(expected);
    }

    private void handleUpdateIntent(Intent intent) {
        String url = intent.getStringExtra(EXTRA_UPDATE_URL);
        if (url == null || url.isEmpty()) return;
        String name = intent.getStringExtra(EXTRA_UPDATE_NAME);
        if (name == null || !name.endsWith(".apk")) name = "AgentPocket-update.apk";
        DownloadManager.Request request = new DownloadManager.Request(Uri.parse(url))
                .setTitle("正在下载 AgentPocket 更新")
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
                    "AgentPocket 任务通知", NotificationManager.IMPORTANCE_DEFAULT);
            channel.setDescription("任务完成、等待回答或审批时通知");
            manager.createNotificationChannel(channel);
        }
        if (Build.VERSION.SDK_INT >= 33 &&
                checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 8);
        }
    }

    /**
     * 为旧版 WebView 注入 crypto.randomUUID polyfill（Chrome/WebView 92 以下不支持）。
     * DeepSeek Harness 前端会调用 crypto.randomUUID 生成 rpcId，缺失时页面报
     * "crypto.randomUUID is not a function"。
     */
    private void injectUuidPolyfill(WebView view) {
        String script = "(function(){" +
                "if(typeof crypto!=='undefined'&&!crypto.randomUUID){" +
                "crypto.randomUUID=function(){" +
                "var u='xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx';" +
                "return u.replace(/[xy]/g,function(c){" +
                "var r=Math.random()*16|0,v=c==='x'?r:(r&0x3|0x8);return v.toString(16);});};}" +
                "})();";
        view.evaluateJavascript(script, null);
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
        showServerList(false);
    }

    private void showServerList(boolean openSessions) {
        List<ServerStore.Server> servers = ServerStore.load(this);
        String active = ServerStore.activeId(this);
        SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
        LinearLayout list = new LinearLayout(this);
        list.setOrientation(LinearLayout.VERTICAL);
        list.setPadding(dp(20), dp(4), dp(20), dp(8));
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle("服务器")
                .setView(list).setNegativeButton("关闭", null).create();

        LinearLayout tabBar = new LinearLayout(this);
        tabBar.setOrientation(LinearLayout.HORIZONTAL);
        Button tabServers = new Button(this);
        tabServers.setText("服务器");
        Button tabSessions = new Button(this);
        tabSessions.setText("活跃会话");
        for (Button tab : new Button[]{ tabServers, tabSessions }) {
            tab.setTextSize(14);
            tab.setAllCaps(false);
            tab.setElevation(0);
            tab.setStateListAnimator(null);
        }
        tabBar.addView(tabServers, new LinearLayout.LayoutParams(0, dp(46), 1));
        LinearLayout.LayoutParams tabSessionsParams = new LinearLayout.LayoutParams(0, dp(46), 1);
        tabSessionsParams.setMargins(dp(8), 0, 0, 0);
        tabBar.addView(tabSessions, tabSessionsParams);
        LinearLayout.LayoutParams tabBarParams = new LinearLayout.LayoutParams(-1, dp(46));
        tabBarParams.setMargins(0, 0, 0, dp(10));
        list.addView(tabBar, tabBarParams);

        LinearLayout serverPage = new LinearLayout(this);
        serverPage.setOrientation(LinearLayout.VERTICAL);

        RecyclerView recyclerView = new RecyclerView(this);
        recyclerView.setHasFixedSize(true);
        recyclerView.setLayoutManager(new LinearLayoutManager(this));
        List<ServerStore.Server> mutableServers = new ArrayList<>(servers);
        ServerListAdapter adapter = new ServerListAdapter(dialog, mutableServers, active, health);
        recyclerView.setAdapter(adapter);
        serverPage.addView(recyclerView, new LinearLayout.LayoutParams(-1, -2));

        ItemTouchHelper.Callback callback = new ItemTouchHelper.SimpleCallback(
                ItemTouchHelper.UP | ItemTouchHelper.DOWN, 0) {
            @Override
            public boolean onMove(RecyclerView recyclerView,
                                  RecyclerView.ViewHolder viewHolder,
                                  RecyclerView.ViewHolder target) {
                int from = viewHolder.getBindingAdapterPosition();
                int to = target.getBindingAdapterPosition();
                if (from == RecyclerView.NO_POSITION || to == RecyclerView.NO_POSITION) return false;
                adapter.moveItem(from, to);
                ServerStore.save(MainActivity.this, adapter.getServers(), active);
                return true;
            }

            @Override
            public void onSwiped(RecyclerView.ViewHolder viewHolder, int direction) {}
        };
        new ItemTouchHelper(callback).attachToRecyclerView(recyclerView);

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

        Button copy = new Button(this);
        copy.setText("复制配置");
        copy.setTextSize(14);
        copy.setTextColor(Color.rgb(78, 88, 105));
        copy.setAllCaps(false);
        copy.setElevation(0);
        copy.setStateListAnimator(null);
        GradientDrawable copyBackground = new GradientDrawable();
        copyBackground.setColor(Color.rgb(245, 247, 250));
        copyBackground.setCornerRadius(dp(14));
        copyBackground.setStroke(dp(1), Color.rgb(217, 222, 231));
        copy.setBackground(new RippleDrawable(
                ColorStateList.valueOf(Color.argb(30, 78, 88, 105)), copyBackground, null));

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
        LinearLayout.LayoutParams copyParams = new LinearLayout.LayoutParams(0, dp(50), 1);
        copyParams.setMargins(dp(5), 0, dp(5), 0);
        actions.addView(copy, copyParams);
        LinearLayout.LayoutParams addParams = new LinearLayout.LayoutParams(0, dp(50), 1);
        addParams.setMargins(dp(5), 0, 0, 0);
        actions.addView(add, addParams);
        serverPage.addView(actions, new LinearLayout.LayoutParams(-1, dp(50)));
        refresh.setOnClickListener(v -> {
            dialog.dismiss();
            if (webView != null) webView.reload();
        });
        copy.setOnClickListener(v -> { dialog.dismiss(); copyServerConfig(); });
        add.setOnClickListener(v -> { dialog.dismiss(); showConfig(false, null, true); });

        LinearLayout sessionsPage = buildSessionsPage(dialog);

        list.addView(serverPage, new LinearLayout.LayoutParams(-1, -2));
        list.addView(sessionsPage, new LinearLayout.LayoutParams(-1, -2));

        Runnable selectServers = () -> {
            styleTab(tabServers, true);
            styleTab(tabSessions, false);
            serverPage.setVisibility(View.VISIBLE);
            sessionsPage.setVisibility(View.GONE);
        };
        Runnable selectSessions = () -> {
            styleTab(tabServers, false);
            styleTab(tabSessions, true);
            serverPage.setVisibility(View.GONE);
            sessionsPage.setVisibility(View.VISIBLE);
        };
        tabServers.setOnClickListener(v -> selectServers.run());
        tabSessions.setOnClickListener(v -> selectSessions.run());
        if (openSessions) selectSessions.run(); else selectServers.run();

        showModernDialog(dialog, null);
    }

    private void styleTab(Button tab, boolean selected) {
        GradientDrawable background = new GradientDrawable();
        background.setCornerRadius(dp(14));
        if (selected) {
            background.setColor(Color.rgb(238, 245, 255));
            background.setStroke(dp(1), Color.rgb(143, 187, 248));
            tab.setTextColor(Color.rgb(25, 112, 238));
        } else {
            background.setColor(Color.rgb(245, 247, 250));
            background.setStroke(dp(1), Color.rgb(217, 222, 231));
            tab.setTextColor(Color.rgb(78, 88, 105));
        }
        tab.setBackground(new RippleDrawable(
                ColorStateList.valueOf(Color.argb(30, 25, 112, 238)), background, null));
    }

    /** 活跃会话页：与服务器卡片同款样式，点击跳转到对应会话。 */
    private LinearLayout buildSessionsPage(AlertDialog dialog) {
        LinearLayout page = new LinearLayout(this);
        page.setOrientation(LinearLayout.VERTICAL);
        JSONArray sessions;
        try {
            sessions = new JSONArray(getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE)
                    .getString("busy_sessions", "[]"));
        } catch (Exception e) {
            sessions = new JSONArray();
        }
        List<ServerStore.Server> servers = ServerStore.load(this);

        if (sessions.length() == 0) {
            TextView empty = new TextView(this);
            empty.setText("暂无运行中的会话");
            empty.setTextSize(14);
            empty.setTextColor(Color.rgb(112, 120, 135));
            empty.setGravity(Gravity.CENTER);
            empty.setPadding(0, dp(24), 0, dp(24));
            page.addView(empty, new LinearLayout.LayoutParams(-1, -2));
            return page;
        }

        ScrollView scroll = new ScrollView(this);
        LinearLayout cards = new LinearLayout(this);
        cards.setOrientation(LinearLayout.VERTICAL);
        for (int i = 0; i < sessions.length(); i++) {
            JSONObject o = sessions.optJSONObject(i);
            if (o == null) continue;
            String serverId = o.optString("serverId");
            String sessionId = o.optString("sessionId");
            String title = o.optString("title", "");
            String serverName = o.optString("serverName", "");
            ServerStore.Server server = null;
            for (ServerStore.Server s : servers) {
                if (s.id.equals(serverId)) { server = s; break; }
            }

            LinearLayout card = new LinearLayout(this);
            card.setOrientation(LinearLayout.HORIZONTAL);
            card.setGravity(Gravity.CENTER_VERTICAL);
            card.setPadding(dp(9), dp(8), dp(14), dp(8));
            GradientDrawable cardBackground = new GradientDrawable();
            cardBackground.setColor(Color.rgb(248, 249, 252));
            cardBackground.setCornerRadius(dp(15));
            cardBackground.setStroke(dp(1), Color.rgb(226, 229, 236));
            card.setBackground(new RippleDrawable(
                    ColorStateList.valueOf(Color.argb(28, 25, 112, 238)), cardBackground, null));

            ImageView icon = backendIconView(server != null ? server.backend : ServerStore.Server.BACKEND_KIMI);
            LinearLayout.LayoutParams logoParams = new LinearLayout.LayoutParams(dp(26), dp(26));
            logoParams.setMargins(0, 0, dp(8), 0);
            card.addView(icon, logoParams);

            LinearLayout text = new LinearLayout(this);
            text.setOrientation(LinearLayout.VERTICAL);
            if (isMeaningfulSessionTitle(title)) {
                TextView titleView = new TextView(this);
                titleView.setText(title);
                titleView.setTextSize(16);
                titleView.setTextColor(Color.rgb(28, 34, 45));
                titleView.setSingleLine(true);
                titleView.setEllipsize(TextUtils.TruncateAt.END);
                text.addView(titleView);
            }
            TextView sub = new TextView(this);
            String address = server != null ? server.host + ":" + server.port : "";
            String subText = !serverName.isEmpty()
                    ? (address.isEmpty() ? serverName : serverName + " · " + address)
                    : address;
            sub.setText(subText);
            sub.setTextSize(13);
            sub.setTextColor(Color.rgb(112, 120, 135));
            if (text.getChildCount() > 0) sub.setPadding(0, dp(3), 0, 0);
            text.addView(sub);
            card.addView(text, new LinearLayout.LayoutParams(0, -2, 1));

            final String targetServerId = serverId;
            final String targetSessionId = sessionId;
            card.setOnClickListener(v -> { dialog.dismiss(); openSession(targetServerId, targetSessionId); });

            card.setMinimumHeight(dp(56));
            LinearLayout.LayoutParams cardParams = new LinearLayout.LayoutParams(-1, -2);
            cardParams.setMargins(0, 0, 0, dp(10));
            cards.addView(card, cardParams);
        }
        scroll.addView(cards, new ViewGroup.LayoutParams(-1, -2));
        page.addView(scroll, new LinearLayout.LayoutParams(-1, -2));
        return page;
    }

    private class ServerListAdapter extends RecyclerView.Adapter<ServerListAdapter.ViewHolder> {
        private final AlertDialog dialog;
        private final List<ServerStore.Server> servers;
        private final String activeId;
        private final SharedPreferences health;

        ServerListAdapter(AlertDialog dialog, List<ServerStore.Server> servers,
                          String activeId, SharedPreferences health) {
            this.dialog = dialog;
            this.servers = servers;
            this.activeId = activeId;
            this.health = health;
        }

        @Override public int getItemCount() { return servers.size(); }

        List<ServerStore.Server> getServers() { return servers; }

        void moveItem(int from, int to) {
            if (from == to) return;
            ServerStore.Server item = servers.remove(from);
            servers.add(to, item);
            notifyItemMoved(from, to);
        }

        @Override public ViewHolder onCreateViewHolder(ViewGroup parent, int viewType) {
            Context context = parent.getContext();
            LinearLayout card = new LinearLayout(context);
            card.setGravity(Gravity.CENTER_VERTICAL);
            card.setPadding(dp(9), dp(12), dp(14), dp(12));
            RecyclerView.LayoutParams cardParams = new RecyclerView.LayoutParams(-1, dp(72));
            cardParams.setMargins(0, 0, 0, dp(10));
            card.setLayoutParams(cardParams);

            ImageView backendIcon = backendIconView(ServerStore.Server.BACKEND_KIMI);
            LinearLayout.LayoutParams logoParams = new LinearLayout.LayoutParams(dp(26), dp(26));
            logoParams.setMargins(0, 0, dp(8), 0);
            card.addView(backendIcon, logoParams);

            LinearLayout text = new LinearLayout(context);
            text.setOrientation(LinearLayout.VERTICAL);
            TextView title = new TextView(context);
            title.setTextSize(16);
            title.setTextColor(Color.rgb(28, 34, 45));
            TextView address = new TextView(context);
            address.setTextSize(13);
            address.setTextColor(Color.rgb(112, 120, 135));
            address.setPadding(0, dp(3), 0, 0);
            text.addView(title);
            text.addView(address);
            card.addView(text, new LinearLayout.LayoutParams(0, -2, 1));

            TextView badge = serverBadge("当前", Color.rgb(25, 112, 238), Color.rgb(220, 235, 255));
            badge.setVisibility(View.GONE);
            LinearLayout.LayoutParams badgeParams = new LinearLayout.LayoutParams(dp(44), dp(28));
            card.addView(badge, badgeParams);

            ImageButton edit = serverIcon(R.drawable.ic_edit, "", false);
            ImageButton delete = serverIcon(R.drawable.ic_delete, "", true);
            LinearLayout.LayoutParams iconParams = new LinearLayout.LayoutParams(dp(38), dp(42));
            iconParams.setMargins(dp(3), 0, 0, 0);
            card.addView(edit, iconParams);
            card.addView(delete, iconParams);

            return new ViewHolder(card, backendIcon, title, address, badge, edit, delete);
        }

        @Override public void onBindViewHolder(ViewHolder holder, int position) {
            ServerStore.Server server = servers.get(position);
            boolean selected = server.id.equals(activeId);

            GradientDrawable cardBackground = new GradientDrawable();
            cardBackground.setColor(selected ? Color.rgb(238, 245, 255) : Color.rgb(248, 249, 252));
            cardBackground.setCornerRadius(dp(15));
            cardBackground.setStroke(dp(1), selected ? Color.rgb(143, 187, 248) : Color.rgb(226, 229, 236));
            holder.card.setBackground(new RippleDrawable(
                    ColorStateList.valueOf(Color.argb(28, 25, 112, 238)), cardBackground, null));

            holder.backendIcon.setImageResource(ServerStore.Server.BACKEND_DSH.equals(server.backend)
                    ? R.drawable.ic_backend_dsh : R.drawable.ic_backend_kimi);
            holder.backendIcon.clearColorFilter();
            boolean known = health.contains("checked_" + server.id);
            boolean online = health.getBoolean("online_" + server.id, false);
            if (!known || !online) {
                holder.backendIcon.setColorFilter(Color.rgb(156, 163, 175));
            }

            holder.title.setText(server.name);
            holder.address.setText(server.host + ":" + server.port);
            holder.badge.setVisibility(selected ? View.VISIBLE : View.GONE);

            holder.edit.setContentDescription("编辑 " + server.name);
            holder.delete.setContentDescription("删除 " + server.name);

            holder.card.setOnClickListener(v -> { dialog.dismiss(); switchServer(server.id); });
            holder.edit.setOnClickListener(v -> { dialog.dismiss(); showConfig(false, server, true); });
            holder.delete.setOnClickListener(v -> { dialog.dismiss(); deleteServer(server); });
        }

        class ViewHolder extends RecyclerView.ViewHolder {
            final LinearLayout card;
            final ImageView backendIcon;
            final TextView title, address, badge;
            final ImageButton edit, delete;

            ViewHolder(LinearLayout card, ImageView backendIcon, TextView title, TextView address,
                       TextView badge, ImageButton edit, ImageButton delete) {
                super(card);
                this.card = card;
                this.backendIcon = backendIcon;
                this.title = title;
                this.address = address;
                this.badge = badge;
                this.edit = edit;
                this.delete = delete;
            }
        }
    }

    /** 把全部服务器配置序列化为 JSON 数组并复制到系统剪贴板（格式与 ServerStore 存储一致，桌面端可直接导入）。 */
    private void copyServerConfig() {
        List<ServerStore.Server> servers = ServerStore.load(this);
        JSONArray items = new JSONArray();
        for (ServerStore.Server server : servers) try {
            items.put(server.json());
        } catch (Exception ignored) {}
        ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
        clipboard.setPrimaryClip(ClipData.newPlainText("服务器配置", items.toString()));
        Toast.makeText(this, "配置已复制到剪贴板（含 API 凭据，注意安全）", Toast.LENGTH_LONG).show();
    }

    /** 服务器后端类型图标：dsh 用鲸鱼 logo，Kimi 用官方 logo。 */
    private ImageView backendIconView(String backend) {
        ImageView icon = new ImageView(this);
        icon.setImageResource(ServerStore.Server.BACKEND_DSH.equals(backend)
                ? R.drawable.ic_backend_dsh : R.drawable.ic_backend_kimi);
        icon.setScaleType(android.widget.ImageView.ScaleType.CENTER_INSIDE);
        return icon;
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

    /** 活跃会话入口：直接打开服务器列表的"活跃会话"页签。 */
    private void showBusySessions() {
        showServerList(true);
    }

    /** 过滤掉默认的占位会话标题，避免在卡片里展示无意义的文字。 */
    private boolean isMeaningfulSessionTitle(String title) {
        if (title == null) return false;
        String trimmed = title.trim();
        if (trimmed.isEmpty()) return false;
        return !"会话".equals(trimmed)
                && !"点击查看会话".equals(trimmed)
                && !"点击查看 Kimi 会话".equals(trimmed)
                && !"New Session".equals(trimmed);
    }

    private void openSession(String serverId, String sessionId) {
        List<ServerStore.Server> servers = ServerStore.load(this);
        for (ServerStore.Server server : servers) {
            if (server.id.equals(serverId)) {
                ServerStore.save(this, servers, serverId);
                loadConfiguredUrl(sessionId);
                return;
            }
        }
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
        TextView backendLabel = fieldLabel("服务器类型");
        final RadioButton kimiRadio = new RadioButton(this);
        kimiRadio.setText("Kimi Code");
        kimiRadio.setTextSize(14);
        final RadioButton dshRadio = new RadioButton(this);
        dshRadio.setText("DeepSeek Harness");
        dshRadio.setTextSize(14);
        RadioGroup backendGroup = new RadioGroup(this);
        backendGroup.setOrientation(RadioGroup.HORIZONTAL);
        backendGroup.setPadding(0, 0, 0, dp(10));
        backendGroup.addView(kimiRadio);
        backendGroup.addView(dshRadio);
        String editingBackend = editing == null ? ServerStore.Server.BACKEND_KIMI : editing.backend;
        kimiRadio.setChecked(ServerStore.Server.BACKEND_KIMI.equals(editingBackend));
        dshRadio.setChecked(ServerStore.Server.BACKEND_DSH.equals(editingBackend));
        EditText name = modernField("服务器名称（例如：工作站）", false);
        name.setText(editing == null ? "" : editing.name);
        EditText ip = modernField("IP 地址或主机名", false);
        ip.setText(editing == null ? "" : editing.host);
        EditText port = modernField("端口", false);
        port.setInputType(android.text.InputType.TYPE_CLASS_NUMBER);
        port.setText(String.valueOf(editing == null ? 58627 : editing.port));
        EditText token = modernField("Token（Kimi 专用，dsh 留空）", false);
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
            Toast.makeText(this, "已识别，正在探测服务器类型…", Toast.LENGTH_SHORT).show();
            probeBackend(parsed.host, parsed.port, backend -> runOnUiThread(() -> {
                if (backend == null) {
                    // 探测失败：dsh 启动串已标明类型则采用，否则回落 Kimi
                    if (ServerStore.Server.BACKEND_DSH.equals(parsed.backend)) dshRadio.setChecked(true);
                    else kimiRadio.setChecked(true);
                    Toast.makeText(this, "未能确认服务器类型，已按 "
                            + (dshRadio.isChecked() ? "DeepSeek Harness" : "Kimi Code") + " 处理", Toast.LENGTH_SHORT).show();
                } else if (ServerStore.Server.BACKEND_DSH.equals(backend)) {
                    dshRadio.setChecked(true);
                    Toast.makeText(this, "已探测到 DeepSeek Harness", Toast.LENGTH_SHORT).show();
                } else {
                    kimiRadio.setChecked(true);
                    Toast.makeText(this, "已探测到 Kimi Code", Toast.LENGTH_SHORT).show();
                }
            }));
        });
        box.addView(hint); box.addView(quickLabel); box.addView(pasted); box.addView(recognize); box.addView(detailLabel);
        box.addView(backendLabel); box.addView(backendGroup);
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
            String backend = dshRadio.isChecked() ? ServerStore.Server.BACKEND_DSH : ServerStore.Server.BACKEND_KIMI;
            String tokenValue = ServerStore.Server.BACKEND_DSH.equals(backend)
                    ? "" : token.getText().toString().trim();
            ServerStore.Server saved = new ServerStore.Server(id, displayName, host, p, tokenValue, backend);
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
        final String host, token, backend;
        final int port;
        ParsedConnection(String host, int port, String token) {
            this(host, port, token, ServerStore.Server.BACKEND_KIMI);
        }
        ParsedConnection(String host, int port, String token, String backend) {
            this.host = host; this.port = port; this.token = token; this.backend = backend;
        }
    }

    private ParsedConnection parseConnection(String text) {
        try {
            boolean isDsh = text.contains("dsh web") || text.contains("DeepSeek Harness");
            String raw = null;
            if (isDsh) {
                // dsh 启动输出形如 "dsh web: http://127.0.0.1:3080 (LAN: http://100.x.y.z:3080)"，
                // 优先取 LAN 地址（127.0.0.1 是手机自身，无法访问）。
                Matcher lanMatch = Pattern.compile("LAN: (https?://[^\\s)]+)").matcher(text);
                if (lanMatch.find()) raw = lanMatch.group(1);
            }
            if (raw == null) {
                Matcher urlMatch = Pattern.compile("https?://[^\\s]+", Pattern.CASE_INSENSITIVE).matcher(text);
                raw = urlMatch.find() ? urlMatch.group() : text.trim();
            }
            raw = raw.replaceAll("[),;，。]+$", "");
            Uri uri = Uri.parse(raw);
            String host = uri.getHost();
            int port = uri.getPort();
            if (host == null || host.isEmpty()) return null;
            if (port < 1) port = "https".equalsIgnoreCase(uri.getScheme()) ? 443 : 80;
            String foundToken = "";
            if (!isDsh) {
                String fragment = uri.getFragment();
                if (fragment != null) foundToken = Uri.parse("http://local/?" + fragment).getQueryParameter("token");
                if (foundToken == null || foundToken.isEmpty()) foundToken = uri.getQueryParameter("token");
                if (foundToken == null || foundToken.isEmpty()) {
                    Matcher tokenMatch = Pattern.compile("(?i)(?:auth[-_ ]?token|token)\\s*[:=]\\s*([A-Za-z0-9._~-]+)").matcher(text);
                    foundToken = tokenMatch.find() ? tokenMatch.group(1) : "";
                }
            }
            return new ParsedConnection(host, port, foundToken == null ? "" : foundToken,
                    isDsh ? ServerStore.Server.BACKEND_DSH : ServerStore.Server.BACKEND_KIMI);
        } catch (Exception ignored) { return null; }
    }

    /** 后端类型探测回调：backend 为 null 表示无法确认。 */
    private interface BackendProbe { void onResult(String backend); }

    /**
     * 自动探测服务器后端类型。
     * dsh 特征：POST /api/agentPreset.list 返回 RPC 信封（type=server-response）；
     * Kimi 特征：GET /api/v2/sessions 存在（无 token 时 401/403 也算存在）。
     */
    private void probeBackend(String host, int port, BackendProbe callback) {
        String base = "http://" + host + ":" + port;
        Request dshProbe = new Request.Builder()
                .url(base + "/api/agentPreset.list")
                .post(RequestBody.create("{}", MediaType.parse("application/json; charset=utf-8")))
                .build();
        probeClient.newCall(dshProbe).enqueue(new Callback() {
            @Override public void onFailure(Call call, IOException e) { probeKimi(); }
            @Override public void onResponse(Call call, Response response) {
                try (Response r = response) {
                    String body = r.body() != null ? r.body().string() : "";
                    if (r.isSuccessful() && body.contains("\"type\":\"server-response\"")) {
                        callback.onResult(ServerStore.Server.BACKEND_DSH);
                        return;
                    }
                } catch (Exception ignored) {}
                probeKimi();
            }
            private void probeKimi() {
                Request kimiProbe = new Request.Builder().url(base + "/api/v2/sessions").build();
                probeClient.newCall(kimiProbe).enqueue(new Callback() {
                    @Override public void onFailure(Call call, IOException e) { callback.onResult(null); }
                    @Override public void onResponse(Call call, Response response) {
                        try (Response r = response) {
                            callback.onResult(r.code() != 404 ? ServerStore.Server.BACKEND_KIMI : null);
                        } catch (Exception ignored) { callback.onResult(null); }
                    }
                });
            }
        });
    }

    private void loadConfiguredUrl() {
        loadConfiguredUrl(null);
    }

    private void loadConfiguredUrl(String sessionId) {
        ServerStore.Server server = ServerStore.active(this);
        if (server == null) return;
        if (ServerStore.Server.BACKEND_DSH.equals(server.backend)) {
            // dsh：无 token 鉴权，前端也没有 URL 会话深链，只打开首页
            webView.loadUrl(server.baseUrl() + "/");
            return;
        }
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
