//! Colors only from console style roles (text, dim,
//! error, bg). Style dir: CONSOLE_STYLE_DIR.
#![allow(dead_code)]
use simplelog::Color;
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
    let ok = |c: char| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    };
    if n.is_empty() || !n.chars().all(ok) {
        return None;
    }
    let f = d.join(format!("{n}.style"));
    let s = std::fs::read_to_string(f).ok()?;
    let mut m = HashMap::new();
    for l in s.lines() {
        if let Some((k, v)) = l.split_once('=') {
            m.insert(k.trim().into(), v.trim().into());
        }
    }
    Some(m)
}

fn parse(v: &str) -> Option<(u8, u8, u8)> {
    let named = match v {
        "black" => Some((0, 0, 0)),
        "red" => Some((220, 40, 40)),
        "white" => Some((255, 255, 255)),
        "gray" | "grey" => Some((110, 110, 110)),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    let h = v.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}

fn map() -> &'static HashMap<String, String> {
    static M: OnceLock<HashMap<String, String>> = OnceLock::new();
    M.get_or_init(|| load().unwrap_or_default())
}

fn pick(k: &str, d: (u8, u8, u8)) -> Color {
    let c = map().get(k).and_then(|v| parse(v)).unwrap_or(d);
    Color::Rgb(c.0, c.1, c.2)
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
