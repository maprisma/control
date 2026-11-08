#!/usr/bin/env bash
# Build and install the Maprisma panel applet for the current user (no root needed):
#   ~/.local/bin/maprisma-applet            the applet
#   ~/.local/share/applications/…desktop           makes it an applet for the panel
# Then add “Maprisma Control” in COSMIC Settings › Desktop › Panel › Applets.
set -euo pipefail
cd "$(dirname "$0")"
id=io.github.ubruckhaus.CosmicMaprisma
data="${XDG_DATA_HOME:-$HOME/.local/share}"
bin="$HOME/.local/bin"

cargo build --release
install -Dm755 target/release/maprisma-applet "$bin/maprisma-applet"
install -Dm755 target/release/maprisma-control-tray "$bin/maprisma-control-tray"
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
echo "Installed KDE tray; it will start at the next Plasma login."
