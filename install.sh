#!/usr/bin/env bash
# Build and install the Maprisma panel applet for the current user (no root needed):
#   ~/.local/bin/maprisma-applet            the applet
#   ~/.local/share/applications/…desktop           makes it an applet for the panel
#   ~/.local/share/icons/hicolor/scalable/apps/…svg  the bar icon (its own name; Maprisma's logo)
# Then add “Maprisma Control” in COSMIC Settings › Desktop › Panel › Applets.
set -euo pipefail
cd "$(dirname "$0")"
id=io.github.ubruckhaus.CosmicMaprisma
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin"

cargo build --release
install -Dm755 target/release/maprisma-applet "$bin/maprisma-applet"
install -Dm755 target/release/maprisma-control-tray "$bin/maprisma-control-tray"
# The bar icon (applet and tray) has a name of its own, so it can be changed without touching Maprisma's.
install -Dm644 "data/icons/$id-symbolic.svg" "$data/icons/hicolor/scalable/apps/$id-symbolic.svg"
gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
# Exec has two escaping layers: command argument quoting, then desktop value
# escaping. Percent signs must also be doubled to avoid field-code expansion.
desktop_exec() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    value="${value//\$/\\\$}"
    value="${value//\`/\\\`}"
    value="${value//\%/%%}"
    value="${value//\\/\\\\}"
    printf '"%s"' "$value"
}
# The panel starts applets without ~/.local/bin in PATH: the full path, quoted.
mkdir -p "$data/applications"
{
    grep -v '^Exec=' "data/$id.desktop"
    printf 'Exec=%s\n' "$(desktop_exec "$bin/maprisma-applet")"
} > "$data/applications/$id.desktop"
echo "Installed. Add “Maprisma Control” in COSMIC Settings › Desktop › Panel › Applets."

autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
mkdir -p "$autostart"
{
    cat data/com.maprisma.Control.Tray.desktop
    printf 'Exec=%s\n' "$(desktop_exec "$bin/maprisma-control-tray")"
} > "$autostart/com.maprisma.Control.Tray.desktop"
if pgrep -f "$bin/maprisma-control-tray" >/dev/null 2>&1; then
    pkill -f "$bin/maprisma-control-tray" || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pgrep -f "$bin/maprisma-control-tray" >/dev/null 2>&1 || break
        sleep 0.1
    done
    setsid -f "$bin/maprisma-control-tray" >/dev/null 2>&1 < /dev/null || true
fi
echo "Installed KDE tray; it will start at the next Plasma or Hyprland login."
