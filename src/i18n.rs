//! Offline gettext-style UI translations shared by the applet and tray.
use std::path::PathBuf;

#[path = "translations_generated.rs"]
mod generated;
pub const CODES: [&str; 9] = ["auto", "en", "de", "fr", "es", "it", "pt", "nl", "pl"];
const NAMES: [&str; 8] = ["English", "Deutsch", "Français", "Español", "Italiano", "Português", "Nederlands", "Polski"];

pub fn system_language_from(values: [&str; 3]) -> &'static str {
    let locale = values.into_iter().find(|v| !v.is_empty()).unwrap_or("en");
    let code = locale.split(['_', '.', '-', '@']).next().unwrap_or("en");
    CODES[1..].iter().copied().find(|c| *c == code).unwrap_or("en")
}
fn system_language() -> &'static str {
    let values = ["LC_ALL", "LC_MESSAGES", "LANG"].map(|key| std::env::var(key).unwrap_or_default());
    system_language_from([&values[0], &values[1], &values[2]])
}
fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()).map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"))
        .join("maprisma-control/settings.conf")
}
pub fn preference() -> String {
    preference_at(&config_path())
}
fn preference_at(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).ok().and_then(|s| {
        s.lines().find_map(|line| line.strip_prefix("language=").map(str::to_owned))
    }).filter(|s| CODES.contains(&s.as_str())).unwrap_or_else(|| "auto".into())
}
fn language() -> String {
    let preference = preference();
    if preference == "auto" { system_language().into() } else { preference }
}
pub fn set_language(index: usize) -> std::io::Result<()> {
    save_language(&config_path(), index)
}
fn save_language(path: &std::path::Path, index: usize) -> std::io::Result<()> {
    let Some(code) = CODES.get(index) else { return Ok(()) };
    std::fs::create_dir_all(path.parent().unwrap())?;
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<_> = old.lines().filter(|l| !l.starts_with("language=")).map(str::to_owned).collect();
    lines.push(format!("language={code}"));
    // Same-directory rename keeps readers from seeing a partial setting.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, lines.join("\n") + "\n")?;
    std::fs::rename(temporary, path)
}
pub fn tr(key: &str) -> String {
    generated::lookup(&language(), key).unwrap_or(key).to_owned()
}
pub fn trf(key: &str, values: &[&str]) -> String {
    // Replace placeholders in one pass, so values containing {n} remain data.
    let text = tr(key);
    let mut result = String::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        rest = &rest[start..];
        if let Some(end) = rest.find('}') {
            if let Ok(index) = rest[1..end].parse::<usize>() {
                if let Some(value) = values.get(index) {
                    result.push_str(value);
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        result.push('{');
        rest = &rest[1..];
    }
    result.push_str(rest);
    result
}
pub fn language_options() -> Vec<String> {
    let system = system_language();
    let name = NAMES[CODES[1..].iter().position(|c| *c == system).unwrap_or(0)];
    let mut options = vec![trf("System ({0})", &[name])];
    options.extend(NAMES.iter().map(|s| s.to_string()));
    options
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locale_precedence_and_fallback() {
        for (values, expected) in [
            (["de_DE.UTF-8", "fr", "pl"], "de"),
            (["", "fr_FR.UTF-8", "pl"], "fr"),
            (["", "", "pt_BR.UTF-8"], "pt"),
            (["unknown", "de", "fr"], "en"),
            (["C.UTF-8", "de", "fr"], "en"),
            (["", "", ""], "en"),
        ] { assert_eq!(system_language_from(values), expected); }
    }
    #[test]
    fn preference_persists_and_unknown_values_follow_system() {
        let directory = std::env::temp_dir().join(format!("maprisma-language-{}", std::process::id()));
        let path = directory.join("settings.conf");
        assert_eq!(preference_at(&path), "auto");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&path, "other=value\nlanguage=unknown\n").unwrap();
        assert_eq!(preference_at(&path), "auto");
        for (index, code) in CODES.iter().enumerate() {
            save_language(&path, index).unwrap();
            assert_eq!(preference_at(&path), *code);
            assert!(std::fs::read_to_string(&path).unwrap().contains("other=value"));
        }
        assert_eq!(generated::lookup("en", "Language"), None);
        assert_eq!(generated::lookup("de", "Language"), Some("Sprache"));
        assert_eq!(generated::lookup("de", "Uncatalogued"), None);
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn catalogs_cover_source_keys_and_are_current() {
        assert!(std::process::Command::new("python3")
            .args(["tools/gen-translations", "--check"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status().unwrap().success());
    }
}
