#!/usr/bin/env bash
# No Rust build, container, or desktop session needed.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf -- "$tmp"' EXIT
export HOME="$tmp/home with spaces" XDG_DATA_HOME="$tmp/data with spaces" XDG_CONFIG_HOME="$tmp/config with spaces" XDG_STATE_HOME="$tmp/state with spaces"
export XDG_CURRENT_DESKTOP= DBUS_SESSION_BUS_ADDRESS=
mkdir -p "$HOME" "$tmp/archive/bin" "$tmp/mock"
archive="$tmp/archive"
cp "$root/packaging/"*.sh "$archive/"
printf '2.17\n' > "$archive/glibc"
for bin in maprisma-applet maprisma-control-tray; do printf '#!/bin/sh\nexit 0\n' > "$archive/bin/$bin"; chmod +x "$archive/bin/$bin"; done
mkdir -p "$archive/share/icons/hicolor/scalable/apps" "$archive/share/applications" "$archive/share/autostart"
id=io.github.ubruckhaus.CosmicMaprisma
cp "$root/data/icons/$id-symbolic.svg" "$archive/share/icons/hicolor/scalable/apps/"
for kind in active off; do cp "$root/data/icons/maprisma-$kind-symbolic.svg" "$archive/share/icons/hicolor/scalable/apps/$id-$kind-symbolic.svg"; done
cp "$root/data/$id.desktop" "$archive/share/applications/"
cp "$root/data/com.maprisma.Control.Tray.desktop" "$archive/share/autostart/"
# Only ldd is mocked: fake shell binaries cannot be queried by the real loader.
printf '#!/bin/sh\necho "${MAPRISMA_TEST_LDD_OUTPUT:-libc.so.6 => /lib/libc.so.6}"\n' > "$tmp/mock/ldd"
chmod +x "$tmp/mock/ldd"
export PATH="$tmp/mock:$PATH"
folder="$HOME/.local/lib/maprisma-control"
desktop="$XDG_DATA_HOME/applications/$id.desktop"
install_release() { bash "$archive/install.sh" > "$tmp/log" 2>&1; }
uninstall_release() { bash "$archive/uninstall.sh" "$@" >> "$tmp/log" 2>&1; }
refuse() { if "$@"; then echo "Expected refusal: $*" >&2; exit 1; fi; }
for diagnostic in 'libwayland-client.so.0 => not found' "version GLIBC_99.0 not found (required by binary)"; do
    export MAPRISMA_TEST_LDD_OUTPUT=$diagnostic
    refuse install_release
    [[ ! -e $HOME/.local && ! -e $XDG_DATA_HOME ]]
done
unset MAPRISMA_TEST_LDD_OUTPUT
printf '99.0\n' > "$archive/glibc"
refuse install_release
[[ ! -e $HOME/.local ]]
printf '2.17\n' > "$archive/glibc"
# Refusals after acquiring the lock must also remove its newly made parents.
for manager in maprisma-settings maprisma-manager; do
    mkdir -p "$XDG_STATE_HOME/$manager/installed"
    touch "$XDG_STATE_HOME/$manager/installed/control.json"
    refuse install_release
    [[ ! -e $HOME/.local ]]
    rm "$XDG_STATE_HOME/$manager/installed/control.json"
done
mkdir -p "$XDG_DATA_HOME/applications"
touch "$desktop"
refuse install_release
[[ ! -e $HOME/.local ]]
rm "$desktop"
install_release
[[ -x $folder/maprisma-applet && -L $HOME/.local/bin/maprisma-applet ]]
[[ $(grep '^Exec=' "$desktop") == "Exec=\"$folder/maprisma-applet\"" ]]
printf '# upgrade\n' >> "$archive/bin/maprisma-applet"
install_release
cmp "$archive/bin/maprisma-applet" "$folder/maprisma-applet"
echo foreign >> "$desktop"
refuse install_release
uninstall_release
[[ -f $desktop && ! -e $folder ]]
refuse install_release
rm "$desktop"
for manager in maprisma-settings maprisma-manager; do
    mkdir -p "$XDG_STATE_HOME/$manager/installed"
    touch "$XDG_STATE_HOME/$manager/installed/control.json"
    refuse install_release
    [[ ! -e $folder ]]
    rm "$XDG_STATE_HOME/$manager/installed/control.json"
done
install_release
cp "$folder/.install-receipt" "$tmp/receipt"
for invalid in unknown duplicate tab newline; do
    cp "$tmp/receipt" "$folder/.install-receipt"
    case $invalid in
        unknown) printf 'foreign\t%s\n' "$(printf a%.0s {1..64})" >> "$folder/.install-receipt" ;;
        duplicate) head -n1 "$tmp/receipt" >> "$folder/.install-receipt" ;;
        tab) printf 'icon\tbad\textra\n' >> "$folder/.install-receipt" ;;
        newline) printf '\n' >> "$folder/.install-receipt" ;;
    esac
    refuse uninstall_release
    refuse install_release
    [[ -f $desktop && -x $folder/maprisma-applet && -L $HOME/.local/bin/maprisma-applet ]]
done
cp "$tmp/receipt" "$folder/.install-receipt"
for step in staged old-moved swapped outer-0 outer-3 outer-6; do
    cp "$folder/maprisma-applet" "$tmp/before"
    printf '# next %s\n' "$step" >> "$archive/bin/maprisma-applet"
    export MAPRISMA_INSTALL_TEST=1 MAPRISMA_INSTALL_TEST_FAIL_AT=$step
    refuse install_release
    unset MAPRISMA_INSTALL_TEST_FAIL_AT
    # Uninstall recovers first; check log and then do another install.
    uninstall_release
    grep -q 'Recovered interrupted installation' "$tmp/log"
    [[ ! -e $folder && ! -e $desktop && ! -L $HOME/.local/bin/maprisma-applet ]]
    install_release
done
journal="$HOME/.local/lib/.maprisma-control-transaction"
# Preserve post-interruption edits to links and regular files during recovery.
for changed in 0 3; do
    icon="$XDG_DATA_HOME/icons/hicolor/scalable/apps/$id-symbolic.svg"
    active="$XDG_DATA_HOME/icons/hicolor/scalable/apps/$id-active-symbolic.svg"
    cp "$icon" "$tmp/old-icon"
    cp "$active" "$tmp/old-active"
    cp "$folder/maprisma-applet" "$tmp/old-applet"
    echo '<!-- upgrade -->' >> "$archive/share/icons/hicolor/scalable/apps/$id-symbolic.svg"
    echo '<!-- upgrade -->' >> "$archive/share/icons/hicolor/scalable/apps/$id-active-symbolic.svg"
    export MAPRISMA_INSTALL_TEST=1 MAPRISMA_INSTALL_TEST_FAIL_AT=outer-3
    refuse install_release
    unset MAPRISMA_INSTALL_TEST_FAIL_AT
    if [[ $changed == 0 ]]; then
        edited="$HOME/.local/bin/maprisma-applet"
        rm "$edited"
        ln -s "$tmp/custom-applet" "$edited"
    else
        edited=$active
        echo '<!-- user edit -->' >> "$edited"
        cp "$edited" "$tmp/edited"
    fi
    # Recovery rolls back the upgrade; installation then refuses the edited file.
    refuse install_release
    grep -Fx "Keeping changed file: $edited" "$tmp/log"
    [[ ! -e $journal ]]
    cmp "$tmp/old-icon" "$icon"
    cmp "$tmp/old-applet" "$folder/maprisma-applet"
    if [[ $changed == 0 ]]; then
        [[ $(readlink "$edited") == "$tmp/custom-applet" ]]
        cmp "$tmp/old-active" "$active"
    else
        cmp "$tmp/edited" "$edited"
        [[ $(readlink "$HOME/.local/bin/maprisma-applet") == "$folder/maprisma-applet" ]]
    fi
    refuse install_release
    uninstall_release
    [[ -e $edited || -L $edited ]]
    rm "$edited"
    install_release
done
# A malformed journal must preserve both live files and the journal itself.
for malformed in missing-backup marker-link marker-directory unexpected backup-name backup-content impossible; do
    export MAPRISMA_INSTALL_TEST=1 MAPRISMA_INSTALL_TEST_FAIL_AT=staged
    refuse install_release
    unset MAPRISMA_INSTALL_TEST_FAIL_AT
    case $malformed in
        missing-backup) rm -rf "$journal/backup"; touch "$journal/outer-started" ;;
        marker-link) rm "$journal/ready"; ln -s "$tmp/receipt" "$journal/ready" ;;
        marker-directory) rm "$journal/ready"; mkdir "$journal/ready" ;;
        unexpected) touch "$journal/surprise" ;;
        backup-name) touch "$journal/backup/7" ;;
        backup-content) echo corrupt > "$journal/backup/2" ;;
        impossible) touch "$journal/committed" ;;
    esac
    before=$(tar -cf - -C "$HOME" . | sha256sum)
    refuse uninstall_release
    grep -q 'Malformed transaction journal' "$tmp/log"
    [[ $(tar -cf - -C "$HOME" . | sha256sum) == "$before" ]]
    rm -rf "$journal"
done
# Simulate SIGKILL during recursive cleanup, including loss of the receipt.
export MAPRISMA_INSTALL_TEST_FAIL_AT=uninstall-detached
refuse uninstall_release
unset MAPRISMA_INSTALL_TEST_FAIL_AT
cleanup="$HOME/.local/lib/.maprisma-control-cleanup"
[[ ! -e $folder && -d $cleanup ]]
rm "$cleanup/.install-receipt"
uninstall_release
[[ ! -e $cleanup ]]
install_release
ln -s "$folder/uninstall.sh" "$tmp/uninstall-link"
(cd /; bash "$tmp/uninstall-link") >> "$tmp/log" 2>&1
[[ ! -e $folder ]]
install_release
# A real executable is needed because /proc/exe identifies the tray.
cat > "$tmp/tray.c" <<'C'
#include <signal.h>
#include <unistd.h>
#include <stdlib.h>
static void stop(int sig) { (void)sig; sleep(2); _exit(0); }
int main(void) {
    if (getenv("MAPRISMA_FAKE_EXIT")) return 1;
    signal(SIGTERM, stop);
    for (;;) pause();
}
C
cc "$tmp/tray.c" -o "$archive/bin/maprisma-control-tray"
install_release
"$folder/maprisma-control-tray" &
old_tray=$!
sleep 0.2
export XDG_CURRENT_DESKTOP=KDE DBUS_SESSION_BUS_ADDRESS=fake
install_release
# ended before the installer returned: gone, or a zombie until this shell reaps it
[[ ! -e /proc/$old_tray || $(awk '{print $3}' "/proc/$old_tray/stat") == Z ]]
wait "$old_tray" || true
uninstall_release
export MAPRISMA_FAKE_EXIT=1
install_release
grep -q 'tray exited' "$tmp/log"
unset MAPRISMA_FAKE_EXIT
export XDG_CURRENT_DESKTOP= DBUS_SESSION_BUS_ADDRESS=
# The hook is ignored outside explicit test mode.
unset MAPRISMA_INSTALL_TEST
export MAPRISMA_INSTALL_TEST_FAIL_AT=swapped
install_release
mkdir -p "$XDG_CONFIG_HOME/maprisma-control"
touch "$XDG_CONFIG_HOME/maprisma-control/settings.conf"
uninstall_release --purge
[[ ! -e $XDG_CONFIG_HOME/maprisma-control && ! -e $folder ]]
echo 'release installer tests passed'
