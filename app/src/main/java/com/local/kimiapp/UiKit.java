package com.local.kimiapp;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Color;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.widget.Button;

/**
 * 界面样式常量与通用底板构造器。
 * 主界面所有手写视图的配色、圆角、描边统一从这里取，禁止在业务代码里散落 rgb() 字面量。
 */
final class UiKit {
    private UiKit() {}

    // 主题蓝
    static final int BLUE = Color.rgb(25, 112, 238);
    static final int BLUE_BG = Color.rgb(238, 245, 255);
    static final int BLUE_STROKE = Color.rgb(143, 187, 248);
    static final int BLUE_BADGE_BG = Color.rgb(220, 235, 255);
    static final int BLUE_OUTLINE = Color.rgb(180, 207, 244);
    static final int BLUE_RIPPLE = Color.argb(30, 25, 112, 238);
    static final int BLUE_RIPPLE_SOFT = Color.argb(28, 25, 112, 238);

    // 中性色
    static final int TEXT_PRIMARY = Color.rgb(28, 34, 45);
    static final int TEXT_SECONDARY = Color.rgb(112, 120, 135);
    static final int TEXT_LABEL = Color.rgb(57, 65, 82);
    static final int TEXT_HINT = Color.rgb(96, 103, 117);
    static final int FIELD_HINT = Color.rgb(145, 151, 164);
    static final int ICON_DISABLED = Color.rgb(156, 163, 175);
    static final int CARD_BG = Color.rgb(248, 249, 252);
    static final int CARD_STROKE = Color.rgb(226, 229, 236);
    static final int ICON_BG = Color.rgb(241, 244, 249);
    static final int ICON_STROKE = Color.rgb(218, 224, 234);
    static final int BUTTON_BG = Color.rgb(245, 247, 250);
    static final int BUTTON_STROKE = Color.rgb(217, 222, 231);
    static final int BUTTON_TEXT = Color.rgb(78, 88, 105);
    static final int FIELD_STROKE = Color.rgb(222, 226, 234);
    static final int DIALOG_NEGATIVE = Color.rgb(94, 101, 116);

    // 危险操作
    static final int DANGER_BG = Color.rgb(255, 241, 241);
    static final int DANGER_STROKE = Color.rgb(247, 199, 199);
    static final int DANGER_RIPPLE = Color.argb(35, 217, 67, 67);
    static final int ICON_RIPPLE = Color.argb(35, 83, 98, 122);

    static int dp(Context context, int value) {
        return Math.round(value * context.getResources().getDisplayMetrics().density);
    }

    /** 圆角描边底板，外裹一层水波纹。 */
    static Drawable rippledPanel(Context context, int fill, int strokeColor,
                                 int strokeWidthDp, int radiusDp, int rippleColor) {
        GradientDrawable background = new GradientDrawable();
        background.setColor(fill);
        background.setCornerRadius(dp(context, radiusDp));
        if (strokeWidthDp > 0) background.setStroke(dp(context, strokeWidthDp), strokeColor);
        return new RippleDrawable(ColorStateList.valueOf(rippleColor), background, null);
    }

    /** 列表卡片底板：选中态蓝底蓝边，普通态灰底灰边。 */
    static Drawable cardBackground(Context context, boolean selected) {
        return rippledPanel(context,
                selected ? BLUE_BG : CARD_BG,
                selected ? BLUE_STROKE : CARD_STROKE,
                1, 15, BLUE_RIPPLE_SOFT);
    }

    /** 灰底描边的次级按钮（刷新页面 / 复制配置等）。 */
    static void styleSecondaryButton(Context context, Button button) {
        button.setTextSize(14);
        button.setTextColor(BUTTON_TEXT);
        button.setAllCaps(false);
        button.setElevation(0);
        button.setStateListAnimator(null);
        button.setBackground(rippledPanel(context, BUTTON_BG, BUTTON_STROKE, 1, 14,
                Color.argb(30, 78, 88, 105)));
    }

    /** 蓝色描边的主按钮（添加服务器）。 */
    static void styleOutlinePrimaryButton(Context context, Button button) {
        button.setTextSize(14);
        button.setTextColor(BLUE);
        button.setAllCaps(false);
        button.setElevation(0);
        button.setStateListAnimator(null);
        button.setBackground(rippledPanel(context, Color.TRANSPARENT, BLUE_OUTLINE, 1, 14, BLUE_RIPPLE));
    }

    /** 实心蓝按钮（识别并填充）。 */
    static void styleFilledPrimaryButton(Context context, Button button) {
        button.setTextSize(14);
        button.setTextColor(Color.WHITE);
        button.setAllCaps(false);
        button.setElevation(0);
        button.setStateListAnimator(null);
        button.setBackground(rippledPanel(context, BLUE, BLUE, 0, 12,
                Color.argb(65, 255, 255, 255)));
    }
}
