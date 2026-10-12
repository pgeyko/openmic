#!/usr/bin/env bash
set -e

# OpenMic — Скрипт удаления десктопной части

if [ "$(id -u)" -eq 0 ]; then
    PREFIX="/usr/local"
    SYSTEMD_DIR="/etc/systemd/user"
else
    PREFIX="${HOME}/.local"
    SYSTEMD_DIR="${HOME}/.config/systemd/user"
fi

BIN_DIR="${PREFIX}/bin"
APP_DIR="${PREFIX}/share/applications"
ICON_BASE="${PREFIX}/share/icons/hicolor"

echo "=== Удаление OpenMic из ${PREFIX} ==="

rm -f "${BIN_DIR}/openmic" "${BIN_DIR}/openmicd" "${BIN_DIR}/openmic-gtk"
rm -f "${APP_DIR}/openmic.desktop"
rm -f "${SYSTEMD_DIR}/openmicd.service"
rm -f "${ICON_BASE}/scalable/apps/openmic.svg"
rm -f "${ICON_BASE}/48x48/apps/openmic.png"
rm -f "${ICON_BASE}/64x64/apps/openmic.png"
rm -f "${ICON_BASE}/128x128/apps/openmic.png"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APP_DIR}" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "${ICON_BASE}" 2>/dev/null || true
fi

echo "OpenMic успешно удалён."
