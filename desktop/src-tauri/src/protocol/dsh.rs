use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use crate::model::{AgentEvent, AgentEventKind};
use crate::protocol::{build_event, or_uuid, ProtocolError, ProtocolState};

pub fn parse_session_list(body: &str, state: &mut ProtocolState) -> Result<(), ProtocolError> {
    let value: Value = serde_json::from_str(body).map_err(ProtocolError::Json)?;

    let result = value
        .get("result")
        .ok_or(ProtocolError::MissingField("result"))?;
    if !result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        return Err(ProtocolError::RpcNotOk);
    }

    let items = result
        .get("value")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
        .ok_or(ProtocolError::MissingField("value.items"))?;

    state.titles.clear();
    state.busy.clear();

    for item in items {
        let session_id = item
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or(ProtocolError::MissingField("sessionId"))?;

        let running = item
            .get("running")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if running {
            state.busy.insert(session_id.to_string());
        }

        if let Some(title) = item
            .get("projections")
            .and_then(|p| p.get("values"))
            .and_then(|v| v.get("title"))
            .and_then(|v| v.as_str())
        {
            if !title.is_empty() && title != "null" {
                state
                    .titles
                    .insert(session_id.to_string(), title.to_string());
            }
        }
    }

    state.baseline_complete = true;
    Ok(())
}

pub fn parse_frame(
    server_id: &str,
    text: &str,
    now: DateTime<Utc>,
    state: &mut ProtocolState,
) -> Result<Vec<AgentEvent>, ProtocolError> {
    let msg: Value = serde_json::from_str(text).map_err(ProtocolError::Json)?;

    if msg.get("type").and_then(|v| v.as_str()) != Some("server-request") {
        return Ok(Vec::new());
    }

    let method = msg
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let payload = msg.get("payload").and_then(|v| v.as_object());
    let payload = match payload {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let session_id = payload
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(String::from);

    match method {
        "session/subscribed" => Ok(Vec::new()),

        "session/event" => {
            let event = payload.get("event").and_then(|v| v.as_object());
            let mut events = Vec::new();
            if let Some(event) = event {
                handle_session_event(server_id, session_id, event, now, state, &mut events)?;
            }
            Ok(events)
        }

        "session/projection" => {
            if payload.get("key").and_then(|v| v.as_str()) == Some("title") {
                if let (Some(id), Some(title)) = (
                    session_id.as_ref(),
                    payload.get("value").and_then(|v| v.as_str()),
                ) {
                    if !title.is_empty() && title != "null" {
                        state.titles.insert(id.clone(), title.to_string());
                    }
                }
            }
            Ok(Vec::new())
        }

        "approval/requested" => {
            let approval_id = or_uuid(payload.get("approvalId").and_then(|v| v.as_str()));
            Ok(vec![build_event(
                server_id,
                session_id,
                AgentEventKind::ApprovalRequired,
                format!("approval:{}", approval_id),
                now,
                state,
            )])
        }

        "question/requested" => {
            let rpc_id = or_uuid(msg.get("rpcId").and_then(|v| v.as_str()));
            Ok(vec![build_event(
                server_id,
                session_id,
                AgentEventKind::QuestionRequired,
                format!("question:{}", rpc_id),
                now,
                state,
            )])
        }

        "session/queue" | "approval/resolved" | "question/resolved" => Ok(Vec::new()),

        _ => Ok(Vec::new()),
    }
}

fn handle_session_event(
    server_id: &str,
    session_id: Option<String>,
    event: &serde_json::Map<String, Value>,
    now: DateTime<Utc>,
    state: &mut ProtocolState,
    out: &mut Vec<AgentEvent>,
) -> Result<(), ProtocolError> {
    let event_type = event
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    match event_type {
        "turn/start" => {
            if let Some(ref id) = session_id {
                state.busy.insert(id.clone());
            }
        }

        "turn/end" => {
            if let Some(ref id) = session_id {
                state.busy.remove(id);
            }
            let kind = event
                .get("data")
                .and_then(|d| d.get("reason"))
                .and_then(|r| r.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            let seq = event.get("seq").and_then(|v| v.as_i64()).unwrap_or(-1);
            let key = if seq >= 0 {
                format!("turn-end:{}", seq)
            } else {
                format!("turn-end:{}", Uuid::new_v4())
            };

            let kind = match kind {
                "completed" => AgentEventKind::Completed,
                "error" | "aborted" | "blocked" | "max-tokens" | "interrupted" => {
                    AgentEventKind::Failed
                }
                _ => return Ok(()),
            };

            out.push(build_event(server_id, session_id, kind, key, now, state));
        }

        "session/title" => {
            if let (Some(id), Some(title)) = (
                session_id.as_ref(),
                event
                    .get("data")
                    .and_then(|d| d.get("title"))
                    .and_then(|v| v.as_str()),
            ) {
                if !title.is_empty() && title != "null" {
                    state.titles.insert(id.clone(), title.to_string());
                }
            }
        }

        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_lines(server_id: &str, text: &str, state: &mut ProtocolState) -> Vec<AgentEvent> {
        let now = Utc::now();
        text.lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .flat_map(|line| parse_frame(server_id, line, now, state).unwrap())
            .collect()
    }

    fn collect_kinds(events: Vec<AgentEvent>) -> Vec<AgentEventKind> {
        events.into_iter().map(|e| e.kind).collect()
    }

    #[test]
    fn session_list_populates_state() {
        let mut state = ProtocolState::default();
        parse_session_list(
            include_str!("../../tests/fixtures/dsh-session-list.json"),
            &mut state,
        )
        .unwrap();

        assert_eq!(state.busy.len(), 1);
        assert!(state.busy.contains("sess-a1"));
        assert!(!state.busy.contains("sess-a2"));
        assert_eq!(
            state.titles.get("sess-a1"),
            Some(&"Fixture Title A".to_string())
        );
        assert_eq!(
            state.titles.get("sess-a2"),
            Some(&"Fixture Title B".to_string())
        );
        assert!(state.baseline_complete);
    }

    #[test]
    fn frame_events_match_expected_kinds() {
        let mut state = ProtocolState::default();
        parse_session_list(
            include_str!("../../tests/fixtures/dsh-session-list.json"),
            &mut state,
        )
        .unwrap();

        let events = parse_lines(
            "srv-dsh",
            include_str!("../../tests/fixtures/dsh-events.json"),
            &mut state,
        );

        assert_eq!(
            collect_kinds(events),
            vec![
                AgentEventKind::ApprovalRequired,
                AgentEventKind::QuestionRequired,
                AgentEventKind::Completed,
                AgentEventKind::Failed,
            ]
        );
    }

    #[test]
    fn ignored_frames_yield_no_events() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        let frames = vec![
            r#"{"type":"server-request","method":"tool/call","rpcId":"rpc-t1","payload":{"sessionId":"sess-a1"}}"#,
            r#"{"type":"server-request","method":"assistant/chunk","rpcId":"rpc-t2","payload":{"sessionId":"sess-a1"}}"#,
            r#"{"type":"server-request","method":"session/queue","rpcId":"rpc-t3","payload":{"sessionId":"sess-a1"}}"#,
            r#"{"type":"server-request","method":"unknown/method","rpcId":"rpc-t4","payload":{"sessionId":"sess-a1"}}"#,
            r#"{"type":"server-response","rpcId":"rpc-t5","payload":{"sessionId":"sess-a1"}}"#,
        ];

        for frame in frames {
            assert!(parse_frame("srv-dsh", frame, now, &mut state)
                .unwrap()
                .is_empty());
        }
    }
}
