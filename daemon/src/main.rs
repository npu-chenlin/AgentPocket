mod kimi;
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
    /// 从 peer 拉取 ~/.kimi-code/config.toml（覆盖本地，旧文件备份为 .bak）
    Pull {
        host: String,
        /// 只打印预览，不落盘
        #[arg(long)]
        dry_run: bool,
    },
    /// 把本地 ~/.kimi-code/config.toml 推送给 peer
    Push { host: String },
    /// 发现并列出 mesh peer
    Peers,
    /// 一次性探测已配置服务器状态
    Status,
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
    /// 停止并移除服务与二进制（需 sudo，配置保留）
    Uninstall,
    /// 输出 shell 自动补全脚本（bash/zsh/fish/elvish/powershell）
    Completions { shell: clap_complete::Shell },
}

fn main() {
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
