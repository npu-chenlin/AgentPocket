#!/bin/sh
# 构建 musl 静态二进制并复制到 dist/（资产名与 update.rs 的 arch_asset_name 一致）。
set -eu
cd "$(dirname "$0")/.."

export CARGO_REGISTRIES_CRATES_IO_INDEX='sparse+https://rsproxy.cn/index/'
export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

ARCH="${1:-x86_64}"
TARGET="${ARCH}-unknown-linux-musl"

rustup target add "$TARGET"
cargo build --release --target "$TARGET" --manifest-path daemon/Cargo.toml

# target 目录：env 优先，其次项目/全局 cargo 配置的 target-dir，默认 daemon/target
TARGET_DIR="${CARGO_TARGET_DIR:-}"
if [ -z "$TARGET_DIR" ]; then
    for cfg in .cargo/config.toml "$HOME/.cargo/config.toml" "$HOME/.cargo/config"; do
        if [ -f "$cfg" ]; then
            TARGET_DIR="$(sed -n 's/^target-dir *= *"\(.*\)"/\1/p' "$cfg" | head -n 1)"
            [ -n "$TARGET_DIR" ] && break
        fi
    done
fi
TARGET_DIR="${TARGET_DIR:-daemon/target}"

mkdir -p dist
cp "${TARGET_DIR}/${TARGET}/release/agentpocket" "dist/agentpocket-${ARCH}-linux-musl"
echo "产物：dist/agentpocket-${ARCH}-linux-musl"
