use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::model::{AgentEvent, AgentEventKind};
use crate::protocol::{build_event, or_uuid, ProtocolError, ProtocolState};

pub fn parse_frame(
    server_id: &str,
    text: &str,
    now: DateTime<Utc>,
    state: &mut ProtocolState,
) -> Result<Vec<AgentEvent>, ProtocolError> {
    let msg: Value = serde_json::from_str(text).map_err(ProtocolError::Json)?;
    let event_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or_default();

    match event_type {
        "server_hello" | "subscribe_ack" | "ack" | "ping" | "resync_required" | "error" => {
            Ok(Vec::new())
        }

        "transcript.ops" => {
            let session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            let ops = msg
                .pointer("/payload/ops")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !session_id.is_empty() {
                apply_activity_ops(session_id, &ops, state);
            }
            Ok(Vec::new())
        }

        "transcript.reset" => {
            // 订阅后的快照：用当前相位补齐已在执行中的活动。
            let session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if let Some(phase) = msg.pointer("/payload/snapshot/meta/agent/phase") {
                if !session_id.is_empty() {
                    apply_phase(session_id, phase, state);
                }
            }
            Ok(Vec::new())
        }

        _ if event_type.starts_with("event.session.") => {
            handle_protocol_event(server_id, &msg, now, state)
        }

        "prompt.submitted" => Ok(Vec::new()),

        "prompt.completed" | "prompt.aborted" => handle_agent_event(server_id, &msg, now, state),

        _ => Ok(Vec::new()),
    }
}

/// 命令预览：取首个非空行并截断，避免把整段脚本塞进界面。
fn command_preview(input_text: &str) -> String {
    let first_line = input_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    const MAX: usize = 80;
    let mut end = first_line.len().min(MAX);
    while !first_line.is_char_boundary(end) {
        end -= 1;
    }
    let mut preview = first_line[..end].to_string();
    if first_line.len() > end {
        preview.push('…');
    }
    preview
}

fn apply_phase(session_id: &str, phase: &Value, state: &mut ProtocolState) {
    let activity = state.activities.entry(session_id.to_string()).or_default();
    match phase.get("kind").and_then(|v| v.as_str()).unwrap_or_default() {
        "tool_call" => {
            let tool_call_id = phase
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let name = phase
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("工具")
                .to_string();
            let command = activity
                .tool_commands
                .get(&tool_call_id)
                .map(|cmd| format!(" · {}", cmd));
            activity.current_tool = Some((tool_call_id, name.clone()));
            activity.display = Some(format!("{}{}", name, command.unwrap_or_default()));
        }
        "streaming" | "running" => {
            activity.current_tool = None;
            activity.display = Some("思考中".to_string());
        }
        _ => {}
    }
}

fn apply_activity_ops(session_id: &str, ops: &[Value], state: &mut ProtocolState) {
    for op in ops {
        match op.get("op").and_then(|v| v.as_str()).unwrap_or_default() {
            "meta.merge" => {
                if let Some(phase) = op.pointer("/meta/agent/phase") {
                    apply_phase(session_id, phase, state);
                }
            }
            "frame.upsert" => {
                let Some(frame) = op.get("frame") else { continue };
                if frame.get("kind").and_then(|v| v.as_str()) != Some("tool") {
                    continue;
                }
                let Some(activity) = state.activities.get_mut(session_id) else {
                    continue;
                };
                let tool_call_id = frame
                    .get("toolCallId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if tool_call_id.is_empty() {
                    continue;
                }
                if let Some(input_text) = frame.get("inputText").and_then(|v| v.as_str()) {
                    let preview = command_preview(input_text);
                    if !preview.is_empty() {
                        activity.tool_commands.insert(tool_call_id.clone(), preview);
                    }
                }
                // 命令到达时若相位正指向该工具，刷新展示文本。
                if let Some((current_id, name)) = activity.current_tool.clone() {
                    if current_id == tool_call_id {
                        if let Some(cmd) = activity.tool_commands.get(&tool_call_id) {
                            activity.display = Some(format!("{} · {}", name, cmd));
                        }
                    }
                }
            }
            "turn.upsert" => {
                // 新轮次开始，丢弃上一轮的命令缓存防止无界增长。
                if let Some(activity) = state.activities.get_mut(session_id) {
                    activity.tool_commands.clear();
                }
            }
            _ => {}
        }
    }
}

fn handle_protocol_event(
    server_id: &str,
    msg: &Value,
    now: DateTime<Utc>,
    state: &mut ProtocolState,
) -> Result<Vec<AgentEvent>, ProtocolError> {
    let event_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or_default();
    let session_id = msg
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let payload = msg.get("payload").and_then(|v| v.as_object());
    let payload = match payload {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let mut events = Vec::new();

    match event_type {
        "event.session.status_changed" => {
            let status = payload
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("idle");
            let previous = payload
                .get("previous_status")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let was_active = !previous.is_empty() && previous != "idle";
            let is_active = status != "idle";

            if let Some(ref id) = session_id {
                if was_active && !is_active {
                    state.busy.remove(id);
                    state.activities.remove(id);
                } else if !was_active && is_active {
                    state.busy.insert(id.clone());
                }
            }

            if previous == "running" && status == "idle" {
                events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::Completed,
                    event_key(msg, "status-complete"),
                    now,
                    state,
                ));
            } else if status == "awaiting_approval" {
                events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::ApprovalRequired,
                    event_key(msg, "status-approval"),
                    now,
                    state,
                ));
            } else if status == "awaiting_question" {
                events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::QuestionRequired,
                    event_key(msg, "status-question"),
                    now,
                    state,
                ));
            } else if status == "aborted" {
                events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::Failed,
                    event_key(msg, "status-aborted"),
                    now,
                    state,
                ));
            }
        }

        "event.session.work_changed" => {
            let busy = payload
                .get("busy")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if let Some(ref id) = session_id {
                let was_busy = state.busy.contains(id);
                if busy && !was_busy {
                    state.busy.insert(id.clone());
                } else if !busy && was_busy {
                    state.busy.remove(id);
                    state.activities.remove(id);
                }
            }

            let pending = payload
                .get("pending_interaction")
                .and_then(|v| v.as_str())
                .unwrap_or("none");

            match pending {
                "approval" => events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::ApprovalRequired,
                    event_key(msg, "approval"),
                    now,
                    state,
                )),
                "question" => events.push(build_event(
                    server_id,
                    session_id,
                    AgentEventKind::QuestionRequired,
                    event_key(msg, "question"),
                    now,
                    state,
                )),
                _ => {}
            }
        }

        "event.session.created" => {
            if let Some(session_obj) = payload.get("session").and_then(|v| v.as_object()) {
                let new_id = session_obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                if let Some(ref id) = new_id {
                    let title = session_obj
                        .get("meta")
                        .and_then(|m| m.get("title"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("New Session");
                    let title = if title.is_empty() || title == "null" {
                        "New Session"
                    } else {
                        title
                    };
                    state.titles.insert(id.clone(), title.to_string());
                    state.busy.insert(id.clone());
                }
            }
        }

        "event.session.updated" | "event.session.deleted" => {}

        _ => {}
    }

    Ok(events)
}

fn handle_agent_event(
    server_id: &str,
    msg: &Value,
    now: DateTime<Utc>,
    state: &mut ProtocolState,
) -> Result<Vec<AgentEvent>, ProtocolError> {
    let event_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or_default();
    let session_id = msg
        .get("session_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let payload = msg.get("payload").and_then(|v| v.as_object());
    let payload = match payload {
        Some(p) => p,
        None => return Ok(Vec::new()),
    };

    let prompt_id = payload
        .get("promptId")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let (kind, prefix) = match event_type {
        "prompt.completed" => (AgentEventKind::Completed, "prompt-complete"),
        "prompt.aborted" => (AgentEventKind::Failed, "prompt-aborted"),
        _ => return Ok(Vec::new()),
    };

    let key = if prompt_id.is_empty() {
        event_key(msg, prefix)
    } else {
        format!("{}:{}", prefix, prompt_id)
    };

    Ok(vec![build_event(
        server_id, session_id, kind, key, now, state,
    )])
}

fn event_key(msg: &Value, prefix: &str) -> String {
    let epoch = msg
        .get("epoch")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    if let Some(seq) = msg.get("seq").and_then(|v| v.as_i64()) {
        if seq >= 0 {
            return format!("{}:{}:{}", prefix, epoch, seq);
        }
    }
    format!(
        "{}:{}",
        prefix,
        or_uuid(msg.get("id").and_then(|v| v.as_str()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str, state: &mut ProtocolState) -> Vec<AgentEventKind> {
        let now = Utc::now();
        text.lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .flat_map(|line| parse_frame("srv-kimi", line, now, state).unwrap())
            .map(|e| e.kind)
            .collect()
    }

    #[test]
    fn status_changed_and_prompt_events() {
        let mut state = ProtocolState::default();
        state
            .titles
            .insert("sess-k1".to_string(), "Fixture Title K1".to_string());
        state.busy.insert("sess-k1".to_string());

        let input = r#"
{"type":"server_hello"}
{"type":"event.session.status_changed","session_id":"sess-k1","payload":{"previous_status":"running","status":"idle"},"epoch":"1","seq":0}
{"type":"event.session.status_changed","session_id":"sess-k1","payload":{"previous_status":"idle","status":"awaiting_approval"},"epoch":"1","seq":1}
{"type":"prompt.completed","session_id":"sess-k1","payload":{"promptId":"prompt-001"}}
{"type":"prompt.aborted","session_id":"sess-k1","payload":{"promptId":"prompt-002"}}
"#;

        assert_eq!(
            kinds(input, &mut state),
            vec![
                AgentEventKind::Completed,
                AgentEventKind::ApprovalRequired,
                AgentEventKind::Completed,
                AgentEventKind::Failed,
            ]
        );
    }

    #[test]
    fn work_changed_and_created_update_state() {
        let mut state = ProtocolState::default();

        let input = r#"
{"type":"event.session.work_changed","session_id":"sess-k2","payload":{"busy":true,"pending_interaction":"approval"},"epoch":"2","seq":0}
{"type":"event.session.created","payload":{"session":{"id":"sess-k3","meta":{"title":"Fixture Title K3"}}}}
{"type":"event.session.work_changed","session_id":"sess-k2","payload":{"busy":false,"pending_interaction":"none"},"epoch":"2","seq":1}
"#;

        let events: Vec<_> = input
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .flat_map(|line| parse_frame("srv-kimi", line, Utc::now(), &mut state).unwrap())
            .collect();

        assert_eq!(
            events.iter().map(|e| e.kind.clone()).collect::<Vec<_>>(),
            vec![AgentEventKind::ApprovalRequired]
        );

        assert!(state.titles.contains_key("sess-k3"));
        assert_eq!(
            state.titles.get("sess-k3"),
            Some(&"Fixture Title K3".to_string())
        );
        assert!(state.busy.contains("sess-k3"));
        assert!(!state.busy.contains("sess-k2"));
    }

    #[test]
    fn duplicate_frames_do_not_drive_count_negative() {
        let mut state = ProtocolState::default();
        let frame = r#"{"type":"event.session.status_changed","session_id":"sess-k1","payload":{"previous_status":"running","status":"idle"},"epoch":"1","seq":10}"#;

        let now = Utc::now();
        let first = parse_frame("srv-kimi", frame, now, &mut state).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, AgentEventKind::Completed);

        let second = parse_frame("srv-kimi", frame, now, &mut state).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(state.busy.len(), 0);
    }

    #[test]
    fn unknown_events_are_ignored() {
        let mut state = ProtocolState::default();
        let frame = r#"{"type":"event.session.unknown","session_id":"sess-k1","payload":{}}"#;
        assert!(parse_frame("srv-kimi", frame, Utc::now(), &mut state)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn transcript_ops_build_activity_display() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 相位进入 tool_call，此时命令还没到。
        let phase = r#"{"type":"transcript.ops","session_id":"sess-a","payload":{"agent_id":"main","ops":[{"op":"meta.merge","meta":{"agent":{"phase":{"kind":"tool_call","turnId":1,"step":2,"toolCallId":"toolu_1","name":"Bash"}}}}]}}"#;
        parse_frame("srv-kimi", phase, now, &mut state).unwrap();
        assert_eq!(
            state.activities.get("sess-a").and_then(|a| a.display.clone()),
            Some("Bash".to_string())
        );

        // 工具帧补上命令，展示文本刷新为 "工具 · 命令首行"。
        let tool_frame = r#"{"type":"transcript.ops","session_id":"sess-a","payload":{"agent_id":"main","ops":[{"op":"frame.upsert","turnId":"t1","stepId":"t1.1","frame":{"kind":"tool","frameId":"f1","toolCallId":"toolu_1","name":"Bash","state":"running","inputText":"git push origin main\nsecond line"}}]}}"#;
        parse_frame("srv-kimi", tool_frame, now, &mut state).unwrap();
        assert_eq!(
            state.activities.get("sess-a").and_then(|a| a.display.clone()),
            Some("Bash · git push origin main".to_string())
        );

        // 相位回到 streaming，展示思考中。
        let streaming = r#"{"type":"transcript.ops","session_id":"sess-a","payload":{"agent_id":"main","ops":[{"op":"meta.merge","meta":{"agent":{"phase":{"kind":"streaming","turnId":1,"step":2,"stream":"thinking"}}}}]}}"#;
        parse_frame("srv-kimi", streaming, now, &mut state).unwrap();
        assert_eq!(
            state.activities.get("sess-a").and_then(|a| a.display.clone()),
            Some("思考中".to_string())
        );

        // 会话转空闲后活动清空。
        let idle = r#"{"type":"event.session.work_changed","session_id":"sess-a","payload":{"busy":false,"pending_interaction":"none"}}"#;
        state.busy.insert("sess-a".to_string());
        parse_frame("srv-kimi", idle, now, &mut state).unwrap();
        assert!(state.activities.get("sess-a").is_none());
    }

    #[test]
    fn command_preview_takes_first_line_and_truncates() {
        assert_eq!(command_preview("ls -la\npwd"), "ls -la");
        let long = "x".repeat(200);
        let preview = command_preview(&long);
        assert!(preview.chars().count() <= 81);
        assert!(preview.ends_with('…'));
        assert_eq!(command_preview("  \n tail"), "tail");
    }
}
