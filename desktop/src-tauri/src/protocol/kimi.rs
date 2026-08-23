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

    // 只关心主 agent：子代理的回合完成/审批/相位等事件一律忽略，
    // 避免子代理触发通知、污染忙碌状态与活动行。
    let agent_id = msg
        .pointer("/payload/agentId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !agent_id.is_empty() && agent_id != "main" {
        return Ok(Vec::new());
    }

    match event_type {
        "server_hello" | "subscribe_ack" | "ack" | "ping" | "resync_required" | "error" => {
            Ok(Vec::new())
        }

        // 相位事件（基础订阅推送）：更新活动文本，区分思考/输出/工具。
        "agent.status.updated" => {
            let session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if let Some(phase) = msg.pointer("/payload/phase") {
                if !session_id.is_empty() {
                    apply_phase(session_id, phase, state);
                }
            }
            Ok(Vec::new())
        }

        // 工具开始事件（基础订阅推送）：命令预览入缓存；子代理按工具名识别。
        "tool.call.started" => {
            let session_id = msg
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !session_id.is_empty() {
                if let Some(payload) = msg.get("payload") {
                    handle_tool_call_started(session_id, payload, state);
                }
            }
            Ok(Vec::new())
        }

        // 轮次结束清命令缓存，防止无界增长。
        "turn.ended" => {
            if let Some(session_id) = msg.get("session_id").and_then(|v| v.as_str()) {
                if let Some(activity) = state.activities.get_mut(session_id) {
                    activity.tool_commands.clear();
                }
            }
            Ok(Vec::new())
        }

        // 后台任务生命周期：增减运行计数后重算有效忙碌。
        "background.task.started" | "background.task.terminated" => {
            if let Some(session_id) = msg.get("session_id").and_then(|v| v.as_str()) {
                if !session_id.is_empty() {
                    let delta: i32 = if event_type == "background.task.started" { 1 } else { -1 };
                    let current = state
                        .bg_running
                        .get(session_id)
                        .copied()
                        .unwrap_or(0) as i32;
                    let next = (current + delta).max(0) as u32;
                    if next == 0 {
                        state.bg_running.remove(session_id);
                    } else {
                        state.bg_running.insert(session_id.to_string(), next);
                    }
                    state.apply_effective_busy(session_id);
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
            let name = display_name(
                phase
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("工具"),
            );
            let command = activity.tool_commands.get(&tool_call_id).cloned();
            activity.current_tool = Some((tool_call_id, name.clone()));
            activity.display = Some(tool_display(&name, command.as_deref()));
        }
        "streaming" | "running" => {
            let stream = phase
                .get("stream")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            activity.current_tool = None;
            activity.display = Some(if stream == "assistant" {
                "输出中".to_string()
            } else {
                "思考中".to_string()
            });
        }
        "ended" => {
            activity.current_tool = None;
        }
        _ => {}
    }
}

fn handle_tool_call_started(session_id: &str, payload: &Value, state: &mut ProtocolState) {
    let tool_call_id = payload
        .get("toolCallId")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if tool_call_id.is_empty() {
        return;
    }

    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut command = payload
        .pointer("/display/command")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if command.is_empty() {
        command = payload
            .pointer("/args/command")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
    }
    if command.is_empty() {
        command = payload
            .pointer("/args/description")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
    }

    let preview = command_preview(&command);
    let activity = state.activities.entry(session_id.to_string()).or_default();
    if !preview.is_empty() {
        activity
            .tool_commands
            .insert(tool_call_id.to_string(), preview.clone());
    }
    // 相位已指向该工具时刷新展示（相位事件可能先到）。
    if let Some((current_id, current_name)) = activity.current_tool.clone() {
        if current_id == tool_call_id {
            let display_name = if name.is_empty() {
                current_name
            } else {
                display_name(&name)
            };
            activity.display = Some(tool_display(&display_name, Some(preview.as_str())));
        }
    }
}

fn is_subagent_tool(name: &str) -> bool {
    matches!(name, "Agent" | "AgentSwarm" | "Task")
}

fn display_name(name: &str) -> String {
    if is_subagent_tool(name) {
        "子代理".to_string()
    } else {
        name.to_string()
    }
}

fn tool_display(name: &str, command: Option<&str>) -> String {
    match command.filter(|c| !c.is_empty()) {
        Some(cmd) => format!("{} · {}", name, cmd),
        None => name.to_string(),
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

            // 状态跃迁等价于主回合活跃性变化；busy 以 work_changed 推送为准。
            // 通知不在这里发：完成/失败走 prompt.*，审批/回答走 work_changed，
            // 单一通路避免一次事件两次提醒。
            if let Some(ref id) = session_id {
                if was_active != is_active {
                    if is_active {
                        state.main_turn_inactive.remove(id);
                    } else {
                        state.main_turn_inactive.insert(id.clone());
                    }
                    state.apply_effective_busy(id);
                }
            }
        }

        "event.session.work_changed" => {
            let busy = payload
                .get("busy")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let pending = payload
                .get("pending_interaction")
                .and_then(|v| v.as_str())
                .unwrap_or("none");

            let prev_pending = session_id
                .as_ref()
                .and_then(|id| state.activities.get(id))
                .and_then(|a| a.pending.clone());

            if let Some(ref id) = session_id {
                // 推送自带细分字段，直接落状态；主回合活跃性缺省视为活跃。
                if busy {
                    state.raw_busy.insert(id.clone());
                } else {
                    state.raw_busy.remove(id);
                }
                if let Some(main_active) = payload.get("main_turn_active").and_then(|v| v.as_bool())
                {
                    if main_active {
                        state.main_turn_inactive.remove(id);
                    } else {
                        state.main_turn_inactive.insert(id.clone());
                    }
                }
                state.apply_effective_busy(id);
                // 待审批/待回答在活动行优先展示，none 回落相位文本。
                if state.busy.contains(id) {
                    state.activities.entry(id.clone()).or_default().pending = match pending {
                        "approval" | "question" => Some(pending.to_string()),
                        _ => None,
                    };
                }
            }

            // 仅在待交互状态跃迁时提醒一次，重复推送不刷屏。
            match pending {
                "approval" if prev_pending.as_deref() != Some("approval") => events.push(
                    build_event(
                        server_id,
                        session_id,
                        AgentEventKind::ApprovalRequired,
                        event_key(msg, "approval"),
                        now,
                        state,
                    ),
                ),
                "question" if prev_pending.as_deref() != Some("question") => events.push(
                    build_event(
                        server_id,
                        session_id,
                        AgentEventKind::QuestionRequired,
                        event_key(msg, "question"),
                        now,
                        state,
                    ),
                ),
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
                    state.raw_busy.insert(id.clone());
                    state.main_turn_inactive.remove(id);
                    state.apply_effective_busy(id);
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
    use crate::protocol::SessionActivity;

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
    fn status_changed_is_silent_prompts_carry_completion() {
        // status_changed 只管主回合活跃性，不产通知事件；
        // 完成/失败由 prompt.* 单一通路产出，避免一次回合两次提醒。
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
            vec![AgentEventKind::Completed, AgentEventKind::Failed]
        );
    }

    #[test]
    fn approval_notified_once_per_pending_transition() {
        // 同一个待审批状态内重复的 work_changed 只提醒一次；回到 none 再进入才再次提醒。
        let mut state = ProtocolState::default();
        let now = Utc::now();

        let approval = r#"{"type":"event.session.work_changed","session_id":"sess-q","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"approval"},"epoch":"1","seq":0}"#;
        let again = r#"{"type":"event.session.work_changed","session_id":"sess-q","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"approval"},"epoch":"1","seq":5}"#;
        let none = r#"{"type":"event.session.work_changed","session_id":"sess-q","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"none"},"epoch":"1","seq":6}"#;

        let first = parse_frame("srv-kimi", approval, now, &mut state).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].kind, AgentEventKind::ApprovalRequired);

        assert!(parse_frame("srv-kimi", again, now, &mut state)
            .unwrap()
            .is_empty());
        assert!(parse_frame("srv-kimi", none, now, &mut state)
            .unwrap()
            .is_empty());

        let second = parse_frame("srv-kimi", approval, now, &mut state).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].kind, AgentEventKind::ApprovalRequired);
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
        // status_changed 不产事件，也不把忙碌计数推成负数。
        let first = parse_frame("srv-kimi", frame, now, &mut state).unwrap();
        assert!(first.is_empty());

        let second = parse_frame("srv-kimi", frame, now, &mut state).unwrap();
        assert!(second.is_empty());
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

    fn display_of(state: &ProtocolState, session_id: &str) -> Option<String> {
        state
            .activities
            .get(session_id)
            .and_then(|a| a.display.clone())
    }

    #[test]
    fn subagent_events_are_ignored() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 子代理的回合完成不触发通知事件。
        let sub = r#"{"type":"prompt.completed","session_id":"sess-s","payload":{"promptId":"p1","agentId":"agent-sub"}}"#;
        assert!(parse_frame("srv-kimi", sub, now, &mut state)
            .unwrap()
            .is_empty());

        // 子代理的相位不写活动行。
        let sub_phase = r#"{"type":"agent.status.updated","session_id":"sess-s","payload":{"agentId":"agent-sub","phase":{"kind":"streaming","stream":"thinking"}}}"#;
        assert!(parse_frame("srv-kimi", sub_phase, now, &mut state)
            .unwrap()
            .is_empty());
        assert!(!state.activities.contains_key("sess-s"));

        // 主 agent 事件照常生效。
        let main = r#"{"type":"prompt.completed","session_id":"sess-s","payload":{"promptId":"p2","agentId":"main"}}"#;
        let events = parse_frame("srv-kimi", main, now, &mut state).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentEventKind::Completed);
    }

    #[test]
    fn agent_status_updated_drives_activity_display() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 思考流 → 思考中。
        let thinking = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"streaming","stream":"thinking"}}}"#;
        parse_frame("srv-kimi", thinking, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("思考中".to_string()));

        // 助手流 → 输出中。
        let assistant = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"streaming","stream":"assistant"}}}"#;
        parse_frame("srv-kimi", assistant, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("输出中".to_string()));

        // 子代理工具改名展示。
        let tool = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"tool_call","toolCallId":"toolu_9","name":"Agent"}}}"#;
        parse_frame("srv-kimi", tool, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("子代理".to_string()));

        // ended 只清当前工具指向，不清展示文本。
        let ended = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"ended"}}}"#;
        parse_frame("srv-kimi", ended, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("子代理".to_string()));
        assert_eq!(state.activities.get("sess-a").unwrap().current_tool, None);
    }

    #[test]
    fn tool_call_started_fills_command_preview() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 相位先指向工具，随后 tool.call.started 补上命令。
        let phase = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"tool_call","toolCallId":"toolu_1","name":"Bash"}}}"#;
        parse_frame("srv-kimi", phase, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("Bash".to_string()));

        let started = r#"{"type":"tool.call.started","session_id":"sess-a","payload":{"agentId":"main","toolCallId":"toolu_1","name":"Bash","display":{"command":"git push origin main\nsecond line"}}}"#;
        parse_frame("srv-kimi", started, now, &mut state).unwrap();
        assert_eq!(
            display_of(&state, "sess-a"),
            Some("Bash · git push origin main".to_string())
        );

        // 无 display.command 时退回 args.command。
        let phase2 = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"tool_call","toolCallId":"toolu_2","name":"Edit"}}}"#;
        parse_frame("srv-kimi", phase2, now, &mut state).unwrap();
        let started2 = r#"{"type":"tool.call.started","session_id":"sess-a","payload":{"agentId":"main","toolCallId":"toolu_2","name":"Edit","args":{"command":"apply patch"}}}"#;
        parse_frame("srv-kimi", started2, now, &mut state).unwrap();
        assert_eq!(
            display_of(&state, "sess-a"),
            Some("Edit · apply patch".to_string())
        );

        // 子代理工具：名字改「子代理」，命令退回 args.description。
        let phase3 = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"tool_call","toolCallId":"toolu_3","name":"Agent"}}}"#;
        parse_frame("srv-kimi", phase3, now, &mut state).unwrap();
        let started3 = r#"{"type":"tool.call.started","session_id":"sess-a","payload":{"agentId":"main","toolCallId":"toolu_3","name":"Agent","args":{"description":"explore repo"}}}"#;
        parse_frame("srv-kimi", started3, now, &mut state).unwrap();
        assert_eq!(
            display_of(&state, "sess-a"),
            Some("子代理 · explore repo".to_string())
        );
    }

    #[test]
    fn turn_ended_clears_command_cache() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        let phase = r#"{"type":"agent.status.updated","session_id":"sess-a","payload":{"agentId":"main","phase":{"kind":"tool_call","toolCallId":"toolu_1","name":"Bash"}}}"#;
        parse_frame("srv-kimi", phase, now, &mut state).unwrap();
        let started = r#"{"type":"tool.call.started","session_id":"sess-a","payload":{"agentId":"main","toolCallId":"toolu_1","name":"Bash","display":{"command":"ls -la"}}}"#;
        parse_frame("srv-kimi", started, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("Bash · ls -la".to_string()));

        // 轮次结束清命令缓存；同一 toolCallId 再次进入相位时只剩工具名。
        let ended = r#"{"type":"turn.ended","session_id":"sess-a","payload":{}}"#;
        parse_frame("srv-kimi", ended, now, &mut state).unwrap();
        parse_frame("srv-kimi", phase, now, &mut state).unwrap();
        assert_eq!(display_of(&state, "sess-a"), Some("Bash".to_string()));
    }

    #[test]
    fn effective_busy_requires_main_turn_or_background_task() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 服务器报 busy 但主回合已结束：不转圈。
        let idle_main = r#"{"type":"event.session.work_changed","session_id":"sess-b","payload":{"busy":true,"main_turn_active":false,"pending_interaction":"none"}}"#;
        parse_frame("srv-kimi", idle_main, now, &mut state).unwrap();
        assert!(!state.busy.contains("sess-b"));

        // 后台任务启动 → 忙碌。
        let bg_start = r#"{"type":"background.task.started","session_id":"sess-b","payload":{}}"#;
        parse_frame("srv-kimi", bg_start, now, &mut state).unwrap();
        assert!(state.busy.contains("sess-b"));

        // 后台任务结束 → 回空闲并清空活动展示。
        state.activities.insert(
            "sess-b".to_string(),
            SessionActivity {
                display: Some("思考中".to_string()),
                ..Default::default()
            },
        );
        let bg_end = r#"{"type":"background.task.terminated","session_id":"sess-b","payload":{}}"#;
        parse_frame("srv-kimi", bg_end, now, &mut state).unwrap();
        assert!(!state.busy.contains("sess-b"));
        assert!(!state.activities.contains_key("sess-b"));
    }

    #[test]
    fn pending_interaction_overrides_activity_display() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 相位停在思考中，随后服务器推送待审批：展示切「等待审批」。
        let thinking = r#"{"type":"agent.status.updated","session_id":"sess-p","payload":{"agentId":"main","phase":{"kind":"streaming","stream":"thinking"}}}"#;
        parse_frame("srv-kimi", thinking, now, &mut state).unwrap();
        let approval = r#"{"type":"event.session.work_changed","session_id":"sess-p","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"approval"}}"#;
        parse_frame("srv-kimi", approval, now, &mut state).unwrap();
        assert_eq!(
            state
                .activities
                .get("sess-p")
                .and_then(|a| a.effective_display()),
            Some("等待审批".to_string())
        );

        // 回到 none 后恢复相位文本。
        let none = r#"{"type":"event.session.work_changed","session_id":"sess-p","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"none"}}"#;
        parse_frame("srv-kimi", none, now, &mut state).unwrap();
        assert_eq!(
            state
                .activities
                .get("sess-p")
                .and_then(|a| a.effective_display()),
            Some("思考中".to_string())
        );

        // 待回答同理。
        let question = r#"{"type":"event.session.work_changed","session_id":"sess-p","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"question"}}"#;
        parse_frame("srv-kimi", question, now, &mut state).unwrap();
        assert_eq!(
            state
                .activities
                .get("sess-p")
                .and_then(|a| a.effective_display()),
            Some("等待回答".to_string())
        );
    }

    #[test]
    fn status_changed_tracks_main_turn_activity() {
        let mut state = ProtocolState::default();
        let now = Utc::now();

        // 种子：忙碌且主回合活跃。
        let start = r#"{"type":"event.session.work_changed","session_id":"sess-c","payload":{"busy":true,"main_turn_active":true,"pending_interaction":"none"}}"#;
        parse_frame("srv-kimi", start, now, &mut state).unwrap();
        assert!(state.busy.contains("sess-c"));

        // 主回合结束（无后台任务）→ 不再忙碌；完成通知由 prompt.completed 产出。
        let idle = r#"{"type":"event.session.status_changed","session_id":"sess-c","payload":{"previous_status":"running","status":"idle"},"epoch":"1","seq":3}"#;
        assert!(parse_frame("srv-kimi", idle, now, &mut state)
            .unwrap()
            .is_empty());
        assert!(!state.busy.contains("sess-c"));
        let completed = r#"{"type":"prompt.completed","session_id":"sess-c","payload":{"promptId":"p-1"}}"#;
        let events = parse_frame("srv-kimi", completed, now, &mut state).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentEventKind::Completed);

        // 仍有后台任务 → 重新判定为忙碌。
        let bg_start = r#"{"type":"background.task.started","session_id":"sess-c","payload":{}}"#;
        parse_frame("srv-kimi", bg_start, now, &mut state).unwrap();
        assert!(state.busy.contains("sess-c"));
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
