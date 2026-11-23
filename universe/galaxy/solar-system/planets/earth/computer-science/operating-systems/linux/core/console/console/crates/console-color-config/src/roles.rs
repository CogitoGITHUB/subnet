//! Colors come only from the console style roles
//! (text, dim, error, bg). See CONSOLE_STYLE_DIR.
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

fn load() -> Option<HashMap<String, String>> {
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
        if let Some((k, v)) = l.split_once('=') {
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
        return Some(Color::Rgb(r, g, b));
    }
    if v.len() == 7
        && v.starts_with('#')
        && v[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        let r = u8::from_str_radix(&v[1..3], 16).ok()?;
        let g = u8::from_str_radix(&v[3..5], 16).ok()?;
        let b = u8::from_str_radix(&v[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    None
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
pub fn bg() -> Color {
    pick("bg", (0, 0, 0))
}
