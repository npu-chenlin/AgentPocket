#!/bin/sh
# AgentPocket mesh 守护进程一键安装：下载二进制 + systemd 服务 + 自启动。
# 用法：curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash
# 卸载：同命令加 --uninstall
set -eu

REPO="npu-chenlin/AgentPocket"
BIN_PATH="/usr/local/bin/agentpocket"
SERVICE_PATH="/etc/systemd/system/agentpocket.service"

if [ "${1:-}" = "--uninstall" ]; then
    systemctl stop agentpocket 2>/dev/null || true
    systemctl disable agentpocket 2>/dev/null || true
    rm -f "$SERVICE_PATH" "$BIN_PATH"
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
DOWNLOAD_URL="$(curl -fsSL "$LATEST_URL" | tr ',' '\n' | grep -o "https://[^\"]*${ASSET}" | head -n 1)"
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

cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=AgentPocket mesh daemon
After=network-online.target
Wants=network-online.target

[Service]
User=${RUN_USER}
Environment=HOME=${HOME_DIR}
ExecStart=${BIN_PATH} serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now agentpocket

echo "安装完成："
echo "  状态    systemctl status agentpocket"
echo "  日志    journalctl -u agentpocket -f"
echo "  发现    sudo -u ${RUN_USER} ${BIN_PATH} peers"
echo "  同步    sudo -u ${RUN_USER} ${BIN_PATH} pull <桌面机IP或MagicDNS名>"
