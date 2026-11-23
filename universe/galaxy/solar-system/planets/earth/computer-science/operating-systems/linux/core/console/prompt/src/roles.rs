//! Colors come only from the console style roles.
#![allow(dead_code)]
use nu_ansi_term::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

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

fn ok_name(n: &str) -> bool {
    !n.is_empty()
        && n.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_'
        })
}

fn load() -> Option<HashMap<String, String>> {
    let d = dir()?;
    let n = std::fs::read_to_string(d.join("active")).ok()?;
    let n = n.trim();
    if !ok_name(n) {
        return None;
    }
    let f = d.join(format!("{n}.style"));
    let s = std::fs::read_to_string(f).ok()?;
    let mut m = HashMap::new();
    for l in s.lines() {
        if let Some((k, v)) = l.split_once('=') {
            m.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    Some(m)
}

fn hex(v: &str) -> Option<Color> {
    if v.len() != 7 || !v.starts_with('#') || !v.is_ascii() {
        return None;
    }
    let p = |a: usize| u8::from_str_radix(&v[a..a + 2], 16).ok();
    Some(Color::Rgb(p(1)?, p(3)?, p(5)?))
}

fn parse(v: &str) -> Option<Color> {
    let (r, g, b) = match v {
        "black" => (0, 0, 0),
        "red" => (220, 40, 40),
        "green" => (40, 200, 40),
        "yellow" => (220, 200, 40),
        "blue" => (60, 100, 230),
        "magenta" => (200, 60, 200),
        "cyan" => (40, 200, 200),
        "white" => (255, 255, 255),
        "gray" | "grey" => (110, 110, 110),
        _ => return hex(v),
    };
    Some(Color::Rgb(r, g, b))
}

fn map() -> &'static HashMap<String, String> {
    static M: OnceLock<HashMap<String, String>> = OnceLock::new();
    M.get_or_init(|| load().unwrap_or_default())
}

fn pick(k: &str, d: (u8, u8, u8)) -> Color {
    map()
        .get(k)
        .and_then(|v| parse(v))
        .unwrap_or(Color::Rgb(d.0, d.1, d.2))
}

pub fn text() -> Color {
    pick("text", (255, 255, 255))
}
pub fn dim() -> Color {
    pick("dim", (110, 110, 110))
}
pub fn err() -> Color {
    pick("error", (220, 40, 40))
}
