//! 远端 Kimi Web API 客户端：MCP 工具直连目标服务器自己的 Kimi Web 端点，
//! 在远端建会话、下发 prompt、取回紧凑状态摘要。凭据取自本地共享配置，
//! 不经过本机 mesh 端点（与 status.rs 同一条 HTTP 路径）。
//! 官方 API 标注 experimental/unstable，所有字段提取都是防御式的。

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, Instant};

use agentpocket_core::config::atomic_write;
use agentpocket_core::mesh_client as client;
use agentpocket_core::model::{Backend, ServerConfig};
use agentpocket_core::server_url::build_server_url;
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// 单次 HTTP 请求超时：tailnet 内明文直连，10s 足够。
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
/// excerpt 截断上限，保证工具输出总量远低于客户端可见上限。
const EXCERPT_MAX_CHARS: usize = 2000;
/// 关联记录里 purpose 保留的 prompt 前缀长度。
const PURPOSE_MAX_CHARS: usize = 200;

/// 目标白名单：env AGENTPOCKET_MCP_ALLOW 逗号分隔的 id/name。
/// 未设置或为空表示放行全部已配置的 Kimi 服务器（已确认的用户决策）。
#[derive(Clone, Debug, Default)]
pub struct AllowList {
    entries: Vec<String>,
}

impl AllowList {
    pub fn from_env() -> Self {
        Self::parse(std::env::var("AGENTPOCKET_MCP_ALLOW").ok().as_deref())
    }

    pub fn parse(raw: Option<&str>) -> Self {
        let entries = raw
            .unwrap_or("")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
        Self { entries }
    }

    pub fn allows(&self, server: &ServerConfig) -> bool {
        self.entries.is_empty()
            || self
                .entries
                .iter()
                .any(|e| e == &server.id || e == &server.name)
    }
}

/// 已解析并通过校验的调用目标。
#[derive(Clone, Debug)]
pub struct Target {
    pub server: ServerConfig,
}

/// 按 id 或 name 从本地配置解析目标；拒绝非 Kimi 后端与未放行目标。
/// 错误消息里列出可用目标，方便调用方（模型）自纠正。
pub fn resolve_target(
    servers: &[ServerConfig],
    name_or_id: &str,
    allow: &AllowList,
) -> Result<Target, String> {
    let available = || {
        let names: Vec<String> = servers
            .iter()
            .filter(|s| s.backend == Backend::Kimi && allow.allows(s))
            .map(|s| format!("{}({})", s.name, s.id))
            .collect();
        if names.is_empty() {
            "（无可用目标）".to_string()
        } else {
            format!("可用目标：{}", names.join("、"))
        }
    };
    let server = servers
        .iter()
        .find(|s| s.id == name_or_id || s.name == name_or_id)
        .ok_or_else(|| format!("未找到服务器「{name_or_id}」。{}", available()))?;
    if server.backend != Backend::Kimi {
        return Err(format!(
            "服务器「{}」是 dsh 后端，暂不支持远程派发（dsh 没有已证实的建会话接口）。{}",
            server.name,
            available()
        ));
    }
    if !allow.allows(server) {
        return Err(format!(
            "服务器「{}」不在白名单（AGENTPOCKET_MCP_ALLOW）内。{}",
            server.name,
            available()
        ));
    }
    Ok(Target {
        server: server.clone(),
    })
}

/// 请求头：Bearer 只在 token 非空时携带（与 GUI monitor 行为一致），POST 带 JSON 类型。
fn headers(server: &ServerConfig, json_body: bool) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    if !server.token.is_empty() {
        headers.push((
            "Authorization".to_string(),
            format!("Bearer {}", server.token),
        ));
    }
    if json_body {
        headers.push(("Content-Type".to_string(), "application/json".to_string()));
    }
    headers
}

fn header_refs(headers: &[(String, String)]) -> Vec<(&str, &str)> {
    headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect()
}

/// 信封校验：HTTP 状态 + code == 0。Kimi 服务端几乎恒回 200，业务结果看 code。
fn check_envelope(status: u16, body: &str) -> Result<Value, String> {
    if status != 200 {
        return Err(format!(
            "远端 HTTP {status}：{}",
            truncate_chars(body, 200)
        ));
    }
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("远端响应不是合法 JSON：{e}"))?;
    let code = value
        .get("code")
        .and_then(|c| c.as_i64())
        .ok_or_else(|| "远端响应缺少 code 字段".to_string())?;
    if code != 0 {
        let msg = value.get("msg").and_then(|m| m.as_str()).unwrap_or("");
        return Err(envelope_error(code, msg));
    }
    Ok(value.get("data").cloned().unwrap_or(Value::Null))
}

fn envelope_error(code: i64, msg: &str) -> String {
    match code {
        40101 => format!("远端未授权（{code}）：token 无效或缺失"),
        40401 => format!("会话不存在（{code}）：可能已被删除或不属于该服务"),
        40901 => format!("会话忙（{code}）：远端正在处理上一个请求"),
        _ => format!("远端返回错误（{code}）：{msg}"),
    }
}

/// 建会话：POST /api/v1/sessions，metadata.cwd 与 workspace_id 二者给一个即可。
pub fn create_session(target: &Target, cwd: &str) -> Result<String, String> {
    let headers = headers(&target.server, true);
    let body = json!({"metadata": {"cwd": cwd}}).to_string();
    let response = client::post(
        &target.server.host,
        target.server.port,
        "/api/v1/sessions",
        &header_refs(&headers),
        &body,
        HTTP_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    let data = check_envelope(response.status, &response.body)?;
    data.get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| "远端响应缺少 data.id".to_string())
}

/// 下发 prompt：字段是 content 内容块数组，不是 prompt/text。
///
/// **必须带 model**。实测：API 建出来的会话不会自动绑定模型——建会话时传
/// `agent_config.model` 也无效（会话里仍然是空），不给 model 的话回合会以
/// `[model.not_configured] Model not set` 秒失败。服务端也没有"默认模型"可查。
pub fn send_prompt(
    target: &Target,
    session: &str,
    prompt: &str,
    model: &str,
) -> Result<(), String> {
    let headers = headers(&target.server, true);
    let body = json!({
        "content": [{"type": "text", "text": prompt}],
        "model": model,
    })
    .to_string();
    let response = client::post(
        &target.server.host,
        target.server.port,
        &format!("/api/v1/sessions/{session}/prompts"),
        &header_refs(&headers),
        &body,
        HTTP_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    check_envelope(response.status, &response.body)?;
    Ok(())
}

/// 目标服务器可用的模型列表（`provider/name` 形式）。
pub fn list_models(target: &Target) -> Result<Vec<String>, String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        "/api/v1/models",
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    let data = check_envelope(response.status, &response.body)?;
    Ok(data
        .get("items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("model").and_then(|v| v.as_str()))
                .filter(|m| !m.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default())
}

/// 决定这次派发用哪个模型。优先级：调用方显式指定 > 本地默认（env）> 自动挑一个。
///
/// 服务端没有"默认模型"这个概念（`/api/v1/models` 里也没有默认标记），所以这里
/// 必须落在一个具体值上。自动挑时优先 Kimi Code 自己托管的 `kimi-code/*`，
/// 否则取列表第一个；选中的模型会回显在摘要里，失败原因也会带上，
/// 调用方据此可以改用显式 model 重试。
pub fn resolve_model(
    target: &Target,
    requested: Option<&str>,
    fallback: Option<&str>,
) -> Result<String, String> {
    if let Some(model) = requested.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(model.to_string());
    }
    if let Some(model) = fallback.map(str::trim).filter(|m| !m.is_empty()) {
        return Ok(model.to_string());
    }
    let models = list_models(target)?;
    models
        .iter()
        .find(|m| m.starts_with("kimi-code/"))
        .or_else(|| models.first())
        .cloned()
        .ok_or_else(|| {
            format!(
                "无法为「{}」确定模型：/api/v1/models 没有可用项，请显式传 model",
                target.server.name
            )
        })
}

/// 远端默认工作目录兜底：GET /api/v1/fs:home。字段名无文档，防御式提取；
/// 认不出来就报错，要求调用方显式传 cwd。
pub fn home_dir(target: &Target) -> Result<String, String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        "/api/v1/fs:home",
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    let data = check_envelope(response.status, &response.body)?;
    for key in ["home", "homeDir", "home_dir", "path", "cwd"] {
        if let Some(dir) = data.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Ok(dir.to_string());
        }
    }
    if let Some(dir) = data.as_str().filter(|s| !s.is_empty()) {
        return Ok(dir.to_string());
    }
    Err("无法确定远端默认工作目录，请显式传 cwd 参数".to_string())
}

/// 一次派发的紧凑摘要：状态 / 待交互 / 当前活动 / 最后输出节选 / 失败原因 / 会话链接。
#[derive(Clone, Debug)]
pub struct Summary {
    pub status: &'static str,
    pub pending_interaction: String,
    pub activity: Option<String>,
    pub excerpt: Option<String>,
    pub url: String,
    pub still_running: bool,
    /// 本次探测是否看到**真实活动**（busy、在等人、或已判定的终态）。
    /// 调用方累积它作为"任务真的跑过"的证据；"没证据所以报 running"不算证据，
    /// 否则下一跳就会把"还没开始"误判成"已完成"。
    pub active: bool,
    /// 失败原因，例如 `[model.not_configured] Model not set`。
    /// 只在 status 为 failed 时抓一次，取不到就是 None，不影响状态判定。
    pub reason: Option<String>,
}

/// 失败原因截断长度。
const FAILURE_REASON_MAX_CHARS: usize = 400;

/// 单会话探测结果（便宜的那一跳）。
struct Probe {
    busy: bool,
    pending_interaction: String,
    activity: Option<String>,
    /// 最近一个回合的结局。实测单会话 GET 上就有这个字段，`failed` 因此不必再去拉
    /// v2 列表；字段缺失时才退回列表接口。
    last_turn_reason: Option<String>,
}

fn probe_session(target: &Target, session: &str) -> Result<Probe, String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        &format!("/api/v1/sessions/{session}"),
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .map_err(|e| e.to_string())?;
    let data = check_envelope(response.status, &response.body)?;
    Ok(Probe {
        busy: data.get("busy").and_then(|v| v.as_bool()).unwrap_or(false),
        pending_interaction: data
            .get("pending_interaction")
            .and_then(|v| v.as_str())
            .unwrap_or("none")
            .to_string(),
        activity: ["/activity/label", "/activity/title", "/activity/status"]
            .iter()
            .find_map(|p| data.pointer(p).and_then(|v| v.as_str()))
            .map(String::from),
        last_turn_reason: data
            .get("last_turn_reason")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
    })
}

/// v2 列表里的 `activity.status`，作为终态判定的**退路**（只在单会话 GET 没给出
/// `last_turn_reason` 时才走到）。列表里找不到该会话就返回 None，不猜。
fn list_activity_status(target: &Target, session: &str) -> Option<String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        "/api/v2/sessions?page_size=50&sort=meta.updated_at_desc&meta.archived=all",
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .ok()?;
    let data = check_envelope(response.status, &response.body).ok()?;
    data.get("items")?
        .as_array()?
        .iter()
        .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(session))
        .and_then(|item| item.pointer("/activity/status").and_then(|v| v.as_str()))
        .map(String::from)
}

/// 失败原因：transcript 的 turn 条目里带 `error` / `endMessage`
/// （实测为 `[model.not_configured] Model not set`）。只在已判定失败时抓一次，
/// 抓不到就返回 None。
fn fetch_failure_reason(target: &Target, session: &str) -> Option<String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        &format!("/api/v1/sessions/{session}/transcript?agent_id=main"),
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .ok()?;
    let data = check_envelope(response.status, &response.body).ok()?;
    data.get("items")?
        .as_array()?
        .iter()
        .rev()
        .filter(|item| item.get("kind").and_then(|v| v.as_str()) == Some("turn"))
        .find_map(|turn| {
            ["error", "endMessage"]
                .iter()
                .find_map(|key| turn.get(*key).and_then(|v| v.as_str()))
                .filter(|text| !text.is_empty())
                .map(|text| truncate_chars(text, FAILURE_REASON_MAX_CHARS))
        })
}

/// 取指定会话摘要。
///
/// 分层探测，既不多发请求也不撒谎：
/// 1. 单会话 GET 拿 `busy` / `pending_interaction` / `last_turn_reason`——便宜，
///    忙碌或在等人时只发这一个请求。
/// 2. 不忙且单会话 GET 没给 `last_turn_reason`（版本差异）时，才退回 v2 列表读
///    `activity.status`。
/// 3. 仍无结论（idle）时，用"是否已有 assistant 回复"作为完成证据。
///
/// `observed_active` 是此前轮询累积的活动证据。**没有活动证据、也没有 assistant
/// 回复时绝不宣称完成**：刚下发 prompt 时远端还没领取，此刻 `busy` 同样是 false。
///
/// `failed` 反映该会话**最近一个回合**的结局。
pub fn session_summary(
    target: &Target,
    session: &str,
    observed_active: bool,
) -> Result<Summary, String> {
    let probe = probe_session(target, session)?;
    let url = build_server_url(&target.server, Some(session))
        .map(|u| u.to_string())
        .unwrap_or_default();

    // 在等人：直接给结论，需要人去处理
    if probe.pending_interaction == "approval" || probe.pending_interaction == "question" {
        let status = if probe.pending_interaction == "approval" {
            "awaiting_approval"
        } else {
            "awaiting_question"
        };
        return Ok(Summary {
            status,
            pending_interaction: probe.pending_interaction,
            activity: probe.activity,
            excerpt: fetch_excerpt(target, session),
            url,
            still_running: false,
            active: true,
            reason: None,
        });
    }

    // 在跑：只此一个请求，这是轮询期间的基本盘
    if probe.busy {
        return Ok(Summary {
            status: "running",
            pending_interaction: probe.pending_interaction,
            activity: probe.activity,
            excerpt: None,
            url,
            still_running: true,
            active: true,
            reason: None,
        });
    }

    // 不忙：先看便宜的 last_turn_reason，缺了才退回 v2 列表
    let terminal = match probe.last_turn_reason {
        Some(reason) => Some(reason),
        None => list_activity_status(target, session),
    };
    let status = match terminal.as_deref() {
        Some("failed") => "failed",
        Some("running") => "running",
        Some("approval") => "awaiting_approval",
        Some("question") => "awaiting_question",
        _ => {
            // idle，或什么都没拿到：只能靠证据判断是否真的出过结果
            let excerpt = fetch_excerpt(target, session);
            if excerpt.is_none() && !observed_active {
                // 刚下发 prompt、远端还没领取时 busy 也是 false。
                // 没有完成证据就不宣称完成，让调用方继续问。
                return Ok(Summary {
                    status: "running",
                    pending_interaction: probe.pending_interaction,
                    activity: probe.activity,
                    excerpt: None,
                    url,
                    still_running: true,
                    active: false,
                    reason: None,
                });
            }
            return Ok(Summary {
                status: "idle",
                pending_interaction: probe.pending_interaction,
                activity: probe.activity,
                excerpt,
                url,
                still_running: false,
                active: true,
                reason: None,
            });
        }
    };

    let still_running = status == "running";
    Ok(Summary {
        status,
        pending_interaction: match status {
            "awaiting_approval" => "approval".to_string(),
            "awaiting_question" => "question".to_string(),
            _ => probe.pending_interaction,
        },
        activity: probe.activity,
        // 运行中不抓 excerpt：轮询期间每 2s 拉一次 100 条消息纯属浪费
        excerpt: if still_running {
            None
        } else {
            fetch_excerpt(target, session)
        },
        url,
        still_running,
        active: true,
        reason: if status == "failed" {
            fetch_failure_reason(target, session)
        } else {
            None
        },
    })
}

/// excerpt 是 best-effort：/messages 的 item 字段名无文档，任何失败都静默降级为 None。
fn fetch_excerpt(target: &Target, session: &str) -> Option<String> {
    let headers = headers(&target.server, false);
    let response = client::get(
        &target.server.host,
        target.server.port,
        &format!("/api/v1/sessions/{session}/messages?page_size=100"),
        &header_refs(&headers),
        HTTP_TIMEOUT,
    )
    .ok()?;
    let data = check_envelope(response.status, &response.body).ok()?;
    let items = data.get("items").and_then(|v| v.as_array())?;
    last_assistant_text(items)
}

/// 取最后一条 assistant 消息的文本。依次尝试 content 字符串、
/// content 内容块数组、text 字段；都不认识就返回 None，绝不让工具因此失败。
fn last_assistant_text(items: &[Value]) -> Option<String> {
    for item in items.iter().rev() {
        let role = ["role", "sender", "author"]
            .iter()
            .find_map(|key| item.get(key))
            .and_then(|v| v.as_str());
        if role != Some("assistant") {
            continue;
        }
        if let Some(text) = message_text(item).filter(|t| !t.is_empty()) {
            return Some(truncate_chars(&text, EXCERPT_MAX_CHARS));
        }
    }
    None
}

fn message_text(item: &Value) -> Option<String> {
    if let Some(content) = item.get("content") {
        if let Some(text) = content.as_str() {
            return Some(text.to_string());
        }
        if let Some(parts) = content.as_array() {
            let texts: Vec<&str> = parts
                .iter()
                .filter(|p| p.get("type").and_then(|t| t.as_str()) == Some("text"))
                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                .collect();
            if !texts.is_empty() {
                return Some(texts.join("\n"));
            }
        }
    }
    item.get("text")
        .and_then(|v| v.as_str())
        .map(String::from)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        text.chars().take(max).collect()
    }
}

/// 本地关联记录：v1 只写不读，用于本地审计 agent 到底往外派了什么。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallRecord {
    pub server_id: String,
    pub server_name: String,
    pub session_id: String,
    pub purpose: String,
    pub created_at: String,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize)]
struct RecordFile {
    version: u32,
    #[serde(default)]
    calls: Vec<CallRecord>,
}

/// 组装一条记录：purpose 是截断后的 prompt，TTL 到期时间一并写入。
pub fn new_record(
    server: &ServerConfig,
    session_id: &str,
    prompt: &str,
    ttl: Duration,
) -> CallRecord {
    let now = Utc::now();
    let expires = now + chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::hours(6));
    CallRecord {
        server_id: server.id.clone(),
        server_name: server.name.clone(),
        session_id: session_id.to_string(),
        purpose: truncate_chars(prompt, PURPOSE_MAX_CHARS),
        created_at: now.to_rfc3339(),
        expires_at: expires.to_rfc3339(),
    }
}

/// 读-改-写追加一条记录，并顺手清掉 expiresAt 已过的条目。
/// 每个 kimi 会话会各自 spawn 自己的 MCP 进程，并发写真实存在：
/// 用 fs2 锁 + 5s 重试（与 core ConfigStore 同款模式），atomic_write 落盘（0600，
/// purpose 是截断后的 prompt，可能含敏感内容）。
pub fn record_call(path: &Path, record: CallRecord) -> Result<(), String> {
    let _lock = RecordLock::acquire(path)?;
    let mut calls = read_records(path);
    let now = Utc::now();
    calls.retain(|c| !expired(c, now));
    calls.push(record);
    let file = RecordFile { version: 1, calls };
    let bytes = serde_json::to_vec_pretty(&file).map_err(|e| e.to_string())?;
    atomic_write(path, &bytes).map_err(|e| format!("写入关联记录失败：{e}"))
}

fn read_records(path: &Path) -> Vec<CallRecord> {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RecordFile>(&bytes).ok())
        .map(|f| f.calls)
        .unwrap_or_default()
}

fn expired(record: &CallRecord, now: DateTime<Utc>) -> bool {
    // 解析不了 expiresAt 的条目保留，不误删
    DateTime::parse_from_rfc3339(&record.expires_at)
        .map(|t| t.with_timezone(&Utc) < now)
        .unwrap_or(false)
}

struct RecordLock {
    file: File,
}

impl RecordLock {
    fn acquire(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let lock_path = path.with_file_name(".remote-calls.lock");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(lock_path)
            .map_err(|e| e.to_string())?;
        let start = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if start.elapsed() >= Duration::from_secs(5) {
                        return Err("关联记录文件正被其他进程写入，等待超时".to_string());
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

impl Drop for RecordLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tiny_http::{Header, Response, Server};

    #[derive(Clone, Debug)]
    struct Seen {
        method: String,
        url: String,
        auth: Option<String>,
        body: String,
    }

    /// tiny_http mock：记录每个请求的方法/路径/头/body，交给闭包决定响应。
    fn spawn_mock(
        handler: impl Fn(&Seen) -> (u16, String) + Send + 'static,
    ) -> (u16, Arc<Mutex<Vec<Seen>>>) {
        let server = Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_in_thread = seen.clone();
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let record = Seen {
                    method: request.method().to_string(),
                    url: request.url().to_string(),
                    auth: request
                        .headers()
                        .iter()
                        .find(|h| h.field.equiv("Authorization"))
                        .map(|h| h.value.to_string()),
                    body,
                };
                seen_in_thread.lock().unwrap().push(record.clone());
                let (status, body) = handler(&record);
                let json = Header::from_bytes("Content-Type", "application/json").unwrap();
                let _ = request.respond(
                    Response::from_string(body)
                        .with_status_code(status)
                        .with_header(json),
                );
            }
        });
        (port, seen)
    }

    fn spawn_code_mock(code: i64) -> u16 {
        spawn_mock(move |_| {
            (
                200,
                json!({"code": code, "msg": "boom", "data": null}).to_string(),
            )
        })
        .0
    }

    fn target(port: u16) -> Target {
        Target {
            server: ServerConfig::new("k1", "pilab", "127.0.0.1", port, "tok", Backend::Kimi),
        }
    }

    #[test]
    fn resolve_target_rejects_unknown_dsh_and_blocked() {
        let servers = vec![
            ServerConfig::new("k1", "pilab", "127.0.0.1", 58627, "tok", Backend::Kimi),
            ServerConfig::new("d1", "dshbox", "127.0.0.1", 3080, "", Backend::Dsh),
        ];
        let all = AllowList::default();
        assert!(resolve_target(&servers, "pilab", &all).is_ok());
        assert!(resolve_target(&servers, "k1", &all).is_ok());

        let err = resolve_target(&servers, "nowhere", &all).unwrap_err();
        assert!(err.contains("pilab"), "错误应列出可用目标：{err}");

        let err = resolve_target(&servers, "dshbox", &all).unwrap_err();
        assert!(err.contains("dsh"), "应说明 dsh 不支持：{err}");

        let blocked = AllowList::parse(Some("other"));
        let err = resolve_target(&servers, "pilab", &blocked).unwrap_err();
        assert!(err.contains("白名单"), "应说明未放行：{err}");

        let listed = AllowList::parse(Some("k1"));
        assert!(resolve_target(&servers, "pilab", &listed).is_ok());
    }

    #[test]
    fn create_session_posts_metadata_cwd_with_bearer() {
        let (port, seen) = spawn_mock(|_| {
            (
                200,
                r#"{"code":0,"msg":"success","data":{"id":"session_abc"}}"#.to_string(),
            )
        });
        let id = create_session(&target(port), "/home/user/proj").unwrap();
        assert_eq!(id, "session_abc");

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let req = &seen[0];
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "/api/v1/sessions");
        assert_eq!(req.auth.as_deref(), Some("Bearer tok"));
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(body, json!({"metadata": {"cwd": "/home/user/proj"}}));
    }

    #[test]
    fn send_prompt_posts_content_parts_and_model() {
        let (port, seen) = spawn_mock(|_| {
            (200, r#"{"code":0,"msg":"success","data":{}}"#.to_string())
        });
        send_prompt(
            &target(port),
            "session_abc",
            "把测试修了",
            "cch/deepseek-v4-flash",
        )
        .unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let req = &seen[0];
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "/api/v1/sessions/session_abc/prompts");
        let body: Value = serde_json::from_str(&req.body).unwrap();
        // model 必带：API 建的会话不会自动绑模型，缺了会以 Model not set 秒失败
        assert_eq!(
            body,
            json!({"content": [{"type": "text", "text": "把测试修了"}],
                   "model": "cch/deepseek-v4-flash"})
        );
    }

    #[test]
    fn resolve_model_prefers_request_then_default_then_kimi_managed() {
        let (port, seen) = spawn_mock(|req| {
            if req.url == "/api/v1/models" {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"model":"openai/gpt-5.6-luna"},{"model":"kimi-code/kimi-for-coding"}]}}"#
                        .to_string(),
                )
            } else {
                (404, "{}".to_string())
            }
        });
        let t = target(port);

        // 调用方显式指定最优先，且不发网络请求
        assert_eq!(
            resolve_model(&t, Some("glm/glm-5.3"), Some("env/model")).unwrap(),
            "glm/glm-5.3"
        );
        assert!(seen.lock().unwrap().is_empty());
        // 其次本地默认
        assert_eq!(
            resolve_model(&t, None, Some("env/model")).unwrap(),
            "env/model"
        );
        assert!(seen.lock().unwrap().is_empty());
        // 都没有才去查列表，并优先 Kimi Code 托管模型（而不是列表第一个）
        assert_eq!(
            resolve_model(&t, None, None).unwrap(),
            "kimi-code/kimi-for-coding"
        );
    }

    #[test]
    fn resolve_model_errors_when_no_model_available() {
        let (port, _seen) = spawn_mock(|_| (200, r#"{"code":0,"data":{"items":[]}}"#.to_string()));
        let err = resolve_model(&target(port), None, None).unwrap_err();
        assert!(err.contains("model"), "{err}");
    }

    #[test]
    fn envelope_error_codes_map_to_chinese_messages() {
        let err = create_session(&target(spawn_code_mock(40901)), "/x").unwrap_err();
        assert!(err.contains("会话忙"), "{err}");
        let err = create_session(&target(spawn_code_mock(40101)), "/x").unwrap_err();
        assert!(err.contains("未授权"), "{err}");
        let err = send_prompt(&target(spawn_code_mock(40401)), "s", "p", "m/1").unwrap_err();
        assert!(err.contains("不存在"), "{err}");
        let err = create_session(&target(spawn_code_mock(50099)), "/x").unwrap_err();
        assert!(err.contains("50099"), "{err}");
    }

    /// 远端刚收到 prompt、还没领取时 busy 同样是 false：
    /// 没有活动证据、也没有 assistant 回复时**不能**报成已完成。
    #[test]
    fn session_summary_does_not_claim_done_without_evidence() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":false,"pending_interaction":"none"}}"#.to_string(),
                )
            } else if req.url.starts_with("/api/v2/sessions") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"id":"s1","activity":{"status":"idle"}}]}}"#
                        .to_string(),
                )
            } else {
                // messages：远端还一个字都没回
                (
                    200,
                    r#"{"code":0,"data":{"items":[],"has_more":false}}"#.to_string(),
                )
            }
        });
        let summary = session_summary(&target(port), "s1", false).unwrap();
        assert_eq!(summary.status, "running");
        assert!(summary.still_running);
        assert!(!summary.active, "没有活动证据就不算证据");
        assert_eq!(summary.excerpt, None);
    }

    /// 退路：单会话 GET 没给 last_turn_reason（版本差异）时，靠 v2 列表拿 failed。
    #[test]
    fn session_summary_reads_failed_from_v2_list() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":false,"pending_interaction":"none"}}"#.to_string(),
                )
            } else if req.url.starts_with("/api/v2/sessions") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"id":"s1","activity":{"status":"failed"}}]}}"#
                        .to_string(),
                )
            } else {
                (
                    200,
                    r#"{"code":0,"data":{"items":[],"has_more":false}}"#.to_string(),
                )
            }
        });
        let summary = session_summary(&target(port), "s1", false).unwrap();
        assert_eq!(summary.status, "failed");
        assert!(!summary.still_running);
        assert!(summary.active);
        // transcript 里没有 turn 条目时，原因取不到就是 None，不影响状态判定
        assert_eq!(summary.reason, None);
    }

    /// 便宜路径：单会话 GET 的 last_turn_reason 就够判失败，不必再拉 v2 列表；
    /// 失败原因从 transcript 的 error 带出来。
    #[test]
    fn session_summary_uses_last_turn_reason_without_v2() {
        let (port, seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":false,"pending_interaction":"none","last_turn_reason":"failed"}}"#
                        .to_string(),
                )
            } else if req.url.starts_with("/api/v1/sessions/s1/transcript") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"kind":"turn","state":"failed","error":"[model.not_configured] Model not set"}]}}"#
                        .to_string(),
                )
            } else {
                (404, "{}".to_string())
            }
        });
        let summary = session_summary(&target(port), "s1", false).unwrap();
        assert_eq!(summary.status, "failed");
        assert_eq!(
            summary.reason.as_deref(),
            Some("[model.not_configured] Model not set")
        );
        let seen = seen.lock().unwrap();
        assert!(
            !seen.iter().any(|r| r.url.starts_with("/api/v2/sessions")),
            "有 last_turn_reason 时不该再拉 v2 列表：{:?}",
            seen.iter().map(|r| r.url.as_str()).collect::<Vec<_>>()
        );
    }

    /// 失败原因的退路：transcript 只有 endMessage 时也取得到；抓不到则 None。
    #[test]
    fn failure_reason_falls_back_to_end_message() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url.starts_with("/api/v1/sessions/s1/transcript") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"kind":"turn","endMessage":"[x] boom"}]}}"#
                        .to_string(),
                )
            } else {
                (200, r#"{"code":0,"data":{"items":[]}}"#.to_string())
            }
        });
        assert_eq!(
            fetch_failure_reason(&target(port), "s1").as_deref(),
            Some("[x] boom")
        );

        let (port, _seen) = spawn_mock(|_| (404, "{}".to_string()));
        assert_eq!(fetch_failure_reason(&target(port), "s1"), None);
    }

    /// v2 说 idle 且确实已有 assistant 回复 → 真的完成，带上节选。
    #[test]
    fn session_summary_reports_idle_with_excerpt() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":false,"pending_interaction":"none"}}"#.to_string(),
                )
            } else if req.url.starts_with("/api/v2/sessions") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"id":"s1","activity":{"status":"idle"}}]}}"#
                        .to_string(),
                )
            } else {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"role":"assistant","content":"OK"}],"has_more":false}}"#
                        .to_string(),
                )
            }
        });
        let summary = session_summary(&target(port), "s1", false).unwrap();
        assert_eq!(summary.status, "idle");
        assert!(!summary.still_running);
        assert_eq!(summary.excerpt.as_deref(), Some("OK"));
        assert!(summary.active);
    }

    /// 见过活动、但正文抓不到（远端消息形状不认识）：按完成报，不再空转。
    #[test]
    fn session_summary_reports_idle_after_observed_activity() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":false,"pending_interaction":"none"}}"#.to_string(),
                )
            } else if req.url.starts_with("/api/v2/sessions") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"id":"s1","activity":{"status":"idle"}}]}}"#
                        .to_string(),
                )
            } else {
                (
                    200,
                    r#"{"code":0,"data":{"items":[],"has_more":false}}"#.to_string(),
                )
            }
        });
        let summary = session_summary(&target(port), "s1", true).unwrap();
        assert_eq!(summary.status, "idle");
        assert!(!summary.still_running);
        assert_eq!(summary.excerpt, None);
    }

    #[test]
    fn excerpt_extracts_last_assistant_text() {
        // content 为字符串：取最后一条 assistant
        let items = json!([
            {"role": "user", "content": "你好"},
            {"role": "assistant", "content": "旧回复"},
            {"role": "assistant", "content": "最新回复"}
        ]);
        assert_eq!(
            last_assistant_text(items.as_array().unwrap()).as_deref(),
            Some("最新回复")
        );

        // content 为内容块数组：拼接 text 块，跳过 thinking
        let items = json!([
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "..."},
                {"type": "text", "text": "块回复"}]}
        ]);
        assert_eq!(
            last_assistant_text(items.as_array().unwrap()).as_deref(),
            Some("块回复")
        );

        // text 字段
        let items = json!([{"role": "assistant", "text": "text 字段回复"}]);
        assert_eq!(
            last_assistant_text(items.as_array().unwrap()).as_deref(),
            Some("text 字段回复")
        );

        // sender 当 role
        let items = json!([{"sender": "assistant", "text": "via sender"}]);
        assert_eq!(
            last_assistant_text(items.as_array().unwrap()).as_deref(),
            Some("via sender")
        );

        // 无法识别的形状 → None，且不报错
        let items = json!([
            {"random": 1},
            {"role": "assistant", "content": {"weird": true}},
            42,
            "just a string"
        ]);
        assert_eq!(last_assistant_text(items.as_array().unwrap()), None);

        // 截断到 2000 字符
        let long = "长".repeat(5000);
        let items = json!([{"role": "assistant", "content": long}]);
        let excerpt = last_assistant_text(items.as_array().unwrap()).unwrap();
        assert_eq!(excerpt.chars().count(), EXCERPT_MAX_CHARS);
    }

    #[test]
    fn session_summary_maps_busy_session() {
        let (port, seen) = spawn_mock(|req| {
            if req.url.starts_with("/api/v1/sessions/s1/messages") {
                (
                    200,
                    r#"{"code":0,"data":{"items":[{"role":"assistant","content":"做完了"}],"has_more":false}}"#
                        .to_string(),
                )
            } else if req.url == "/api/v1/sessions/s1" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":true,"pending_interaction":"none","activity":{"status":"running","label":"编辑 main.rs"}}}"#
                        .to_string(),
                )
            } else {
                (404, "{}".to_string())
            }
        });
        let summary = session_summary(&target(port), "s1", false).unwrap();
        assert_eq!(summary.status, "running");
        assert!(summary.still_running);
        assert!(summary.active);
        assert_eq!(summary.pending_interaction, "none");
        assert_eq!(summary.activity.as_deref(), Some("编辑 main.rs"));
        // 运行中不算出结果，不抓 excerpt：mock 明明能返回正文，也必须拿到 None，
        // 否则轮询期间每 2s 就会多拉一次 100 条消息。
        assert_eq!(summary.excerpt, None);
        let seen = seen.lock().unwrap();
        // 忙碌这一跳只发一个请求：既不拉 messages，也不拉 v2 列表
        assert_eq!(
            seen.len(),
            1,
            "忙碌时只应有单会话探测这一个请求，实际：{:?}",
            seen.iter().map(|r| r.url.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(seen[0].url, "/api/v1/sessions/s1");
        assert_eq!(
            summary.url,
            format!("http://127.0.0.1:{port}/sessions/s1#token=tok")
        );
    }

    #[test]
    fn session_summary_maps_awaiting_approval() {
        let (port, _seen) = spawn_mock(|req| {
            if req.url == "/api/v1/sessions/s2" {
                (
                    200,
                    r#"{"code":0,"data":{"busy":true,"pending_interaction":"approval","activity":{"status":"running"}}}"#
                        .to_string(),
                )
            } else {
                (404, "{}".to_string())
            }
        });
        let summary = session_summary(&target(port), "s2", false).unwrap();
        assert_eq!(summary.status, "awaiting_approval");
        assert!(summary.active);
        assert!(!summary.still_running);
        // messages 端点 404 时 excerpt 静默降级为 None
        assert_eq!(summary.excerpt, None);
    }

    #[test]
    fn home_dir_extracts_defensively() {
        let (port, _seen) = spawn_mock(|_| {
            (
                200,
                r#"{"code":0,"data":{"home":"/home/remote","recent_workspaces":[]}}"#.to_string(),
            )
        });
        assert_eq!(home_dir(&target(port)).unwrap(), "/home/remote");

        let (port, _seen) = spawn_mock(|_| {
            (200, r#"{"code":0,"data":{"unexpected":true}}"#.to_string())
        });
        let err = home_dir(&target(port)).unwrap_err();
        assert!(err.contains("cwd"), "{err}");
    }

    #[test]
    fn record_call_appends_and_prunes_expired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote-calls.json");

        let stale = CallRecord {
            server_id: "k0".into(),
            server_name: "old".into(),
            session_id: "stale".into(),
            purpose: "旧任务".into(),
            created_at: (Utc::now() - chrono::Duration::hours(7)).to_rfc3339(),
            expires_at: (Utc::now() - chrono::Duration::hours(1)).to_rfc3339(),
        };
        let fresh = CallRecord {
            session_id: "fresh".into(),
            expires_at: (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            ..stale.clone()
        };
        let initial = json!({"version": 1, "calls": [stale, fresh]});
        fs::write(&path, serde_json::to_vec_pretty(&initial).unwrap()).unwrap();

        let server = ServerConfig::new("k1", "pilab", "127.0.0.1", 58627, "", Backend::Kimi);
        let record = new_record(&server, "session_new", "新任务", Duration::from_secs(6 * 3600));
        record_call(&path, record).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let file: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(file["version"], 1);
        let calls = file["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 2, "过期条目应被清掉：{text}");
        assert_eq!(calls[0]["sessionId"], "fresh");
        assert_eq!(calls[1]["sessionId"], "session_new");

        // 再写一次：条目继续累积而不是覆盖
        let record = new_record(&server, "session_newer", "又一个", Duration::from_secs(3600));
        record_call(&path, record).unwrap();
        let file: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["calls"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn record_call_starts_fresh_on_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remote-calls.json");
        fs::write(&path, b"not json").unwrap();
        let server = ServerConfig::new("k1", "pilab", "127.0.0.1", 58627, "", Backend::Kimi);
        record_call(&path, new_record(&server, "s", "p", Duration::from_secs(60))).unwrap();
        let file: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(file["calls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn new_record_truncates_purpose() {
        let server = ServerConfig::new("k1", "pilab", "127.0.0.1", 58627, "", Backend::Kimi);
        let record = new_record(&server, "s1", &"长".repeat(500), Duration::from_secs(3600));
        assert_eq!(record.purpose.chars().count(), PURPOSE_MAX_CHARS);
        assert!(DateTime::parse_from_rfc3339(&record.created_at).is_ok());
        assert!(DateTime::parse_from_rfc3339(&record.expires_at).is_ok());
    }
}
