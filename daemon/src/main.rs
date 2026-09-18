mod gui;
mod kimi;
mod kimi_web;
mod mcp;
mod mcp_setup;
mod mesh;
mod ops;
mod paths;
mod remote_agent;
mod status;
mod uninstall;
mod update;

use std::time::Duration;

use agentpocket_core::discovery;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agentpocket", about = "AgentPocket 节点守护进程")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum KimiWebAction {
    /// 生成并启动 Kimi Web，同时注册为本机 Agent 服务连接
    Enable {
        /// 监听端口
        #[arg(long, default_value_t = kimi_web::DEFAULT_PORT)]
        port: u16,
    },
    /// 重启 kimi web 服务（有活跃会话时拒绝，--force 强制）
    Restart {
        /// 忽略活跃会话，强制重启
        #[arg(long)]
        force: bool,
    },
    /// 停止并移除 kimi web 服务
    Disable,
    /// 查看状态
    Status,
}

#[derive(Subcommand)]
enum McpAction {
    /// 把本机 agentpocket 登记进 ~/.kimi-code/mcp.json（幂等；改动前备份为 .bak）
    Install,
    /// 查看是否已登记、登记的路径是否还在
    Status,
}

#[derive(Subcommand)]
enum Command {
    /// 前台运行：节点端点 + 自动更新（systemd 拉起此命令）
    Serve,
    /// 打印版本
    Version,
    /// 从节点获取 ~/.kimi-code/config.toml（覆盖本地，旧文件备份为 .bak）
    Pull {
        host: String,
        /// 只打印预览，不落盘
        #[arg(long)]
        dry_run: bool,
    },
    /// 把本地 ~/.kimi-code/config.toml 发送给节点
    Push { host: String },
    /// 发现并列出 AgentPocket 节点
    Peers,
    /// 一次性探测已配置 Agent 服务的状态
    Status,
    /// MCP 派发：不带动作时前台运行 stdio server（MCP 客户端拉起的就是它）
    Mcp {
        #[command(subcommand)]
        action: Option<McpAction>,
    },
    /// 手动检查并更新
    Update,
    /// 查询或安装/升级 Kimi Code CLI（省略 host 为本机，指定则为对应 mesh 节点）
    Kimi {
        /// 目标节点（IP 或 MagicDNS 名）
        host: Option<String>,
        /// 执行安装/升级（缺省只查询版本）
        #[arg(long)]
        upgrade: bool,
    },
    /// kimi web 服务管理（用户级 systemd 单元，监听 0.0.0.0、tailnet 内免 token）
    KimiWeb {
        #[command(subcommand)]
        action: KimiWebAction,
    },
    /// 停止并移除服务与二进制（需 sudo，配置保留）
    Uninstall,
    /// 输出 shell 自动补全脚本（bash/zsh/fish/elvish/powershell）
    Completions { shell: clap_complete::Shell },
}

fn main() {
    // 裸执行（不带任何子命令）：桌面上就是"打开应用"；节点上没装 GUI，退回打印帮助
    if std::env::args_os().len() == 1 {
        if let Err(e) = gui::launch_or_help() {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }

    let cli = Cli::parse();
    match cli.command {
        Command::Serve => {
            let ctx = mesh::MeshContext {
                config_dir: paths::default_config_dir(),
                kimi_home: agentpocket_core::kimi_config::home_dir(),
                version: env!("CARGO_PKG_VERSION"),
                hostname: agentpocket_core::host::hostname(),
            };
            match mesh::start(ctx, mesh::MESH_PORT) {
                Ok(handle) => {
                    println!(
                        "[mesh] 监听 0.0.0.0:{}（仅 tailnet/回环可达），配置目录 {}",
                        mesh::MESH_PORT,
                        paths::default_config_dir().display()
                    );
                    update::spawn_update_loop();
                    handle.wait();
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Version => println!("{}", env!("CARGO_PKG_VERSION")),
        Command::Pull { host, dry_run } => {
            let home = agentpocket_core::kimi_config::home_dir();
            match ops::run_pull(&home, &host, dry_run) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Push { host } => {
            let home = agentpocket_core::kimi_config::home_dir();
            match ops::run_push(&home, &host) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Peers => {
            let tailscale = discovery::find_tailscale_binary();
            if tailscale.is_none() {
                eprintln!("未找到 tailscale CLI，仅探测手动添加的节点");
            }
            let peers = discovery::discover(
                &paths::default_config_dir(),
                tailscale.as_deref(),
                Duration::from_secs(3),
            );
            if peers.is_empty() {
                println!("未发现 AgentPocket 节点");
            }
            for peer in peers {
                println!(
                    "{}  {}  {}",
                    peer.name,
                    peer.host,
                    peer.version.as_deref().unwrap_or("-")
                );
            }
        }
        Command::Status => {
            let config_dir = paths::default_config_dir();
            let outcome = match agentpocket_core::config::ConfigStore::new(config_dir).load() {
                Ok(outcome) => outcome,
                Err(e) => {
                    eprintln!("读取配置失败：{e}");
                    std::process::exit(1);
                }
            };
            if outcome.config.servers.is_empty() {
                println!("尚未配置任何 Agent 服务");
            }
            // 并发探测全部已配置服务器，每个线程一个 5s 超时
            std::thread::scope(|scope| {
                let handles: Vec<_> = outcome
                    .config
                    .servers
                    .iter()
                    .map(|server| {
                        scope.spawn(move || {
                            status::probe_server(server, Duration::from_secs(5))
                        })
                    })
                    .collect();
                for handle in handles {
                    let probe = handle.join().expect("probe thread");
                    let backend = match probe.backend {
                        agentpocket_core::model::Backend::Kimi => "kimi",
                        agentpocket_core::model::Backend::Dsh => "dsh",
                        agentpocket_core::model::Backend::Opencode => "opencode",
                    };
                    if probe.online {
                        println!(
                            "{}  {}  在线 {}  {} 个活跃会话",
                            probe.name,
                            backend,
                            probe.version.as_deref().unwrap_or("-"),
                            probe.busy
                        );
                    } else {
                        println!(
                            "{}  {}  离线（{}）",
                            probe.name,
                            backend,
                            probe.error.as_deref().unwrap_or("未知错误")
                        );
                    }
                }
            });
        }
        Command::Mcp { action } => match action {
            // 不带动作：被 MCP 客户端拉起的 stdio server（常驻，协议走 stdout、日志走 stderr）
            None => {
                if let Err(e) = mcp::run() {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            Some(McpAction::Install) => {
                let cli = std::env::current_exe().expect("无法解析自身路径");
                match mcp_setup::install(&cli) {
                    Ok(msg) => println!("{msg}"),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
            Some(McpAction::Status) => {
                let cli = std::env::current_exe().expect("无法解析自身路径");
                match mcp_setup::status(&cli) {
                    Ok(msg) => println!("{msg}"),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            }
        },
        Command::Update => {
            match update::check_and_apply("https://api.github.com", Duration::from_secs(60)) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Kimi { host, upgrade } => {
            let home = agentpocket_core::kimi_config::home_dir();
            let result = match (host, upgrade) {
                (None, false) => ops::kimi_local_status(&home),
                (None, true) => ops::kimi_local_upgrade(&home),
                (Some(host), false) => ops::kimi_remote_status(&host),
                (Some(host), true) => ops::kimi_remote_upgrade(&host),
            };
            match result {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::KimiWeb { action } => {
            let home = agentpocket_core::kimi_config::home_dir();
            let result = match action {
                KimiWebAction::Enable { port } => {
                    ops::kimi_web_enable(&paths::default_config_dir(), &home, port)
                }
                KimiWebAction::Restart { force } => ops::kimi_web_restart(&home, force),
                KimiWebAction::Disable => ops::kimi_web_disable(&home),
                KimiWebAction::Status => ops::kimi_web_status(&home),
            };
            match result {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Uninstall => uninstall::run(),
        Command::Completions { shell } => {
            use clap::CommandFactory;
            // 补全按安装后的命令名 agentpocket 生成，而非当前可执行文件名
            clap_complete::generate(shell, &mut Cli::command(), "agentpocket", &mut std::io::stdout());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn completions_script_covers_command() {
        let mut buf = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Bash,
            &mut Cli::command(),
            "agentpocket",
            &mut buf,
        );
        let script = String::from_utf8(buf).unwrap();
        assert!(script.contains("agentpocket"));
        assert!(script.contains("pull"));
    }
}
