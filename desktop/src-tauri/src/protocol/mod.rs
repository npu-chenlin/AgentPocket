use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use thiserror::Error;
use uuid::Uuid;

use crate::model::{AgentEvent, AgentEventKind};

pub mod dsh;
pub mod kimi;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolState {
    pub titles: HashMap<String, String>,
    /// 有效忙碌：服务器报忙碌且（主回合活跃 或 仍有后台任务），界面据此转圈。
    pub busy: HashSet<String>,
    /// 服务器侧原始忙碌（work_changed 推送 / 会话列表种子）。
    pub raw_busy: HashSet<String>,
    /// 主回合已结束的会话（main_turn_active == false）。不在集合中默认视为活跃。
    pub main_turn_inactive: HashSet<String>,
    /// 各会话运行中的后台任务数（background.task.* 事件维护）。
    pub bg_running: HashMap<String, u32>,
    pub baseline_complete: bool,
    /// 服务端版本号（kimi 从 meta 接口获取；dsh 暂无此信息）。
    pub server_version: Option<String>,
    /// 各会话当前正在执行的活动（kimi 基础订阅的相位与工具事件解析而来）。
    pub activities: HashMap<String, SessionActivity>,
    /// 已纳入基础订阅（client_hello / subscribe）的会话 id。
    pub subscribed: HashSet<String>,
}

impl ProtocolState {
    /// 活动行文本：待审批/待回答 > 等后台 > 相位文本（与手机端 sessionState 一致）。
    pub fn activity_text(&self, session_id: &str) -> Option<String> {
        let activity = self.activities.get(session_id);
        match activity.and_then(|a| a.pending.as_deref()) {
            Some("approval") => return Some("等待审批".to_string()),
            Some("question") => return Some("等待回答".to_string()),
            _ => {}
        }
        if self.main_turn_inactive.contains(session_id) {
            let running = self.bg_running.get(session_id).copied().unwrap_or(0);
            return Some(if running > 0 {
                format!("主 agent 已完成 · 等 {} 个后台任务", running)
            } else {
                "主 agent 已完成".to_string()
            });
        }
        activity.and_then(|a| a.display.clone())
    }

    /// 有效忙碌 = 服务器报 busy 且（主回合活跃 或 仍有后台任务）；变空闲即清空活动展示。
    pub fn apply_effective_busy(&mut self, session_id: &str) {
        let raw = self.raw_busy.contains(session_id);
        let main_active = !self.main_turn_inactive.contains(session_id);
        let running = self.bg_running.get(session_id).copied().unwrap_or(0);
        if raw && (main_active || running > 0) {
            self.busy.insert(session_id.to_string());
        } else {
            self.busy.remove(session_id);
            self.activities.remove(session_id);
        }
    }
}

/// 单个会话的实时活动，来自 agent.status.updated 相位与 tool.call.started 命令。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionActivity {
    /// 展示文本，如 "Bash · git push" 或 "思考中"。
    pub display: Option<String>,
    /// 当前相位指向的工具调用 (toolCallId, 工具名)。
    pub current_tool: Option<(String, String)>,
    /// 工具命令预览 (toolCallId -> 首行截断)。
    pub tool_commands: HashMap<String, String>,
    /// 服务器侧待交互状态（approval / question），展示时优先于相位文本。
    pub pending: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("invalid JSON frame")]
    Json(#[source] serde_json::Error),
    #[error("RPC returned ok=false or missing result")]
    RpcNotOk,
    #[error("missing field: {0}")]
    MissingField(&'static str),
}

pub(crate) fn or_uuid(value: Option<&str>) -> String {
    value
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

pub(crate) fn build_event(
    server_id: &str,
    session_id: Option<String>,
    kind: AgentEventKind,
    event_key: String,
    occurred_at: DateTime<Utc>,
    state: &ProtocolState,
) -> AgentEvent {
    let session_title = session_id
        .as_ref()
        .and_then(|id| state.titles.get(id).cloned());
    AgentEvent {
        server_id: server_id.to_string(),
        session_id,
        session_title,
        kind,
        event_key,
        body: None,
        occurred_at,
    }
}
