//! stdio MCP server：把"调用其他服务器上的 Kimi agent"作为三个工具暴露给
//! MCP 客户端（Kimi Code）。协议是 newline-delimited JSON-RPC（一行一个对象，
//! 无 Content-Length 框架，无批量），stdout 只写协议消息，日志一律走 stderr
//! （客户端保留 4KB stderr 尾部用于诊断）。
//!
//! 明确不做的能力（防止后续被顺手加上，见计划文档）：
//! - 不做 list_sessions（那才是状态洪流入口）；
//! - 不代答远端的审批/提问，不取消远端任务；
//! - 不订阅/转发 transcript；
//! - 不支持 dsh 后端。

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use agentpocket_core::config::ConfigStore;
use agentpocket_core::model::{Backend, ServerConfig};
use serde_json::{json, Value};

use crate::paths;
use crate::remote_agent::{self, AllowList};

/// Kimi Code 内嵌 MCP SDK 严格校验的协议版本集合；回复集合外的字符串会被直接断开。
const SUPPORTED_VERSIONS: &[&str] = &[
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];
const LATEST_VERSION: &str = "2025-11-25";
/// ask_remote 默认等待秒数与硬上限：都留在客户端 60s 工具超时之内。
/// 上限可用 AGENTPOCKET_MCP_MAX_WAIT_SECONDS 抬高，但需与 mcp.json 的
/// toolTimeoutMs 配对使用才真等更久。
const DEFAULT_WAIT_SECONDS: u64 = 30;
const DEFAULT_MAX_WAIT_SECONDS: u64 = 45;
/// check_remote 默认不等待，立即返回快照。
const DEFAULT_CHECK_WAIT_SECONDS: u64 = 0;
/// 关联记录默认 TTL（小时），可用 AGENTPOCKET_MCP_RECORD_TTL_HOURS 调整。
const DEFAULT_RECORD_TTL_HOURS: u64 = 6;
/// 轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_secs(2);

struct Ctx {
    config_dir: PathBuf,
    record_path: PathBuf,
    allow: AllowList,
    max_wait: u64,
    record_ttl: Duration,
    /// 默认模型（env `AGENTPOCKET_MCP_MODEL`）。未设时按目标服务器的
    /// `/api/v1/models` 自动挑一个——API 建的会话不会自动绑模型，必须给值。
    default_model: Option<String>,
    /// notifications/cancelled 置位；worker 在轮询间隙检查后提前收尾。
    cancel: Arc<AtomicBool>,
}

impl Ctx {
    fn from_env() -> Self {
        let max_wait = std::env::var("AGENTPOCKET_MCP_MAX_WAIT_SECONDS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_MAX_WAIT_SECONDS);
        let ttl_hours = std::env::var("AGENTPOCKET_MCP_RECORD_TTL_HOURS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_RECORD_TTL_HOURS);
        let default_model = std::env::var("AGENTPOCKET_MCP_MODEL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        Self {
            config_dir: paths::default_config_dir(),
            record_path: paths::remote_calls_file(),
            allow: AllowList::from_env(),
            max_wait,
            record_ttl: Duration::from_secs(ttl_hours * 3600),
            default_model,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn servers(&self) -> Result<Vec<ServerConfig>, String> {
        let outcome = ConfigStore::new(self.config_dir.clone())
            .load()
            .map_err(|e| format!("读取配置失败：{e}"))?;
        Ok(outcome.config.servers)
    }
}

/// 长驻循环：reader（主线程）立即应答 ping / initialize / tools/list 与协议层错误，
/// tools/call 派给单个 worker 线程（同一时刻只跑一个），保证 5s liveness ping
/// 在工具调用阻塞几十秒时也能被应答。stdout 用 Mutex 串行化，一行一个 JSON。
pub fn run() -> Result<(), String> {
    // 人类在终端里直接敲 `agentpocket mcp` 只会看到一个"卡住"的进程——它其实在等
    // 协议消息。提示走 stderr，stdout 保持纯净（照旧可以手工喂 JSON-RPC 调试）。
    {
        use std::io::IsTerminal;
        if std::io::stdin().is_terminal() {
            eprintln!("[mcp] 这是 stdio MCP server，由 MCP 客户端（如 Kimi Code）拉起；直接运行会一直等 stdin。");
            eprintln!("[mcp] 要接入本机 Kimi Code：agentpocket mcp install");
        }
    }
    let ctx = Arc::new(Ctx::from_env());
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let (tx, rx) = mpsc::channel::<(Value, String, Value)>();
    let worker = {
        let ctx = ctx.clone();
        let stdout = stdout.clone();
        std::thread::spawn(move || {
            while let Ok((id, name, args)) = rx.recv() {
                let response = match dispatch_tool(&name, &args, &ctx) {
                    Outcome::Result(result) => {
                        json!({"jsonrpc": "2.0", "id": id, "result": result})
                    }
                    Outcome::Error(code, message) => error_response(id, code, message),
                };
                write_line(&stdout, &response);
            }
        })
    };

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("[mcp] 读取 stdin 失败：{e}");
                break;
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("[mcp] 无法解析的输入行：{e}");
                continue;
            }
        };
        if msg.is_array() {
            eprintln!("[mcp] 忽略批量请求（不支持）");
            continue;
        }
        match route(&msg) {
            Route::Ignore => {}
            Route::Cancel => ctx.cancel.store(true, Ordering::SeqCst),
            Route::Reply(response) => write_line(&stdout, &response),
            Route::ToolCall(id, name, args) => {
                if tx.send((id, name, args)).is_err() {
                    break;
                }
            }
        }
    }
    // stdin 关闭后让 worker 把已入队的调用跑完再退出（channel 断开后 recv 返回 Err）
    drop(tx);
    let _ = worker.join();
    Ok(())
}

/// 一条输入消息的分类结果。
enum Route {
    /// 通知或无法处理的消息：不回。
    Ignore,
    /// notifications/cancelled：置取消标志，不回。
    Cancel,
    /// 立即回复（initialize / ping / tools/list / 协议层错误）。
    Reply(Value),
    /// 派给 worker 线程的工具调用：(id, 工具名, arguments)。
    ToolCall(Value, String, Value),
}

fn route(msg: &Value) -> Route {
    let method = msg.get("method").and_then(|m| m.as_str());
    let id = msg.get("id").cloned();
    let Some(method) = method else {
        return match id {
            Some(id) => Route::Reply(error_response(id, -32600, "缺少 method 字段")),
            None => Route::Ignore,
        };
    };
    let Some(id) = id else {
        // 通知一律不回；notifications/initialized 忽略，cancelled 顺带置取消标志
        return if method == "notifications/cancelled" {
            Route::Cancel
        } else {
            Route::Ignore
        };
    };
    match method {
        "initialize" => {
            let requested = msg
                .pointer("/params/protocolVersion")
                .and_then(|v| v.as_str());
            Route::Reply(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": initialize_result(requested),
            }))
        }
        "ping" => Route::Reply(json!({"jsonrpc": "2.0", "id": id, "result": {}})),
        "tools/list" => Route::Reply(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": tools_list_result(),
        })),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(|v| v.as_str());
            match name {
                Some(name) => {
                    let args = msg
                        .pointer("/params/arguments")
                        .cloned()
                        .filter(|a| a.is_object())
                        .unwrap_or_else(|| json!({}));
                    Route::ToolCall(id, name.to_string(), args)
                }
                None => Route::Reply(error_response(id, -32602, "tools/call 缺少 params.name")),
            }
        }
        _ => Route::Reply(error_response(id, -32601, format!("未知方法：{method}"))),
    }
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// 客户端请求的版本在支持集合内就回显，否则回最新版。
fn initialize_result(requested: Option<&str>) -> Value {
    let version = match requested {
        Some(v) if SUPPORTED_VERSIONS.contains(&v) => v,
        _ => LATEST_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "agentpocket", "version": env!("CARGO_PKG_VERSION")},
    })
}

fn tools_list_result() -> Value {
    // 每个 inputSchema 的 type 必须字面量是 "object"，否则客户端校验失败、server 起不来
    json!({"tools": [
        {
            "name": "list_targets",
            "description": "列出本地配置里可远程派发的 Agent 服务器（不带 server 参数时只读本地配置、不访问网络）。带 server 时额外返回该服务器可用的模型列表，用来确定 ask_remote 的 server / model 参数。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server": {"type": "string", "description": "可选：额外查这个服务器的可用模型（会发一次网络请求）"}
                },
                "additionalProperties": false
            }
        },
        {
            "name": "ask_remote",
            "description": "在目标服务器的 Kimi 上创建会话并下发 prompt。注意 API 建的会话不会自动绑定模型，model 省略时会在目标服务器可用模型里自动挑一个（选中值回显在返回的 model 字段）。在 wait_seconds 内完成则返回结果摘要；仍在运行则返回 still_running=true 和 session_id，之后用 check_remote 继续查询。若返回 awaiting_approval/awaiting_question，需要人打开返回的 url 处理。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server": {"type": "string", "description": "目标服务器的 id 或 name（见 list_targets）"},
                    "prompt": {"type": "string", "description": "下发给远端 agent 的任务描述"},
                    "model": {"type": "string", "description": "可选：远端使用的模型，形如 provider/name（见 list_targets 带 server）。省略则自动挑一个"},
                    "cwd": {"type": "string", "description": "远端工作目录；省略时取远端 home 目录"},
                    "wait_seconds": {"type": "integer", "description": "等待远端完成的秒数，默认 30，有硬上限（默认 45）"}
                },
                "required": ["server", "prompt"],
                "additionalProperties": false
            }
        },
        {
            "name": "check_remote",
            "description": "查询 ask_remote 派发的某个会话的最新状态与输出摘要，返回同一种摘要结构。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "server": {"type": "string", "description": "目标服务器的 id 或 name"},
                    "session_id": {"type": "string", "description": "ask_remote 返回的 session_id"},
                    "wait_seconds": {"type": "integer", "description": "可选的短等待秒数，默认 0（立即返回快照）"}
                },
                "required": ["server", "session_id"],
                "additionalProperties": false
            }
        }
    ]})
}

enum Outcome {
    Result(Value),
    Error(i64, String),
}

fn dispatch_tool(name: &str, args: &Value, ctx: &Ctx) -> Outcome {
    let result = match name {
        "list_targets" => tool_list_targets(ctx, args),
        "ask_remote" => tool_ask_remote(ctx, args),
        "check_remote" => tool_check_remote(ctx, args),
        _ => return Outcome::Error(-32602, format!("未知工具：{name}")),
    };
    match result {
        Ok(text) => Outcome::Result(json!({
            "content": [{"type": "text", "text": text}],
            "isError": false,
        })),
        // 工具失败一律放在 result 里（isError: true）；
        // JSON-RPC error 只留给方法/工具不存在，避免客户端走重连重试路径
        Err(text) => Outcome::Result(json!({
            "content": [{"type": "text", "text": text}],
            "isError": true,
        })),
    }
}

/// 列出静态目标（不是会话状态）。不带 server 参数时只读本地配置、不发网络请求；
/// 带了 server 就额外查一次该服务器的可用模型，供调用方挑 model。
fn tool_list_targets(ctx: &Ctx, args: &Value) -> Result<String, String> {
    let servers = ctx.servers()?;
    let wanted = args
        .get("server")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut models: Option<Vec<String>> = None;
    if let Some(name) = wanted {
        let target = remote_agent::resolve_target(&servers, name, &ctx.allow)?;
        models = Some(remote_agent::list_models(&target)?);
    }

    let targets: Vec<Value> = servers
        .iter()
        .map(|s| {
            json!({
                "id": s.id.as_str(),
                "name": s.name.as_str(),
                "host": s.host.as_str(),
                "port": s.port,
                "backend": match s.backend {
                    Backend::Kimi => "kimi",
                    Backend::Dsh => "dsh",
                },
                "dispatchable": s.backend == Backend::Kimi && ctx.allow.allows(s),
            })
        })
        .collect();

    let mut out = json!({"targets": targets});
    if let Some(models) = models {
        out["models"] = json!(models);
    }
    Ok(out.to_string())
}

fn tool_ask_remote(ctx: &Ctx, args: &Value) -> Result<String, String> {
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 server（目标服务器的 id 或 name，见 list_targets）".to_string())?;
    let prompt = args
        .get("prompt")
        .and_then(|v| v.as_str())
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "缺少参数 prompt（下发给远端 agent 的任务描述）".to_string())?;
    let servers = ctx.servers()?;
    let target = remote_agent::resolve_target(&servers, server, &ctx.allow)?;
    // 先定模型再建会话：模型定不下来就别在远端留下一个跑不了的孤儿会话
    let model = remote_agent::resolve_model(
        &target,
        args.get("model").and_then(|v| v.as_str()),
        ctx.default_model.as_deref(),
    )?;
    let cwd = match args.get("cwd").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        Some(cwd) => cwd.to_string(),
        None => remote_agent::home_dir(&target)?,
    };
    let session_id = remote_agent::create_session(&target, &cwd)?;
    // 建会话成功后立刻落本地审计记录；写失败只记 stderr，不阻断派发
    let record = remote_agent::new_record(&target.server, &session_id, prompt, ctx.record_ttl);
    if let Err(e) = remote_agent::record_call(&ctx.record_path, record) {
        eprintln!("[mcp] 写入关联记录失败：{e}");
    }
    remote_agent::send_prompt(&target, &session_id, prompt, &model)?;
    let wait = requested_wait(args, ctx.max_wait, DEFAULT_WAIT_SECONDS);
    let summary = poll_summary(ctx, &target, &session_id, wait)?;
    let mut out = summary_json(&target, &session_id, &summary);
    // 回显用了哪个模型：失败时调用方据此改成显式 model 重试
    out["model"] = json!(model);
    Ok(out.to_string())
}

fn tool_check_remote(ctx: &Ctx, args: &Value) -> Result<String, String> {
    let server = args
        .get("server")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少参数 server（目标服务器的 id 或 name，见 list_targets）".to_string())?;
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "缺少参数 session_id（ask_remote 返回的会话 id）".to_string())?;
    let servers = ctx.servers()?;
    let target = remote_agent::resolve_target(&servers, server, &ctx.allow)?;
    let wait = requested_wait(args, ctx.max_wait, DEFAULT_CHECK_WAIT_SECONDS);
    let summary = poll_summary(ctx, &target, session_id, wait)?;
    Ok(summary_json(&target, session_id, &summary).to_string())
}

fn requested_wait(args: &Value, max_wait: u64, default: u64) -> u64 {
    args.get("wait_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
        .min(max_wait)
}

/// 在时间预算内轮询；仍在跑就按时返回 still_running=true，不阻塞等完
/// （客户端工具调用默认 60s 超时，硬上限默认 45s 刻意留在其内）。
/// 轮询间隙检查取消标志：notifications/cancelled 能让等待提前收尾。
///
/// 累积"活动证据"传给下一跳：远端刚收到 prompt、还没领取时 busy 同样是 false，
/// 只看 busy 会把"还没开始"误判成"已完成"。
fn poll_summary(
    ctx: &Ctx,
    target: &remote_agent::Target,
    session_id: &str,
    wait_seconds: u64,
) -> Result<remote_agent::Summary, String> {
    ctx.cancel.store(false, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(wait_seconds);
    let mut observed_active = false;
    loop {
        let summary = remote_agent::session_summary(target, session_id, observed_active)?;
        // 只有真实活动才算证据；"没证据所以报 running"不算，
        // 否则下一跳就会拿它当依据、把"还没开始"判成"已完成"。
        observed_active |= summary.active;
        if !summary.still_running
            || Instant::now() >= deadline
            || ctx.cancel.swap(false, Ordering::SeqCst)
        {
            return Ok(summary);
        }
        std::thread::sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
}

/// 紧凑摘要：总量控制在 4KB 以内（excerpt 已在远端截到 2000 字符）。
fn summary_json(
    target: &remote_agent::Target,
    session_id: &str,
    summary: &remote_agent::Summary,
) -> Value {
    json!({
        "server": target.server.name.as_str(),
        "session_id": session_id,
        "status": summary.status,
        "pending_interaction": summary.pending_interaction.as_str(),
        "activity": summary.activity.as_deref(),
        "excerpt": summary.excerpt.as_deref(),
        "reason": summary.reason.as_deref(),
        "url": summary.url.as_str(),
        "still_running": summary.still_running,
    })
}

/// 序列化为单行：serde_json 会转义控制字符，输出保证不含内嵌换行，stdout 保持纯净。
fn serialize_line(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","error":{"code":-32603,"message":"序列化失败"}}"#.to_string()
    })
}

fn write_line(stdout: &Mutex<std::io::Stdout>, value: &Value) {
    let line = serialize_line(value);
    let mut out = stdout.lock().unwrap();
    let _ = out.write_all(line.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(dir: &std::path::Path, allow: AllowList) -> Ctx {
        let config = json!({
            "schema": 1,
            "servers": [
                {"id": "k1", "name": "pilab", "host": "127.0.0.1", "port": 58627, "token": "tok", "backend": "kimi"},
                {"id": "d1", "name": "dshbox", "host": "127.0.0.1", "port": 3080, "backend": "dsh"}
            ]
        });
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("config.json"), config.to_string()).unwrap();
        Ctx {
            config_dir: dir.to_path_buf(),
            record_path: dir.join("remote-calls.json"),
            allow,
            max_wait: DEFAULT_MAX_WAIT_SECONDS,
            record_ttl: Duration::from_secs(3600),
            default_model: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn initialize_returns_supported_version_and_tools_capability() {
        // 客户端请求的版本在集合内：回显
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"1"}}});
        let Route::Reply(response) = route(&msg) else {
            panic!("initialize 必须立即回复")
        };
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"]["tools"].is_object());
        assert_eq!(response["result"]["serverInfo"]["name"], "agentpocket");

        // 集合外的版本：回最新版，且必须在客户端接受集合内
        let result = initialize_result(Some("1999-01-01"));
        let version = result["protocolVersion"].as_str().unwrap();
        assert_eq!(version, LATEST_VERSION);
        assert!(SUPPORTED_VERSIONS.contains(&version));
        let result = initialize_result(None);
        assert!(SUPPORTED_VERSIONS.contains(&result["protocolVersion"].as_str().unwrap()));
    }

    #[test]
    fn ping_returns_empty_object() {
        let msg = json!({"jsonrpc":"2.0","id":7,"method":"ping"});
        let Route::Reply(response) = route(&msg) else {
            panic!("ping 必须立即回复")
        };
        assert_eq!(response["id"], 7);
        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn notifications_are_never_replied() {
        let initialized = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(matches!(route(&initialized), Route::Ignore));
        let cancelled = json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1,"reason":"user"}});
        assert!(matches!(route(&cancelled), Route::Cancel));
        let unknown = json!({"jsonrpc":"2.0","method":"notifications/whatever"});
        assert!(matches!(route(&unknown), Route::Ignore));
    }

    #[test]
    fn tools_list_has_three_tools_with_object_schemas() {
        let msg = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
        let Route::Reply(response) = route(&msg) else {
            panic!("tools/list 必须立即回复")
        };
        let tools = response["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_targets", "ask_remote", "check_remote"]);
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn unknown_method_is_jsonrpc_error() {
        let msg = json!({"jsonrpc":"2.0","id":3,"method":"resources/list"});
        let Route::Reply(response) = route(&msg) else {
            panic!("未知方法应回 JSON-RPC error")
        };
        assert_eq!(response["id"], 3);
        assert_eq!(response["error"]["code"], -32601);
    }

    #[test]
    fn unknown_tool_is_jsonrpc_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path(), AllowList::default());
        let Outcome::Error(code, message) = dispatch_tool("restart_server", &json!({}), &ctx)
        else {
            panic!("未知工具应回 JSON-RPC error")
        };
        assert_eq!(code, -32602);
        assert!(message.contains("restart_server"));
    }

    #[test]
    fn unknown_target_is_tool_error_listing_valid_targets() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path(), AllowList::default());
        let Outcome::Result(result) =
            dispatch_tool("ask_remote", &json!({"server": "nowhere", "prompt": "hi"}), &ctx)
        else {
            panic!("工具失败应放在 result 里")
        };
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("pilab"), "错误应列出可用目标：{text}");
    }

    #[test]
    fn non_allowlisted_target_is_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path(), AllowList::parse(Some("other")));
        let Outcome::Result(result) =
            dispatch_tool("ask_remote", &json!({"server": "pilab", "prompt": "hi"}), &ctx)
        else {
            panic!("工具失败应放在 result 里")
        };
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("白名单"), "{text}");
    }

    #[test]
    fn list_targets_reads_config_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = test_ctx(dir.path(), AllowList::default());
        // 不带 server 参数：只读本地配置、不发网络请求（配置里的 127.0.0.1:58627
        // 并没有服务在听，真发请求就会失败）
        let text = tool_list_targets(&ctx, &json!({})).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        let targets = value["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0]["id"], "k1");
        assert_eq!(targets[0]["backend"], "kimi");
        assert_eq!(targets[0]["dispatchable"], true);
        assert_eq!(targets[1]["backend"], "dsh");
        assert_eq!(targets[1]["dispatchable"], false);
        assert!(value.get("models").is_none(), "不传 server 时不该有 models");
    }

    #[test]
    fn wait_seconds_is_capped() {
        assert_eq!(requested_wait(&json!({}), 45, 30), 30);
        assert_eq!(requested_wait(&json!({"wait_seconds": 10}), 45, 30), 10);
        assert_eq!(requested_wait(&json!({"wait_seconds": 999}), 45, 30), 45);
        assert_eq!(requested_wait(&json!({"wait_seconds": 240}), 240, 30), 240);
    }

    #[test]
    fn serialize_line_emits_single_line() {
        let value = json!({"text": "第一行\n第二行\t\0", "nested": {"a": [1, 2, 3]}});
        let line = serialize_line(&value);
        assert!(!line.contains('\n'), "输出必须是单行：{line:?}");
        assert!(!line.contains('\r'));
        assert_eq!(serde_json::from_str::<Value>(&line).unwrap(), value);
    }
}
