mod client;
mod discovery;
mod mesh;
mod ops;
mod paths;

use std::time::Duration;

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
    },
    /// 把本地配置推送给 peer
    Push { host: String },
    /// 发现并列出 mesh peer
    Peers,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve => {
            let ctx = mesh::MeshContext {
                config_dir: paths::default_config_dir(),
                version: env!("CARGO_PKG_VERSION"),
                hostname: paths::hostname(),
            };
            match mesh::start(ctx, mesh::MESH_PORT) {
                Ok(handle) => {
                    println!(
                        "[mesh] 监听 0.0.0.0:{}（仅 tailnet/回环可达），配置目录 {}",
                        mesh::MESH_PORT,
                        paths::default_config_dir().display()
                    );
                    handle.wait();
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Version => println!("{}", env!("CARGO_PKG_VERSION")),
        Command::Pull { host, replace, dry_run } => {
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
        Command::Push { host } => {
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
    }
}
