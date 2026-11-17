#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ] && [ -f "/usr/local/bin/zalo-next" ] && command -v sudo >/dev/null 2>&1; then
    exec sudo bash "$0" "$@"
fi

rm -f /usr/local/bin/zalo-next \
      /usr/local/share/applications/zalo-next.desktop \
      /usr/local/share/icons/hicolor/512x512/apps/zalo-next.png \
      "$HOME/.local/bin/zalo-next" \
      "$HOME/.local/share/applications/zalo-next.desktop" \
      "$HOME/.local/share/icons/hicolor/512x512/apps/zalo-next.png"

echo "Đã gỡ cài đặt zalo-next."
