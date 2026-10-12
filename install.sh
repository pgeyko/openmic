#!/usr/bin/env bash
set -e

# OpenMic — Скрипт установки десктопной части на ALT Linux / Linux
# Не требует root прав, устанавливается в окружение текущего пользователя.

PREFIX="${HOME}/.local"
BIN_DIR="${PREFIX}/bin"
APP_DIR="${PREFIX}/share/applications"
ICON_DIR="${PREFIX}/share/icons/hicolor/scalable/apps"

echo "=== Сборка релизных бинарников OpenMic ==="
cargo build --release

echo "=== Установка исполняемых файлов в ${BIN_DIR} ==="
mkdir -p "${BIN_DIR}"
cp -f target/release/openmic "${BIN_DIR}/openmic"
cp -f target/release/openmicd "${BIN_DIR}/openmicd"
cp -f target/release/openmic-gtk "${BIN_DIR}/openmic-gtk"
chmod +x "${BIN_DIR}/openmic" "${BIN_DIR}/openmicd" "${BIN_DIR}/openmic-gtk"

echo "=== Установка иконки и меню приложений ==="
mkdir -p "${ICON_DIR}" "${APP_DIR}"
cp -f assets/openmic.svg "${ICON_DIR}/openmic.svg"
cp -f assets/openmic.desktop "${APP_DIR}/openmic.desktop"

# Обновляем кэш десктоп-меню, если утилита доступна
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APP_DIR}" 2>/dev/null || true
fi

echo ""
echo "Установка успешно завершена!"
echo "Теперь вы можете:"
echo "1. Запускать OpenMic из системного меню приложений ALT Linux."
echo "2. Запускать из терминала команду 'openmic-gtk' или CLI 'openmic'."
echo ""
