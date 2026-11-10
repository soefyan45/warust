#!/bin/sh
# Installs warust's icon and a desktop entry for this checkout into the user's
# ~/.local/share, so Wayland docks and menus show the app icon. Run again
# after moving the folder; remove with: scripts/install-desktop.sh --remove
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor"
if [ "$1" = "--remove" ]; then
  rm -f "$apps/warust.desktop" "$icons/256x256/apps/warust.png" "$icons/scalable/apps/warust.svg"
  echo "removed"
  exit 0
fi
mkdir -p "$apps" "$icons/256x256/apps" "$icons/scalable/apps"
cp "$root/assets/app/warust-256.png" "$icons/256x256/apps/warust.png"
cp "$root/assets/app/warust.svg" "$icons/scalable/apps/warust.svg"
cat > "$apps/warust.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=warust
Comment=WhatsApp gateway desktop
Icon=warust
Path=$root
Exec=$root/target/debug/warust
StartupWMClass=warust
Terminal=false
Categories=Network;Chat;
DESKTOP
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$icons" 2>/dev/null || true
command -v update-desktop-database >/dev/null && update-desktop-database -q "$apps" 2>/dev/null || true
echo "installed: $apps/warust.desktop"
