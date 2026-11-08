use std::process::Command;

#[test]
fn desktop_exec_quotes_both_layers_and_percent_codes() {
    let installer = include_str!("../install.sh");
    let start = installer.find("desktop_exec() {").unwrap();
    let end = start + installer[start..].find("\n}\n").unwrap() + 3;
    let function = &installer[start..end];
    let path = "/tmp/a space/quote\" dollar$ tick` slash\\ percent%/maprisma-control-tray";
    let output = Command::new("bash")
        .args([
            "-c",
            &format!("{function}\ndesktop_exec \"$1\""),
            "test",
            path,
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let encoded = String::from_utf8(output.stdout).unwrap();
    assert!(encoded.starts_with('"') && encoded.ends_with('"'));
    // Decode desktop value escaping, then quoted Exec argument escaping.
    fn unescape(value: &str) -> String {
        let mut chars = value.chars();
        let mut out = String::new();
        while let Some(c) = chars.next() {
            out.push(if c == '\\' { chars.next().unwrap() } else { c });
        }
        out
    }
    let argument = unescape(&encoded);
    let decoded = unescape(&argument[1..argument.len() - 1]).replace("%%", "%");
    assert_eq!(decoded, path);
    assert!(encoded.contains("%%"));
    assert!(include_str!("../data/com.maprisma.Control.Tray.desktop").contains("OnlyShowIn=KDE;"));
}
