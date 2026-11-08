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
# The panel starts applets without ~/.local/bin in PATH: the full path.
mkdir -p "$data/applications"
sed "s|^Exec=.*|Exec=$bin/maprisma-applet|" "data/$id.desktop" > "$data/applications/$id.desktop"
echo "Installed. Add “Maprisma Control” in COSMIC Settings › Desktop › Panel › Applets."
