#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ "$(id -u)" -eq 0 ]; then
    BIN_DIR="/usr/local/bin"
    APP_DIR="/usr/local/share/applications"
    ICON_DIR="/usr/local/share/icons/hicolor/512x512/apps"
elif command -v sudo >/dev/null 2>&1; then
    echo "Cài đặt zalo-next vào /usr/local/bin (yêu cầu quyền sudo)..."
    exec sudo bash "$0" "$@"
else
    BIN_DIR="$HOME/.local/bin"
    APP_DIR="$HOME/.local/share/applications"
    ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"
fi

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"

install -m 755 "$SCRIPT_DIR/zalo-next" "$BIN_DIR/zalo-next"
install -m 644 "$SCRIPT_DIR/zalo-next.desktop" "$APP_DIR/zalo-next.desktop"
install -m 644 "$SCRIPT_DIR/zalo-next.png" "$ICON_DIR/zalo-next.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi

echo "Đã cài đặt thành công zalo-next tại $BIN_DIR/zalo-next"
echo "Chạy ứng dụng bằng lệnh: zalo-next (hoặc mở từ menu ứng dụng)."
