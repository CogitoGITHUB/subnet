use std::path::PathBuf;

use clap::ValueEnum;

const BASH: &str = include_str!("shells/bash.sh");
const ZSH: &str = include_str!("shells/zsh.sh");
const FISH: &str = include_str!("shells/fish.fish");
const POWERSHELL: &str = include_str!("shells/powershell.ps1");
const NUSHELL: &str = include_str!("shells/nushell.nu");

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    Powershell,
    #[value(alias = "nu")]
    Nushell,
}

impl Shell {
    pub fn as_str(self) -> &'static str {
        match self {
            Shell::Bash => "bash",
            Shell::Zsh => "zsh",
            Shell::Fish => "fish",
            Shell::Powershell => "powershell",
            Shell::Nushell => "nushell",
        }
    }
}

pub fn init_snippet(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => BASH,
        Shell::Zsh => ZSH,
        Shell::Fish => FISH,
        Shell::Powershell => POWERSHELL,
        Shell::Nushell => NUSHELL,
    }
}

/// Best-effort guess from `$SHELL` (or `$PSModulePath` for Windows PowerShell). Returns
/// `None` when neither env var points to a recognised shell so callers can fall back to
/// an interactive picker or fail with a clear message.
pub fn detect_shell(env: impl Fn(&str) -> Option<String>) -> Option<Shell> {
    if let Some(shell) = env("SHELL").as_deref().and_then(classify_shell_path) {
        return Some(shell);
    }
    // PowerShell typically doesn't set `$SHELL`; this env var is a strong signal we're
    // running under it (present on both Windows PowerShell and PowerShell 7 sessions).
    if env("PSModulePath").is_some() {
        return Some(Shell::Powershell);
    }
    None
}

fn classify_shell_path(path: &str) -> Option<Shell> {
    // Strip the directory prefix by splitting on either separator ourselves — Rust's
    // `Path` is platform-aware (`\` isn't a separator on Unix), so using it here would
    // leave `C:\...\pwsh.exe` as one opaque segment on Linux tests and in WSL. Split on
    // both `/` and `\` then trim the trailing `.exe` so `zsh.exe` / `pwsh.exe` classify
    // on every platform.
    let basename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let bare = basename
        .split('.')
        .next()
        .unwrap_or(basename)
        .to_ascii_lowercase();
    match bare.as_str() {
        "bash" => Some(Shell::Bash),
        "zsh" => Some(Shell::Zsh),
        "fish" => Some(Shell::Fish),
        "pwsh" | "powershell" => Some(Shell::Powershell),
        "nu" | "nushell" => Some(Shell::Nushell),
        _ => None,
    }
}

/// Default rc file path for the given shell. Callers should still check existence and
/// decide whether to create the file (creating a missing `~/.zshrc` is fine, creating
/// `$PROFILE` on a Windows box where it's missing needs `--yes`).
pub fn default_rc_path(shell: Shell) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(match shell {
        Shell::Bash => home.join(".bashrc"),
        Shell::Zsh => home.join(".zshrc"),
        Shell::Fish => home.join(".config").join("fish").join("config.fish"),
        // PowerShell's real answer is `$PROFILE` which we can't resolve without invoking
        // pwsh. This path is the cross-version CurrentUserAllHosts default and is what
        // most users edit; callers should treat missing parent dirs as "needs --yes".
        Shell::Powershell => home
            .join("Documents")
            .join("PowerShell")
            .join("Microsoft.PowerShell_profile.ps1"),
        // Nushell's config can live elsewhere (`~/Library/Application Support/nushell/`
        // on macOS, `%APPDATA%\nushell\` on Windows), but `~/.config/nushell/config.nu`
        // is the documented default on Linux and works on macOS too. Users on other
        // layouts pass `--rc-path` to `splash install`.
        Shell::Nushell => home.join(".config").join("nushell").join("config.nu"),
    })
}

/// Source line appended inside the marker block; delegates to `splash init <shell>`
/// so any snippet update lands automatically on next shell start. Nushell is the odd
/// one out: its `source` command requires a parse-time constant path, so we can't use
/// the same eval-on-startup indirection. Instead `source_line` returns the snippet
/// itself for Nushell, which `format_block` embeds verbatim between markers — re-running
/// `splash install` rewrites the block in place when the snippet changes.
pub fn source_line(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => "eval \"$(splash init bash)\"",
        Shell::Zsh => "eval \"$(splash init zsh)\"",
        Shell::Fish => "splash init fish | source",
        Shell::Powershell => "Invoke-Expression (& splash init powershell | Out-String)",
        Shell::Nushell => NUSHELL,
    }
}

