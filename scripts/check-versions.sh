#!/bin/sh
# 校验发布标签与各端产品版本一致。CI 或发布前运行：
#   scripts/check-versions.sh v2.9.0
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
EXPECTED=${1:-}
if [ -z "$EXPECTED" ]; then
    EXPECTED=$(git -C "$ROOT" describe --tags --abbrev=0 2>/dev/null || true)
fi
EXPECTED=${EXPECTED#v}
[ -n "$EXPECTED" ] || {
    echo "无法确定版本，请传入 vX.Y.Z" >&2
    exit 1
}

read_value() {
    file=$1
    pattern=$2
    value=$(sed -n "${pattern}" "$ROOT/$file" | head -n 1)
    [ -n "$value" ] || {
        echo "无法从 $file 读取版本" >&2
        exit 1
    }
    printf '%s' "$value"
}

check() {
    file=$1
    actual=$2
    if [ "$actual" != "$EXPECTED" ]; then
        echo "$file: $actual (期望 $EXPECTED)" >&2
        exit 1
    fi
}

check core/Cargo.toml "$(read_value core/Cargo.toml '/^version = / s/.*"\([^"]*\)".*/\1/p')"
check daemon/Cargo.toml "$(read_value daemon/Cargo.toml '/^version = / s/.*"\([^"]*\)".*/\1/p')"
check desktop/package.json "$(read_value desktop/package.json '/^[[:space:]]*"version"/ s/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
check desktop/package-lock.json "$(sed -n '3s/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/desktop/package-lock.json")"
check desktop/src-tauri/Cargo.toml "$(read_value desktop/src-tauri/Cargo.toml '/^version = / s/.*"\([^"]*\)".*/\1/p')"
check desktop/src-tauri/tauri.conf.json "$(read_value desktop/src-tauri/tauri.conf.json '/^[[:space:]]*"version"/ s/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"
check app/build.gradle "$(sed -n "/versionName[[:space:]]/s/.*versionName[[:space:]]*['\"]\([^'\"]*\)['\"].*/\1/p" "$ROOT/app/build.gradle" | head -n 1)"

echo "版本一致：$EXPECTED"
