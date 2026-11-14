#!/usr/bin/env bash
# Per-user prebuilt installer. All receipt and journal names map to fixed paths.
set -euo pipefail
umask 077
src=$(cd -- "$(dirname -- "$0")" && pwd)
mode=install
purge=0
case "${1:-}" in
    --uninstall) mode=uninstall; shift; if [[ ${1:-} == --purge ]]; then purge=1; shift; fi ;;
esac
[[ $# == 0 ]] || { echo 'Usage: install.sh (or uninstall.sh [--purge])' >&2; exit 1; }
die() { echo "Maprisma Control: $*" >&2; exit 1; }
base="$HOME/.local/lib"
folder="$base/maprisma-control"
journal="$base/.maprisma-control-transaction"
cleanup="$base/.maprisma-control-cleanup"
data=${XDG_DATA_HOME:-$HOME/.local/share}
config=${XDG_CONFIG_HOME:-$HOME/.config}
state=${XDG_STATE_HOME:-$HOME/.local/state}
id=io.github.ubruckhaus.CosmicMaprisma
names=(applet-link tray-link icon active-icon off-icon applet-desktop tray-desktop)
paths=("$HOME/.local/bin/maprisma-applet" "$HOME/.local/bin/maprisma-control-tray"
    "$data/icons/hicolor/scalable/apps/$id-symbolic.svg"
    "$data/icons/hicolor/scalable/apps/$id-active-symbolic.svg"
    "$data/icons/hicolor/scalable/apps/$id-off-symbolic.svg"
    "$data/applications/$id.desktop" "$config/autostart/com.maprisma.Control.Tray.desktop")
for p in "$HOME" "$data" "$config" "$state"; do
    [[ $p == /* && $p != *$'\n'* && $p != *$'\t'* ]] || die 'Paths must be absolute and contain no tabs or newlines.'
done
exists() { [[ -e $1 || -L $1 ]]; }
settings_check() {
    for manager in maprisma-settings maprisma-manager; do
        ! exists "$state/$manager/installed/control.json" || die 'Update/remove Control through Maprisma Settings instead.'
    done
}
settings_check
# A receipt is data, never paths or shell code. Require the complete fixed set.
declare -A hashes=() links=()
read_receipt() {
    local line name value target count=0 receipt_root=${1:-$folder}
    [[ -d $receipt_root && ! -L $receipt_root && -f $receipt_root/.install-receipt && ! -L $receipt_root/.install-receipt ]] || return 1
    LC_ALL=C tr -d '\000' < "$receipt_root/.install-receipt" | cmp -s "$receipt_root/.install-receipt" - || return 1
    hashes=(); links=()
    while IFS= read -r line || [[ -n $line ]]; do
        [[ $line == *$'\t'* ]] || return 1
        name=${line%%$'\t'*}; value=${line#*$'\t'}
        case $name in applet-link|tray-link|icon|active-icon|off-icon|applet-desktop|tray-desktop) ;; *) return 1 ;; esac
        [[ ! ${hashes[$name]+yes} && ! ${links[$name]+yes} ]] || return 1
        case $name in
            applet-link|tray-link)
                [[ $value == link$'\t'* ]] || return 1
                target=${value#*$'\t'}
                [[ $target == "$folder/maprisma-applet" && $name == applet-link || $target == "$folder/maprisma-control-tray" && $name == tray-link ]] || return 1
                links[$name]=$target ;;
            icon|active-icon|off-icon|applet-desktop|tray-desktop)
                [[ $value =~ ^[0-9a-f]{64}$ ]] || return 1
                hashes[$name]=$value ;;
            *) return 1 ;;
        esac
        count=$((count+1))
    done < "$receipt_root/.install-receipt"
    [[ $count == 7 ]]
}
matches() {
    local i=$1 name=${names[$1]} path=${paths[$1]}
    if [[ ${links[$name]+yes} ]]; then
        [[ -L $path && $(readlink -- "$path") == "${links[$name]}" ]]
    else
        [[ -f $path && ! -L $path && $(sha256sum -- "$path" | cut -d' ' -f1) == "${hashes[$name]}" ]]
    fi
}
# No writes at all until architecture, loader, and dependencies have passed.
if [[ $mode == install ]]; then
    [[ $(uname -m) == x86_64 ]] || die 'This archive requires x86_64.'
    required=$(cat -- "$src/glibc")
    [[ $required =~ ^2\.[0-9]+$ ]] || die 'Invalid archive glibc baseline.'
    current=$(getconf GNU_LIBC_VERSION) || die 'GNU glibc is required.'
    current=${current##* }
    [[ $(printf '%s\n%s\n' "$required" "$current" | sort -V | head -n1) == "$required" ]] || die "glibc $required or newer required (found $current); upgrade your distribution."
    hint='libxkbcommon, Wayland, fontconfig, freetype and Mesa runtime packages'
    if [[ -r /etc/os-release ]]; then
        # Standard, root-controlled distribution metadata.
        . /etc/os-release
        case " ${ID:-} ${ID_LIKE:-} " in
            *arch*) hint='sudo pacman -S libxkbcommon wayland fontconfig freetype2 mesa' ;;
            *debian*|*ubuntu*) hint='sudo apt install libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libwayland-cursor0 libwayland-egl1 libfontconfig1 libfreetype6 libegl1 libgl1' ;;
            *fedora*) hint='sudo dnf install libxkbcommon libxkbcommon-x11 wayland-libs fontconfig freetype mesa-libEGL mesa-libGL' ;;
            *suse*) hint='sudo zypper install libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 fontconfig libfreetype6 Mesa-libEGL1 Mesa-libGL1' ;;
        esac
    fi
    for binary in maprisma-applet maprisma-control-tray; do
        [[ -f $src/bin/$binary && -x $src/bin/$binary ]] || die "Missing binary: $binary"
        output=$(LC_ALL=C ldd -- "$src/bin/$binary" 2>&1) || die "$output; install runtime dependencies: $hint"
        [[ ! $output =~ not\ found|version.*not\ found|version.*required ]] || die "$output; install runtime dependencies: $hint"
    done
fi
mkdir -p -- "$base"
exec 9>"$base/.maprisma-control-install.lock"
flock -x 9
settings_check
tray_pids() {
    local root=$1 proc exe
    for proc in /proc/[0-9]*; do
        [[ -O $proc ]] || continue
        exe=$(readlink "$proc/exe" 2>/dev/null) || continue
        if [[ $exe == "$root/maprisma-control-tray" || $exe == "$root/maprisma-control-tray (deleted)" ]]; then
            echo "${proc##*/}"
        fi
    done
}
stop_tray() {
    local root=$1 pid attempt
    for pid in $(tray_pids "$root"); do kill -TERM "$pid" 2>/dev/null || true; done
    for ((attempt=0; attempt<50; attempt++)); do
        [[ -n $(tray_pids "$root") ]] || return 0
        sleep 0.1
    done
    for pid in $(tray_pids "$root"); do kill -KILL "$pid" 2>/dev/null || true; done
    for ((attempt=0; attempt<50; attempt++)); do
        [[ -n $(tray_pids "$root") ]] || return 0
        sleep 0.1
    done
    die 'Old tray did not terminate; refusing to start a replacement.'
}
hook() {
    if [[ ${MAPRISMA_INSTALL_TEST:-} == 1 && ${MAPRISMA_INSTALL_TEST_FAIL_AT:-} == "$1" ]]; then
        kill -KILL "$$"
    fi
}
# Completed journals are atomically detached before deletion. Killing rm cannot
# leave half a journal that a later invocation mistakes for rollback metadata.
discard_journal() {
    mv -- "$journal" "$cleanup"
    rm -rf -- "$cleanup"
}
# Validate all rollback metadata before deleting cleanup or touching live paths.
context() { printf '%s\n' "$HOME" "$data" "$config"; }
validate_journal() {
    local entry rel i tree
    [[ -d $journal && ! -L $journal && -O $journal ]] || return 1
    while IFS= read -r -d '' entry; do
        [[ $(stat -c %u -- "$entry") == "$(id -u)" ]] || return 1
        rel=${entry#"$journal/"}
        case $rel in
            context|ready|no-old|swap-started|outer-started|committed)
                [[ -f $entry && ! -L $entry ]] || return 1
                [[ $rel == context || ! -s $entry ]] || return 1 ;;
            new|old|backup|outer) [[ -d $entry && ! -L $entry ]] || return 1 ;;
            backup/*|outer/*)
                i=${rel#*/}
                [[ $i =~ ^[0-6]$ ]] || return 1
                if [[ $i -lt 2 ]]; then
                    [[ -L $entry && $(readlink -- "$entry") == "$folder/${paths[$i]##*/}" ]] || return 1
                else
                    [[ -f $entry && ! -L $entry ]] || return 1
                fi ;;
            new/*|old/*)
                # Installed payloads contain only regular files, never links or directories.
                case ${rel#*/} in maprisma-applet|maprisma-control-tray|install.sh|uninstall.sh|glibc|.install-receipt) ;; *) return 1 ;; esac
                [[ -f $entry && ! -L $entry ]] || return 1 ;;
            *) return 1 ;;
        esac
    done < <(find "$journal" -mindepth 1 -print0)
    if exists "$journal/context"; then
        cmp -s "$journal/context" <(context) || return 1
    else
        [[ -z $(find "$journal" -mindepth 1 -print -quit) ]] || return 1
    fi
    if exists "$journal/ready"; then
        [[ -f $journal/context && -d $journal/backup && -d $journal/outer ]] || return 1
        for i in "${!names[@]}"; do exists "$journal/outer/$i" || return 1; done
        for tree in "$journal/new" "$journal/old"; do
            if exists "$tree"; then
                (read_receipt "$tree") || return 1
                for entry in maprisma-applet maprisma-control-tray install.sh uninstall.sh glibc; do
                    [[ -f $tree/$entry ]] || return 1
                done
            fi
        done
        if exists "$journal/no-old"; then
            [[ ! -e $journal/old ]] || return 1
            [[ -z $(find "$journal/backup" -mindepth 1 -print -quit) ]] || return 1
        fi
        if ! exists "$journal/no-old"; then
            tree=$folder
            [[ ! -d $journal/old ]] || tree=$journal/old
            (
                read_receipt "$tree" || exit 1
                for i in "${!names[@]}"; do
                    entry=$journal/backup/$i
                    exists "$entry" || continue
                    if [[ $i -ge 2 ]]; then
                        [[ $(sha256sum -- "$entry" | cut -d' ' -f1) == "${hashes[${names[$i]}]}" ]] || exit 1
                    fi
                done
            ) || return 1
        fi
        if ! exists "$journal/swap-started"; then
            [[ -d $journal/new && ! -e $journal/old && ! -e $journal/outer-started && ! -e $journal/committed ]] || return 1
        fi
        if exists "$journal/outer-started"; then
            [[ -f $journal/swap-started && ! -e $journal/new ]] || return 1
        fi
        if exists "$journal/committed"; then [[ -f $journal/outer-started ]] || return 1; fi
    else
        for entry in no-old swap-started outer-started committed old; do
            # no-old can be written immediately before ready.
            [[ $entry == no-old || ! -e $journal/$entry ]] || return 1
        done
    fi
    return 0
}
if exists "$journal"; then
    validate_journal || die 'Malformed transaction journal; nothing changed.'
fi
if exists "$cleanup"; then
    [[ -d $cleanup && ! -L $cleanup && -O $cleanup ]] || die 'Invalid transaction cleanup directory.'
    rm -rf -- "$cleanup"
    if [[ $mode == uninstall && ! -e $folder && ! -e $journal ]]; then
        echo 'Finished interrupted removal.'
        exit
    fi
fi
recover() {
    validate_journal || die 'Malformed transaction journal; nothing changed.'
    if [[ ! -f $journal/context && ! -f $journal/ready ]]; then
        discard_journal
        return
    fi
    [[ -f $journal/context && $(cat "$journal/context") == "$(context)" ]] || die 'Interrupted install used different XDG paths; rerun with those paths.'
    if [[ -f $journal/committed ]]; then
        stop_tray "$journal/old"
        discard_journal
        return
    fi
    if [[ -f $journal/ready ]]; then
        for i in "${!names[@]}"; do
            p=${paths[$i]}
            rm -f -- "$p.maprisma-control-new"
            # Accepted risk: edits made between an interrupted run and the next
            # invocation are overwritten when recovery restores outer files.
            if [[ -f $journal/outer-started ]]; then
                if exists "$journal/backup/$i"; then
                    rm -f -- "$p"
                    cp -a -- "$journal/backup/$i" "$p"
                else
                    rm -f -- "$p"
                fi
            fi
        done
        if [[ -d $journal/old ]]; then
            rm -rf -- "$folder"
            mv -- "$journal/old" "$folder"
        elif [[ -f $journal/no-old && -f $journal/swap-started ]]; then
            rm -rf -- "$folder"
        fi
    fi
    discard_journal
    echo 'Recovered interrupted installation.'
}
if exists "$journal"; then recover; fi
valid=0
if exists "$folder"; then
    read_receipt || die 'Invalid or missing receipt; nothing replaced or removed.'
    valid=1
fi

if [[ $mode == uninstall ]]; then
    [[ $valid == 1 ]] || die 'No installation receipt; nothing removed.'
    stop_tray "$folder"
    for i in "${!names[@]}"; do
        if matches "$i"; then rm -- "${paths[$i]}"
        elif exists "${paths[$i]}"; then echo "Keeping changed file: ${paths[$i]}"; fi
    done
    mv -- "$folder" "$cleanup"
    hook uninstall-detached
    rm -rf -- "$cleanup"
    if [[ $purge == 1 ]]; then
        echo "Removing configuration: $config/maprisma-control"
        rm -rf -- "$config/maprisma-control"
    fi
    echo 'Maprisma Control uninstalled.'
    exit
fi
for i in "${!names[@]}"; do
    if exists "${paths[$i]}"; then
        [[ $valid == 1 ]] && matches "$i" || die "Foreign or changed target: ${paths[$i]}"
    fi
    ! exists "${paths[$i]}.maprisma-control-new" || die "Foreign staging file: ${paths[$i]}.maprisma-control-new"
done
mkdir -- "$journal"
context > "$journal/context"
rollback() { local status=$?; trap - EXIT; if [[ $status != 0 ]]; then recover; fi; exit "$status"; }
trap rollback EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
mkdir -- "$journal/new" "$journal/backup" "$journal/outer"
cp -- "$src/bin/"* "$journal/new/"
cp -- "$src/install.sh" "$src/uninstall.sh" "$src/glibc" "$journal/new/"
chmod 755 "$journal/new/maprisma-applet" "$journal/new/maprisma-control-tray" "$journal/new/"*.sh
ln -s -- "$folder/maprisma-applet" "$journal/outer/0"
ln -s -- "$folder/maprisma-control-tray" "$journal/outer/1"
cp -- "$src/share/icons/hicolor/scalable/apps/$id-symbolic.svg" "$journal/outer/2"
cp -- "$src/share/icons/hicolor/scalable/apps/$id-active-symbolic.svg" "$journal/outer/3"
cp -- "$src/share/icons/hicolor/scalable/apps/$id-off-symbolic.svg" "$journal/outer/4"
desktop_exec() {
    local value=$1
    value=${value//\\/\\\\}; value=${value//\"/\\\"}
    value=${value//\$/\\\$}; value=${value//\`/\\\`}; value=${value//%/%%}
    value=${value//\\/\\\\}
    printf '"%s"' "$value"
}
for spec in '5 maprisma-applet' '6 maprisma-control-tray'; do
    read -r i binary <<< "$spec"
    if [[ $i == 5 ]]; then template="$src/share/applications/$id.desktop"
    else template="$src/share/autostart/com.maprisma.Control.Tray.desktop"; fi
    { sed '/^Exec=/d' "$template"; printf 'Exec=%s\n' "$(desktop_exec "$folder/$binary")"; } > "$journal/outer/$i"
done
for i in "${!names[@]}"; do
    if [[ $i -lt 2 ]]; then
        printf '%s\tlink\t%s\n' "${names[$i]}" "$(readlink "$journal/outer/$i")"
    else
        chmod 644 "$journal/outer/$i"
        printf '%s\t%s\n' "${names[$i]}" "$(sha256sum "$journal/outer/$i" | cut -d' ' -f1)"
    fi
    if exists "${paths[$i]}"; then cp -a -- "${paths[$i]}" "$journal/backup/$i"; fi
done > "$journal/new/.install-receipt"
[[ $valid == 1 ]] || touch "$journal/no-old"
touch "$journal/ready"
hook staged
for i in "${!names[@]}"; do
    mkdir -p -- "$(dirname -- "${paths[$i]}")"
    cp -a -- "$journal/outer/$i" "${paths[$i]}.maprisma-control-new"
done
touch "$journal/swap-started"
if [[ $valid == 1 ]]; then mv -- "$folder" "$journal/old"; fi
hook old-moved
mv -- "$journal/new" "$folder"
hook swapped
touch "$journal/outer-started"
for i in "${!names[@]}"; do
    mv -Tf -- "${paths[$i]}.maprisma-control-new" "${paths[$i]}"
    hook "outer-$i"
done
touch "$journal/committed"
# The old executable's /proc path follows the rename into the journal.
stop_tray "$journal/old"
discard_journal
trap - EXIT
if [[ -n ${DBUS_SESSION_BUS_ADDRESS:-} && :${XDG_CURRENT_DESKTOP:-}: =~ :(KDE|Hyprland): ]]; then
    setsid -f "$folder/maprisma-control-tray" </dev/null >/dev/null 2>&1 9>&- || true
    sleep 1
    [[ -n $(tray_pids "$folder") ]] || echo 'Could not start tray or tray exited; it will start at login.' >&2
fi
echo 'Installed both COSMIC applet and KDE/Hyprland tray integrations.'
echo 'COSMIC: add it in COSMIC Settings > Desktop > Panel > Applets. An already-placed applet restarts at the next login or when the panel restarts.'
echo 'The tray needs a bar with a system tray.'
if [[ :${XDG_CURRENT_DESKTOP:-}: == *:Hyprland:* ]]; then
    command="$folder/maprisma-control-tray"
    command=${command//\\/\\\\}; command=${command//\"/\\\"}
    command=${command//\$/\\\$}; command=${command//\`/\\\`}
    command="\"$command\""
    echo 'Hyprland without XDG autostart/uwsm: add this to your config (never added automatically):'
    printf 'exec-once = %s\nPlain command: %s\n' "$command" "$command"
    echo 'Lua exec-once syntax is not verified here; use the plain command with your configuration’s startup API.'
fi
