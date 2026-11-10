Maprisma Control includes English, Deutsch, Français, Español, Italiano,
Português, Nederlands and Polski. Choose Language in the COSMIC popup,
`maprisma-applet --window`, or the KDE tray submenu. The check mark identifies
the saved choice. Changes apply immediately; other running Control instances
read the shared setting on their next refresh (normally within two seconds).

The setting is `language=auto` or an explicit language code in
`$XDG_CONFIG_HOME/maprisma-control/settings.conf` (default:
`~/.config/maprisma-control/settings.conf`). Auto uses the first non-empty
LC_ALL, LC_MESSAGES, LANG, with unsupported locales falling back to English.
No setting file is needed until a choice is saved.

Control has no Omarchy original. The base translations/<code>.json catalogs
are therefore empty; translations/extra/<code>.json contains all 21 Control
keys per language. Matching terms follow the read-only reference catalogs in
maprisma-playground/translations. Product names, paths, versions, times and
native language names remain data. Python helpers are unchanged.

Use `tr("English text")` or `trf("Text with {0}", &[&value])`. Unknown keys
remain English. Add all seven translations, then run `tools/gen-translations`.
The generated Rust source is compiled into both binaries and needs no runtime
assets or downloads. `tools/gen-translations --check` scans literal keys and
desktop metadata, checks all seven catalogs and placeholders, and rejects stale
generated source. `cargo test --offline` runs that check along with locale and
settings persistence tests.
