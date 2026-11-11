use serde::Deserialize;
use std::{collections::HashSet, env, fs, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Theme {
    #[serde(rename = "$schema")]
    _schema: Option<String>,
    id: String,
    symbol: String,
    family: String,
    name: String,
    mode: String,
    order: u32,
    colors: Option<Colors>,
    inherits: Option<Inherits>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Colors {
    background: String,
    surface: String,
    elevated: String,
    foreground: String,
    muted: String,
    border: String,
    link: String,
    primary: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inherits {
    light: String,
    dark: String,
}
fn color(value: &str) -> u32 {
    assert!(
        value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|c| c.is_ascii_hexdigit()),
        "expected #RRGGBB, got {value:?}"
    );
    u32::from_str_radix(&value[1..], 16).unwrap()
}
fn main() {
    let directory = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../themes");
    println!("cargo:rerun-if-changed={}", directory.display());
    let mut themes = Vec::new();
    let mut paths: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        if path.extension().is_none_or(|ext| ext != "json")
            || path.file_name().unwrap() == "schema.json"
            || path.file_name().unwrap() == "semantic-tags.json"
        {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() <= 16384, "theme too large: {}", path.display());
        let theme: Theme = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            !theme.id.is_empty()
                && theme
                    .id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "invalid theme ID: {}",
            path.display()
        );
        assert!(
            theme
                .symbol
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_uppercase)
                && theme.symbol.bytes().all(|c| c.is_ascii_alphanumeric()),
            "invalid symbol: {}",
            path.display()
        );
        assert!(
            !theme.family.trim().is_empty() && !theme.name.trim().is_empty(),
            "empty theme label: {}",
            path.display()
        );
        assert!(
            matches!(theme.mode.as_str(), "system" | "light" | "dark"),
            "invalid mode: {}",
            path.display()
        );
        assert!(
            theme.family != "Default" || matches!(theme.id.as_str(), "system" | "light" | "dark"),
            "Default family is reserved: {}",
            path.display()
        );
        if theme.id == "light" {
            assert!(theme.mode == "light" && theme.symbol == "Light");
        }
        if theme.id == "dark" {
            assert!(theme.mode == "dark" && theme.symbol == "Dark");
        }
        if theme.mode == "system" {
            assert!(theme.id == "system" && theme.symbol == "System" && theme.colors.is_none());
            let inherits = theme.inherits.as_ref().expect("System requires inherits");
            assert!(inherits.light == "light" && inherits.dark == "dark");
        } else {
            assert!(
                theme.colors.is_some() && theme.inherits.is_none(),
                "theme requires colors: {}",
                path.display()
            );
        }
        themes.push(theme);
    }
    themes.sort_by_key(|theme| (theme.order, theme.id.clone()));
    let mut ids = HashSet::new();
    let mut symbols = HashSet::new();
    let mut code = String::from(
        "#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]\npub enum AppTheme {\n",
    );
    for theme in &themes {
        assert!(
            ids.insert(&theme.id) && symbols.insert(&theme.symbol),
            "duplicate theme identity"
        );
        code.push_str(&format!(
            "#[serde(rename = {:?})] {},\n",
            theme.id, theme.symbol
        ));
    }
    code.push_str("}\nimpl AppTheme {\npub const ALL: &'static [Self] = &[");
    for theme in &themes {
        code.push_str(&format!("Self::{},", theme.symbol));
    }
    code.push_str("];\n");
    for (method, field) in [("key", "id"), ("label", "name"), ("family", "family")] {
        code.push_str(&format!(
            "pub const fn {method}(self) -> &'static str {{ match self {{"
        ));
        for theme in &themes {
            let value = match field {
                "id" => &theme.id,
                "name" => &theme.name,
                _ => &theme.family,
            };
            code.push_str(&format!("Self::{} => {:?},", theme.symbol, value));
        }
        code.push_str("}}\n");
    }
    code.push_str("pub const fn is_dark(self) -> Option<bool> { match self {");
    for theme in &themes {
        code.push_str(&format!(
            "Self::{} => {},",
            theme.symbol,
            match theme.mode.as_str() {
                "dark" => "Some(true)",
                "light" => "Some(false)",
                _ => "None",
            }
        ));
    }
    code.push_str("}}\npub fn from_key(key: &str) -> Option<Self> { Self::ALL.iter().copied().find(|theme| theme.key() == key) }\n");
    code.push_str("pub const fn palette_colors(self) -> Option<[u32; 8]> { match self {");
    for theme in &themes {
        code.push_str(&format!("Self::{} => ", theme.symbol));
        if let Some(c) = &theme.colors {
            let values = [
                &c.background,
                &c.surface,
                &c.elevated,
                &c.foreground,
                &c.muted,
                &c.border,
                &c.link,
                &c.primary,
            ]
            .map(|v| color(v));
            code.push_str(&format!("Some({values:?}),"));
        } else {
            code.push_str("None,");
        }
    }
    code.push_str("}}\n}\n");
    for required in ["system", "light", "dark"] {
        assert!(
            ids.contains(&required.to_string()),
            "missing {required} theme"
        );
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SemanticMode {
        danger: [String; 2],
        success: [String; 2],
        warning: [String; 2],
        info: [String; 2],
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct SemanticTags {
        light: SemanticMode,
        dark: SemanticMode,
    }
    let tags: SemanticTags = serde_json::from_slice(
        &fs::read(directory.join("semantic-tags.json")).expect("semantic-tags.json"),
    )
    .expect("invalid themes/semantic-tags.json");
    for (name, mode) in [
        ("LIGHT_TAG_COLORS", tags.light),
        ("DARK_TAG_COLORS", tags.dark),
    ] {
        let values = [mode.danger, mode.success, mode.warning, mode.info]
            .map(|pair| pair.map(|value| color(&value)));
        code.push_str(&format!("pub const {name}: [[u32; 2]; 4] = {values:?};\n"));
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("themes.rs"),
        code,
    )
    .unwrap();
}
