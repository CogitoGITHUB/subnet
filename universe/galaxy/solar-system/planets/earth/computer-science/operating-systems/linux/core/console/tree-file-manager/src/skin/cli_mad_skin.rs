use {
    super::roles::{bgc, dm, tx},
    termimad::MadSkin,
};

/// build a termimad skin for cli output (mostly
/// for the install process); colors come from the style roles
pub fn make_cli_mad_skin() -> MadSkin {
    let mut skin = MadSkin::default();
    if let Some(c) = tx() {
        skin.set_headers_fg(c);
        skin.inline_code.set_fg(c);
        skin.code_block.set_fg(c);
    }
    if let Some(c) = bgc() {
        skin.inline_code.set_bg(c);
        skin.code_block.set_bg(c);
    }
    if let Some(c) = dm() {
        skin.italic.set_fg(c);
    }
    skin
}
