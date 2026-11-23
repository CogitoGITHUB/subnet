use {
    crokey::crossterm::style::Color,
    std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::OnceLock,
    },
};

fn dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("CONSOLE_STYLE_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    let h = std::env::var("HOME").ok()?;
    let f = Path::new(&h).join(".config/run/style-dir");
    let d = std::fs::read_to_string(f).ok()?;
    let p = PathBuf::from(d.trim());
    if p.is_dir() { Some(p) } else { None }
}

fn roles() -> Option<HashMap<String, String>> {
    let d = dir()?;
    let n = std::fs::read_to_string(d.join("active")).ok()?;
    let n = n.trim();
    if n.is_empty() {
        return None;
    }
    if !n.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return None;
    }
    let s = std::fs::read_to_string(d.join(format!("{n}.style"))).ok()?;
    let mut m = HashMap::new();
    for l in s.lines() {
        if let Some((k, v)) = l.split_once("=") {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    Some(m)
}

fn parse(v: &str) -> Option<Color> {
    let named = match v {
        "black" => Some((0, 0, 0)),
        "red" => Some((220, 40, 40)),
        "green" => Some((40, 200, 40)),
        "yellow" => Some((220, 200, 40)),
        "blue" => Some((60, 100, 230)),
        "magenta" => Some((200, 60, 200)),
        "cyan" => Some((40, 200, 200)),
        "white" => Some((255, 255, 255)),
        "gray" | "grey" => Some((110, 110, 110)),
        _ => None,
    };
    if let Some((r, g, b)) = named {
        return Some(Color::Rgb { r, g, b });
    }
    if v.len() == 7
        && v.starts_with("#")
        && v[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        let r = u8::from_str_radix(&v[1..3], 16).ok()?;
        let g = u8::from_str_radix(&v[3..5], 16).ok()?;
        let b = u8::from_str_radix(&v[5..7], 16).ok()?;
        return Some(Color::Rgb { r, g, b });
    }
    None
}

struct Roles {
    tx: Option<Color>,
    dm: Option<Color>,
    er: Option<Color>,
    bg: Option<Color>,
}

fn load() -> &'static Roles {
    static R: OnceLock<Roles> = OnceLock::new();
    R.get_or_init(|| {
        let m = roles().unwrap_or_default();
        let get = |k: &str, d: &str| {
            parse(m.get(k).map(|s| s.as_str()).unwrap_or(d))
        };
        Roles {
            tx: get("text", "white"),
            dm: get("dim", "#6e6e6e"),
            er: get("error", "red"),
            bg: get("bg", "none"),
        }
    })
}

/// The four style roles are the only color source of the app.
pub fn tx() -> Option<Color> {
    load().tx
}
pub fn dm() -> Option<Color> {
    load().dm
}
pub fn er() -> Option<Color> {
    load().er
}
pub fn bgc() -> Option<Color> {
    load().bg
}

/// dim role as a plain color (fallback for missing styles)
pub fn dmc() -> Color {
    dm().unwrap_or(Color::Reset)
}
/// text role as a plain color (fallback for missing styles)
pub fn txc() -> Color {
    tx().unwrap_or(Color::Reset)
}

fn hexc(c: Option<Color>, d: &str) -> String {
    match c {
        Some(Color::Rgb { r, g, b }) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        _ => d.to_string(),
    }
}

/// Syntax theme in tmTheme form, built only from the four style roles.
pub fn tm_theme_xml() -> String {
    format!(
        r##"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>name</key><string>style</string>
<key>settings</key><array>
<dict><key>settings</key><dict>
<key>foreground</key><string>{tx}</string>
<key>background</key><string>{bg}</string>
</dict></dict>
<dict><key>scope</key><string>comment</string><key>settings</key><dict>
<key>foreground</key><string>{dm}</string></dict></dict>
<dict><key>scope</key><string>invalid</string><key>settings</key><dict>
<key>foreground</key><string>{er}</string></dict></dict>
</array></dict></plist>"##,
        tx = hexc(tx(), "#ffffff"),
        dm = hexc(dm(), "#6e6e6e"),
        er = hexc(er(), "#dc2828"),
        bg = hexc(bgc(), "#000000"),
    )
}
