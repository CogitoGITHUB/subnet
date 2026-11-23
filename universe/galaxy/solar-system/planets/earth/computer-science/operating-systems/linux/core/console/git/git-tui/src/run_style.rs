use crate::color::ColorTheme;
use ratatui::style::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
        return Some(Color::Rgb(r, g, b));
    }
    if v.len() == 7
        && v.starts_with("#")
        && v[1..].chars().all(|c| c.is_ascii_hexdigit())
    {
        let r = u8::from_str_radix(&v[1..3], 16).ok()?;
        let g = u8::from_str_radix(&v[3..5], 16).ok()?;
        let b = u8::from_str_radix(&v[5..7], 16).ok()?;
        return Some(Color::Rgb(r, g, b));
    }
    None
}

fn hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        _ => String::from("#000000"),
    }
}

struct Roles {
    tx: Color,
    dm: Color,
    er: Color,
    bg: Color,
}

fn load_roles() -> Roles {
    let m = roles().unwrap_or_default();
    let get = |k: &str, d: &str| parse(m.get(k).map(|s| s.as_str()).unwrap_or(d));
    Roles {
        tx: get("text", "white").unwrap_or(Color::White),
        dm: get("dim", "#6e6e6e").unwrap_or(Color::DarkGray),
        er: get("error", "red").unwrap_or(Color::Red),
        bg: get("bg", "none").unwrap_or(Color::Reset),
    }
}

/// Syntax theme in tmTheme form, built only from the four style roles.
pub fn tm_theme_xml() -> String {
    let r = load_roles();
    let bg = if r.bg == Color::Reset { String::from("#000000") } else { hex(r.bg) };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
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
</array></dict></plist>"#,
        tx = hex(r.tx),
        dm = hex(r.dm),
        er = hex(r.er),
        bg = bg
    )
}

/// Set every color of the theme from the four style roles.
pub fn apply(t: &mut ColorTheme) -> bool {
    let r = load_roles();
    let (tx, dm, er, bg) = (r.tx, r.dm, r.er, r.bg);
    t.fg = tx;
    t.bg = bg;
    t.list_selected_fg = tx;
    t.list_selected_bg = dm;
    t.list_compare_marked_fg = tx;
    t.list_compare_marked_bg = dm;
    t.list_ref_paren_fg = dm;
    t.list_ref_branch_fg = tx;
    t.list_ref_remote_branch_fg = dm;
    t.list_ref_tag_fg = tx;
    t.list_ref_stash_fg = dm;
    t.list_head_fg = tx;
    t.list_commit_message_fg = tx;
    t.list_name_fg = tx;
    t.list_hash_fg = dm;
    t.list_date_fg = dm;
    t.list_match_fg = bg;
    t.list_match_bg = tx;
    t.detail_label_fg = dm;
    t.detail_name_fg = tx;
    t.detail_date_fg = dm;
    t.detail_email_fg = dm;
    t.detail_hash_fg = dm;
    t.detail_ref_branch_fg = tx;
    t.detail_ref_remote_branch_fg = dm;
    t.detail_ref_tag_fg = tx;
    t.detail_file_change_add_fg = tx;
    t.detail_file_change_modify_fg = dm;
    t.detail_file_change_delete_fg = er;
    t.detail_file_change_move_fg = dm;
    t.ref_selected_fg = tx;
    t.ref_selected_bg = dm;
    t.help_block_title_fg = tx;
    t.help_key_fg = tx;
    t.virtual_cursor_fg = tx;
    t.status_input_fg = tx;
    t.status_input_transient_fg = dm;
    t.status_info_fg = tx;
    t.status_success_fg = tx;
    t.status_warn_fg = tx;
    t.status_error_fg = er;
    t.divider_fg = dm;
    t.graph_branches = vec![hex(tx), hex(dm)];
    true
}

#[allow(dead_code)]
fn roles4() -> (Color, Color, Color, Color) {
    static R: std::sync::OnceLock<(Color, Color, Color, Color)> =
        std::sync::OnceLock::new();
    *R.get_or_init(|| {
        let r = load_roles();
        (r.tx, r.dm, r.er, r.bg)
    })
}

#[allow(dead_code)]
pub fn role_text() -> Color {
    roles4().0
}

#[allow(dead_code)]
pub fn role_dim() -> Color {
    roles4().1
}

#[allow(dead_code)]
pub fn role_error() -> Color {
    roles4().2
}

#[allow(dead_code)]
pub fn role_bg() -> Color {
    roles4().3
}
