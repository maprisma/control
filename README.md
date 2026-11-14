# Maprisma Control for COSMIC

A COSMIC panel applet that starts and stops Maprisma (`maprisma-applet`; `maprisma-applet --window` opens the same menu in a window, and `maprisma-control-tray` is the KDE tray; its robot is green while Maprisma runs and plain when it is stopped). Interface languages: see [docs/translations.md](docs/translations.md). Tests: `cargo test --offline`.

Control finds Maprisma in this order: executable `~/.local/bin/maprisma`, executable `maprisma` in absolute directories on `PATH`, the launch record `$XDG_STATE_HOME/maprisma/launch.json` (default `~/.local/state/maprisma/launch.json`), then `/usr/bin/maprisma`. Maprisma writes the record when it starts, allowing Control to find portable bundles and AppImages after their first launch. Control validates the record and executable ownership and permissions before using it; extraction-mode AppImages receive `--appimage-extract-and-run` for Start and Toggle. Test-session launches pass only the discovered executable path to the development `start.sh`.

## F12: a screenshot with a note

While the popup (or the `--window` window) has the keyboard, **F12** (without Ctrl, Shift, Alt or Super) saves a picture of it, for telling what is wrong with Control itself (Maprisma's own F12 is separate):

- **Only on F12, and only that popup or window:** iced draws it once more into the picture (`window::screenshot`, encoded as PNG in the applet). No screenshot program runs, nothing else on the screen is in it, and nothing is uploaded or sent anywhere. The KDE tray's menu is drawn by Plasma and is not captured.
- **Where:** `~/Pictures/Maprisma Control/maprisma-control-<date>-<time>.png`. The pictures folder comes from `~/.config/user-dirs.dirs` when it is set there (read only as a regular file of at most 64 KiB, never through a link). The app then names the file and asks **What is wrong? (optional)**; **Enter** or **Done** saves the text next to the picture as a `.txt` with the same name (at most 4000 characters; no text, no file), **Esc** skips it, and closing the popup keeps what was typed. F12 waits until that is closed.
- **Private files:** the folder is made `0700` when missing and only used when it is a real folder of yours (not a link) that no one else can write to, checked on the opened folder; pictures and notes are new `0600` files made inside that folder, never through a link and never over an existing file (`-2`, `-3` … are added instead, also when only a note has the name), synced to disk, and removed again when they cannot be written completely.
- **Off:** choose **F12 saves a screenshot** in the menu (the check mark shows it is on); stored as `screenshot_key=false` in `$XDG_CONFIG_HOME/maprisma-control/settings.conf` (next to `language=`).

## Corners

**Corners** in the menu: *Like the desktop (N px)* (the default: the COSMIC theme exactly as it is, the native look; N is the theme's medium corner radius, 16, 8 or 2 px by COSMIC Settings › Appearance, 16 px where there is no COSMIC theme, e.g. on KDE), *Slightly rounded (6 px)*, *Rounded (12 px)* or *Square*, as in the other Maprisma COSMIC apps. With one of the last three, popups, cards, the note card and drop-down menus take that radius; drop-downs, text fields and menu rows at most 8 px; small parts (bars and their tracks, slider tracks, check boxes) at most 4 px; pills (libcosmic's buttons and toggles, round dots) stay round, or square in *Square*. libcosmic's widgets take their corners from the theme, so the popup's content is drawn with a copy of the active theme with these corner radii (iced's `themer`); slider knobs take the full radius. The panel button keeps the panel's look. Stored as `corners=desktop|slight|rounded|square` in `settings.conf`; a file without it stays valid and means *Like the desktop*.

MIT license.

## Install from a release

Download `maprisma-control-<version>-x86_64.tar.gz` and its `.sha256` file from
[Releases](https://github.com/maprisma/control/releases). In the download directory,
replace `<version>` in these commands with the downloaded version:

```sh
sha256sum -c maprisma-control-<version>-x86_64.tar.gz.sha256
tar -xzf maprisma-control-<version>-x86_64.tar.gz
cd maprisma-control-<version>
./install.sh
```

This is a per-user installation for x86_64 Linux with GNU glibc (the archive records
its required version, at most 2.39). It checks both binaries' runtime libraries and
prints distribution package hints if dependencies are missing. No Cargo or Rust is
needed. The installer requires Bash, coreutils, `ldd`, and `flock` (util-linux).

It installs the binaries and uninstaller in `~/.local/lib/maprisma-control`, command
links in `~/.local/bin`, three symbolic icons and the COSMIC applet entry under
`${XDG_DATA_HOME:-~/.local/share}`, and the KDE/Hyprland tray autostart entry under
`${XDG_CONFIG_HOME:-~/.config}/autostart`. Both integrations are always installed;
the tray autostart is limited to KDE and Hyprland, and the applet entry is for COSMIC.
Re-running the installer upgrades through a rollback journal and refuses to overwrite
foreign or modified integration files. Keep the same XDG directories when upgrading
or uninstalling.

In **COSMIC**, add Control in **COSMIC Settings > Desktop > Panel > Applets**.
An already-placed applet restarts at the next login or when the panel restarts.
The installer never restarts the panel. In **KDE Plasma**, the tray starts now when a
session bus is available, and at subsequent logins. **Hyprland** needs a bar with a
system tray. With XDG autostart (for example uwsm), the tray starts at login; otherwise
use the installer's absolute quoted `exec-once` command in your configuration.
It prints the hyprlang form and plain command; Lua startup syntax is not verified.
The installer never edits your Hyprland config.

Uninstall with `~/.local/lib/maprisma-control/uninstall.sh` (or `./uninstall.sh`
in the extracted archive). Add `--purge` to also remove
`${XDG_CONFIG_HOME:-~/.config}/maprisma-control`. Modified integration files are
kept and reported. An invalid receipt prevents removal.

**Maprisma Settings users:** keep updating and removing Control through Settings.
This installer refuses installations managed by Settings (including its old
`maprisma-manager` receipt directory).

For development, the repository's existing `./install.sh` still builds from source
with Cargo. Release maintainers use `tools/build-release`: it vendors already cached
Cargo sources offline and builds in an Ubuntu 24.04 container with the host's exact
Rust version (libcosmic requires Rust 1.93 or newer). The first builder preparation
requires `--allow-bootstrap-network` for apt/rustup downloads; subsequent runs reuse
the cached builder and compilation always runs with networking disabled. The default
permits only Podman image pulls and otherwise refuses uncached dependencies. Populate
the host Cargo cache separately before building. `--no-container` is a local test
build, marked in its filename; it still fails if either binary exceeds GLIBC 2.39.
