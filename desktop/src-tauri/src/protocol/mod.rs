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
    pub busy: HashSet<String>,
    pub baseline_complete: bool,
    /// 服务端版本号（kimi 从 meta 接口获取；dsh 暂无此信息）。
    pub server_version: Option<String>,
    /// 各会话当前正在执行的活动（kimi transcript 流解析而来）。
    pub activities: HashMap<String, SessionActivity>,
    /// 已通过 subscribe_v2 订阅 transcript 的会话 id。
    pub v2_subscribed: HashSet<String>,
}

/// 单个会话的实时活动，来自 transcript.ops 的相位与工具帧。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionActivity {
    /// 展示文本，如 "Bash · git push" 或 "思考中"。
    pub display: Option<String>,
    /// 当前相位指向的工具调用 (toolCallId, 工具名)。
    pub current_tool: Option<(String, String)>,
    /// 工具帧的命令预览 (toolCallId -> 首行截断)。
    pub tool_commands: HashMap<String, String>,
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
