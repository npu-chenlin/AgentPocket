mod client;
mod mesh;
mod ops;
mod paths;

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
    }
}
