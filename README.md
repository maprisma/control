# Maprisma Control for COSMIC

A COSMIC panel applet that starts and stops Maprisma (`maprisma-applet`; `maprisma-applet --window` opens the same menu in a window, and `maprisma-control-tray` is the KDE tray). Interface languages: see [docs/translations.md](docs/translations.md). Tests: `cargo test --offline`.

## F12: a screenshot with a note

While the popup (or the `--window` window) has the keyboard, **F12** (without Ctrl, Shift, Alt or Super) saves a picture of it, for telling what is wrong with Control itself (Maprisma's own F12 is separate):

- **Only on F12, and only that popup or window:** iced draws it once more into the picture (`window::screenshot`, encoded as PNG in the applet). No screenshot program runs, nothing else on the screen is in it, and nothing is uploaded or sent anywhere. The KDE tray's menu is drawn by Plasma and is not captured.
- **Where:** `~/Pictures/Maprisma Control/maprisma-control-<date>-<time>.png`. The pictures folder comes from `~/.config/user-dirs.dirs` when it is set there (read only as a regular file of at most 64 KiB, never through a link). The app then names the file and asks **What is wrong? (optional)**; **Enter** or **Done** saves the text next to the picture as a `.txt` with the same name (at most 4000 characters; no text, no file), **Esc** skips it, and closing the popup keeps what was typed. F12 waits until that is closed.
- **Private files:** the folder is made `0700` when missing and only used when it is a real folder of yours (not a link) that no one else can write to, checked on the opened folder; pictures and notes are new `0600` files made inside that folder, never through a link and never over an existing file (`-2`, `-3` … are added instead, also when only a note has the name), synced to disk, and removed again when they cannot be written completely.
- **Off:** choose **F12 saves a screenshot** in the menu (the check mark shows it is on); stored as `screenshot_key=false` in `$XDG_CONFIG_HOME/maprisma-control/settings.conf` (next to `language=`).

## Corners

**Corners** in the menu: *Like the desktop (N px)* (the default: COSMIC's window corners, the theme's small radius plus 4 px as libcosmic rounds windows; 12 px where there is no COSMIC theme, e.g. on KDE), *Slightly rounded (6 px)*, *Rounded (12 px)* or *Square*. Popups, cards, the note card and drop-down menus take the full radius; buttons, drop-downs and text fields at most 8 px; small parts (bars and their tracks, slider tracks, check boxes) at most 4 px; round dots stay round, or square in *Square*. libcosmic's widgets take their corners from the theme, so the popup runs on a copy of the desktop's COSMIC theme with these corners (made again when the desktop's theme changes). libcosmic draws toggle switches and progress bars with the same theme slot as buttons, so they get the button corners (at most 8 px) rather than pills; slider knobs take the full radius. The panel button keeps the panel's look. Stored as `corners=desktop|slight|rounded|square` in `settings.conf`; a file without it stays valid and means *Like the desktop*.

MIT license.
