use super::normalize_theme_source;
use std::path::Path;

#[test]
fn imports_vscode_theme_json() {
    let source = r##"{
        "name": "VS Code Test",
        "type": "light",
        "colors": {
            "editor.background": "#ffffff",
            "editor.foreground": "#111111",
            "button.background": "#0066cc"
        }
    }"##;
    let normalized = normalize_theme_source(Path::new("test.json"), source).unwrap();
    assert!(normalized.contains("VS Code Test"));
    assert!(normalized.contains("#ffffff"));
}

#[test]
fn imports_alacritty_toml() {
    let source = r##"
        [colors.primary]
        background = "#101010"
        foreground = "#eeeeee"
        [colors.cursor]
        cursor = "#ffcc00"
    "##;
    let normalized = normalize_theme_source(Path::new("test.toml"), source).unwrap();
    assert!(normalized.contains("#101010"));
    assert!(normalized.contains("#ffcc00"));
}

#[test]
fn imports_alacritty_yaml() {
    let source = r##"
colors:
  primary:
    background: "#fafafa"
    foreground: "#202020"
  selection:
    background: "#cccccc"
"##;
    let normalized = normalize_theme_source(Path::new("test.yml"), source).unwrap();
    assert!(normalized.contains("#fafafa"));
    assert!(normalized.contains("\"mode\": \"light\""));
}

#[test]
fn imports_iterm2_plist() {
    let source = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Background Color</key><dict>
    <key>Red Component</key><real>0.1</real>
    <key>Green Component</key><real>0.2</real>
    <key>Blue Component</key><real>0.3</real>
  </dict>
  <key>Foreground Color</key><dict>
    <key>Red Component</key><real>0.9</real>
    <key>Green Component</key><real>0.8</real>
    <key>Blue Component</key><real>0.7</real>
  </dict>
</dict></plist>"#;
    let normalized = normalize_theme_source(Path::new("Ocean.itermcolors"), source).unwrap();
    assert!(normalized.contains("Ocean"));
    assert!(normalized.contains("#1A334D"));
}
