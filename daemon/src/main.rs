mod kimi_config;
mod mesh;
mod ops;
mod paths;
mod status;
mod uninstall;
mod update;

use std::time::Duration;

use agentpocket_core::discovery;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agentpocket", about = "AgentPocket mesh 守护进程")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 前台运行：mesh 端点 + 自动更新（systemd 拉起此命令）
    Serve,
    /// 打印版本
    Version,
    /// 从 peer 拉取配置（默认合并）
    Pull {
        host: String,
        /// 用拉取结果替换本地服务器列表
        #[arg(long)]
        replace: bool,
        /// 只打印预览，不落盘
        #[arg(long)]
        dry_run: bool,
        /// 同步 ~/.kimi-code/config.toml（单文件覆盖，--replace 不适用）
        #[arg(long, conflicts_with = "replace")]
        kimi_config: bool,
    },
    /// 把本地配置推送给 peer
    Push {
        host: String,
        /// 同步 ~/.kimi-code/config.toml（单文件覆盖）
        #[arg(long)]
        kimi_config: bool,
    },
    /// 发现并列出 mesh peer
    Peers,
    /// 一次性探测已配置服务器状态
    Status,
    /// 手动检查并更新
    Update,
    /// 停止并移除服务与二进制（需 sudo，配置保留）
    Uninstall,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve => {
            let ctx = mesh::MeshContext {
                config_dir: paths::default_config_dir(),
                kimi_home: kimi_config::home_dir(),
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
        Command::Pull { host, replace, dry_run, kimi_config: kimi } => {
            if kimi {
                match ops::run_pull_kimi_config(&kimi_config::home_dir(), &host, dry_run) {
                    Ok(message) => println!("{message}"),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            let mode = if dry_run {
                ops::PullMode::DryRun
            } else if replace {
                ops::PullMode::Replace
            } else {
                ops::PullMode::Merge
            };
            match ops::run_pull(&paths::default_config_dir(), &host, mode) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Push { host, kimi_config: kimi } => {
            if kimi {
                match ops::run_push_kimi_config(&kimi_config::home_dir(), &host) {
                    Ok(message) => println!("{message}"),
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            match ops::run_push(&paths::default_config_dir(), &host) {
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
                eprintln!("未找到 tailscale CLI，仅探测手动 peer");
            }
            let peers = discovery::discover(
                &paths::default_config_dir(),
                tailscale.as_deref(),
                Duration::from_secs(3),
            );
            if peers.is_empty() {
                println!("未发现 AgentPocket peer");
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
                println!("尚未配置任何服务器（可 agentpocket pull <host> 先同步配置）");
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
        Command::Update => {
            match update::check_and_apply("https://api.github.com", Duration::from_secs(60)) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Uninstall => uninstall::run(),
    }
}
