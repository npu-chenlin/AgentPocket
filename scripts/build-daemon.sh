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

mkdir -p dist
cp "daemon/target/${TARGET}/release/agentpocket" "dist/agentpocket-${ARCH}-linux-musl"
echo "产物：dist/agentpocket-${ARCH}-linux-musl"
