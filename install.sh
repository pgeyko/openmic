#!/usr/bin/env bash
set -e

# OpenMic — Установочный скрипт для Linux (ALT Linux и др.)
# Поддерживает установку в пользовательский каталог (~/.local) без прав root
# или общесистемную установку (/usr/local) при запуске с sudo.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

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

echo "==========================================="
echo "   Установка OpenMic (директория: ${PREFIX})"
echo "==========================================="

echo "-> Проверка зависимостей сборки..."
if ! command -v cargo >/dev/null 2>&1; then
    echo "Ошибка: cargo не найден. Установите Rust (sudo apt-get install rust rust-cargo)" >&2
    exit 1
fi

echo "-> Сборка оптимизированных релизных бинарников..."
cargo build --release

echo "-> Копирование исполняемых файлов в ${BIN_DIR}..."
mkdir -p "${BIN_DIR}"
install -m 755 target/release/openmic "${BIN_DIR}/openmic"
install -m 755 target/release/openmicd "${BIN_DIR}/openmicd"
install -m 755 target/release/openmic-gtk "${BIN_DIR}/openmic-gtk"

echo "-> Установка иконок (SVG и растровые PNG)..."
mkdir -p "${ICON_BASE}/scalable/apps"
mkdir -p "${ICON_BASE}/48x48/apps"
mkdir -p "${ICON_BASE}/64x64/apps"
mkdir -p "${ICON_BASE}/128x128/apps"

install -m 644 assets/openmic.svg "${ICON_BASE}/scalable/apps/openmic.svg"
[ -f assets/openmic_48.png ] && install -m 644 assets/openmic_48.png "${ICON_BASE}/48x48/apps/openmic.png"
[ -f assets/openmic_64.png ] && install -m 644 assets/openmic_64.png "${ICON_BASE}/64x64/apps/openmic.png"
[ -f assets/openmic_128.png ] && install -m 644 assets/openmic_128.png "${ICON_BASE}/128x128/apps/openmic.png"

echo "-> Установка десктоп-ярлыка меню..."
mkdir -p "${APP_DIR}"
install -m 644 assets/openmic.desktop "${APP_DIR}/openmic.desktop"
# Подставляем абсолютный путь к бинарнику для гарантированного запуска через меню/лаунчеры
sed -i "s|^Exec=.*|Exec=${BIN_DIR}/openmic-gtk|" "${APP_DIR}/openmic.desktop"

echo "-> Установка systemd user-сервиса..."
mkdir -p "${SYSTEMD_DIR}"
install -m 644 assets/openmicd.service "${SYSTEMD_DIR}/openmicd.service"

echo "-> Обновление системных баз данных..."
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "${APP_DIR}" 2>/dev/null || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "${ICON_BASE}" 2>/dev/null || true
fi

echo ""
echo "Установка успешно завершена!"
echo ""
echo "Исполняемые файлы: ${BIN_DIR}/openmic-gtk, openmicd, openmic"
echo "Ярлык меню:        ${APP_DIR}/openmic.desktop"
echo ""
echo "Запуск приложения:"
echo "  - Из системного меню приложений (в разделе «Аудио и видео»)"
echo "  - Или командой в терминале: openmic-gtk"
echo ""
