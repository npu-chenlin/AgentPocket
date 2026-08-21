#!/bin/sh
# AgentPocket mesh 守护进程一键安装：下载二进制 + systemd 服务 + 自启动。
# 用法：curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash
# 卸载：sudo agentpocket uninstall（或同命令加 --uninstall）
set -eu

REPO="npu-chenlin/AgentPocket"
BIN_PATH="/usr/local/bin/agentpocket"
SERVICE_PATH="/etc/systemd/system/agentpocket.service"
COMPLETION_PATH="/usr/share/bash-completion/completions/agentpocket"

if [ "${1:-}" = "--uninstall" ]; then
    # 二进制自带卸载命令，优先委托给它
    if [ -x "$BIN_PATH" ]; then
        exec "$BIN_PATH" uninstall
    fi
    systemctl stop agentpocket 2>/dev/null || true
    systemctl disable agentpocket 2>/dev/null || true
    rm -f "$SERVICE_PATH" "$BIN_PATH" "$COMPLETION_PATH"
    systemctl daemon-reload
    echo "已卸载 agentpocket（配置目录 ~/.local/share/com.local.agentpocket.desktop 保留）"
    exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "请用 sudo 运行（或 curl … | sudo bash）" >&2
    exit 1
fi

case "$(uname -m)" in
    x86_64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "不支持的架构：$(uname -m)" >&2; exit 1 ;;
esac
ASSET="agentpocket-${ARCH}-linux-musl"

LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"
LATEST_JSON="$(curl -fsSL "$LATEST_URL")" || {
    echo "查询 GitHub release 失败（未认证 API 限额 60 次/小时/IP，可能临时限流，稍后重试）" >&2
    exit 1
}
DOWNLOAD_URL="$(printf '%s' "$LATEST_JSON" | tr ',' '\n' | grep -o "https://[^\"]*${ASSET}" | head -n 1)"
if [ -z "$DOWNLOAD_URL" ]; then
    echo "未在最新 release 找到资产 ${ASSET}，请到 https://github.com/${REPO}/releases 手动下载" >&2
    exit 1
fi

echo "下载 ${DOWNLOAD_URL} …"
curl -fsSL -o "$BIN_PATH" "$DOWNLOAD_URL"
chmod 755 "$BIN_PATH"

RUN_USER="${SUDO_USER:-root}"
HOME_DIR="$(getent passwd "$RUN_USER" | cut -d: -f6)"
[ -n "$HOME_DIR" ] || { HOME_DIR="/root"; RUN_USER="root"; }
RUN_UID="$(id -u "$RUN_USER")"

cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=AgentPocket mesh daemon
After=network-online.target
Wants=network-online.target

[Service]
User=${RUN_USER}
Environment=HOME=${HOME_DIR}
Environment=XDG_RUNTIME_DIR=/run/user/${RUN_UID}
ExecStart=${BIN_PATH} serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now agentpocket

# bash 补全（新 shell 生效）
if [ -d /usr/share/bash-completion/completions ]; then
    "$BIN_PATH" completions bash > "$COMPLETION_PATH" 2>/dev/null || true
fi

# 可选：安装 Kimi Code CLI 并生成 kimi-web 服务（交互终端才询问）
if [ -e /dev/tty ]; then
    printf '是否安装 Kimi Code CLI 并生成 kimi-web 服务？[y/N] ' > /dev/tty
    read -r REPLY < /dev/tty || REPLY=""
    case "$REPLY" in
        y|Y)
            sudo -u "$RUN_USER" "$BIN_PATH" kimi --upgrade > /dev/tty 2>&1 || true
            sudo -u "$RUN_USER" "$BIN_PATH" kimi-web enable > /dev/tty 2>&1 || true
            ;;
    esac
fi

echo "安装完成："
echo "  状态    systemctl status agentpocket"
echo "  日志    journalctl -u agentpocket -f"
echo "  发现    sudo -u ${RUN_USER} ${BIN_PATH} peers"
echo "  同步    sudo -u ${RUN_USER} ${BIN_PATH} pull <桌面机IP或MagicDNS名>"
echo "  卸载    sudo ${BIN_PATH} uninstall"
