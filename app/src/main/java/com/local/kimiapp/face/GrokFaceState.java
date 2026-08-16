package com.local.kimiapp.face;

/** 悬浮球应用侧状态，映射到 gist 内部状态名（用于查表情池/节奏）。 */
public enum GrokFaceState {
    SLEEPING("sleeping"),
    WAKING("waking"),
    IDLE("idle"),
    WORKING("working"),
    LOADING("loading"),
    CELEBRATE("celebrate"),
    THINKING("thinking"),
    SAD("sad"),
    DROWSY("drowsy"),
    DRAGGING("dragging");

    /** gist 状态名，用于从 ExpressionData 查 POOLS / EXPR_CADENCE / BLINK。 */
    public final String gistState;

    GrokFaceState(String gistState) {
        this.gistState = gistState;
    }
}
