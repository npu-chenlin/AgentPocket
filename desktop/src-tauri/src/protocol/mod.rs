use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use thiserror::Error;

use crate::model::{AgentEvent, AgentEventKind};

pub mod dsh;
pub mod kimi;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProtocolState {
    pub titles: HashMap<String, String>,
    pub busy: HashSet<String>,
    pub baseline_complete: bool,
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
