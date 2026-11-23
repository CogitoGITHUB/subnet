#![allow(dead_code)]
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

type Rgb = (u8, u8, u8);

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

fn parse(v: &str) -> Option<Rgb> {
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
    if named.is_some() {
        return named;
    }
    if v.len() == 7
        && v.starts_with("#")
        && v[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        let r = u8::from_str_radix(&v[1..3], 16).ok()?;
        let g = u8::from_str_radix(&v[3..5], 16).ok()?;
        let b = u8::from_str_radix(&v[5..7], 16).ok()?;
        return Some((r, g, b));
    }
    None
}

#[derive(Clone, Copy)]
struct Cfg {
    on: bool,
    text: Option<Rgb>,
    dim: Option<Rgb>,
    err: Rgb,
}

fn cfg() -> Cfg {
    static C: OnceLock<Cfg> = OnceLock::new();
    *C.get_or_init(|| {
        let m = roles().unwrap_or_default();
        let get = |k: &str| m.get(k).and_then(|s| parse(s));
        let tty = std::io::stdout().is_terminal()
            || std::io::stderr().is_terminal();
        Cfg {
            on: tty && std::env::var_os("NO_COLOR").is_none(),
            text: get("text"),
            dim: get("dim"),
            err: get("error").unwrap_or((220, 40, 40)),
        }
    })
}

fn paint(s: &str, c: Option<Rgb>, attr: &str) -> String {
    if !cfg().on {
        return s.to_string();
    }
    let col = match c {
        Some((r, g, b)) => format!("\x1b[38;2;{r};{g};{b}m"),
        None => String::new(),
    };
    format!("{attr}{col}{s}\x1b[0m")
}

/// Colors come only from the style roles text, dim, error.
pub trait Paint {
    fn text(&self) -> String;
    fn dim(&self) -> String;
    fn err(&self) -> String;
    fn bold(&self) -> String;
}

impl<T: AsRef<str> + ?Sized> Paint for T {
    fn text(&self) -> String {
        paint(self.as_ref(), cfg().text, "")
    }
    fn dim(&self) -> String {
        match cfg().dim {
            Some(c) => paint(self.as_ref(), Some(c), ""),
            None => paint(self.as_ref(), None, "\x1b[2m"),
        }
    }
    fn err(&self) -> String {
        paint(self.as_ref(), Some(cfg().err), "")
    }
    fn bold(&self) -> String {
        paint(self.as_ref(), None, "\x1b[1m")
    }
}
