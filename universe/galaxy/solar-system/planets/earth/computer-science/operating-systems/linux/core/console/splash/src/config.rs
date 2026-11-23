#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::layout::{BgLevel, BorderStyle, Child, Flex, Layout, TitleAlign};
use crate::render::RenderSpec;

const DEFAULT_SETTINGS: &str = include_str!("default_settings.toml");
const DEFAULT_HOME_DASHBOARD: &str = include_str!("default_home_dashboard.toml");
const DEFAULT_PROJECT_DASHBOARD: &str = include_str!("default_project_dashboard.toml");

/// Combined view the runtime consumes: user preferences composed with the active dashboard.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub general: General,
    pub widgets: Vec<WidgetConfig>,
    pub rows: Vec<RowConfig>,
}

/// User preferences: `[general]`. Loaded from `$HOME/.splash/settings.toml`.
/// Shared across every dashboard the same user sees; per-dir overrides are 0.x out of scope.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsConfig {
    #[serde(default)]
    pub general: General,
}

/// Widgets + rows. Loaded from one of:
///
/// - a per-dir `./.splash/dashboard.toml` or `./.splash.toml`
/// - `$HOME/.splash/project.dashboard.toml` (CWD == git repo root, no per-dir override)
/// - `$HOME/.splash/home.dashboard.toml` (everything else)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DashboardConfig {
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetConfig>,
    #[serde(default, rename = "row")]
    pub rows: Vec<RowConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct General {
    #[serde(default)]
    pub wait_for_fresh: bool,
    /// Inline viewport height in rows. `None` uses the built-in default. Configs that ship more
    /// widgets than fit in the default bump this to make room.
    #[serde(default)]
    pub height: Option<u16>,
    /// Space reserved around the root layout. `padding = 1` pads one cell on all four sides;
    /// `padding = { x = 2, y = 1 }` splits horizontal / vertical. The viewport bg (if any)
    /// still paints across the whole splash area so the padded band shows the theme colour,
    /// not the terminal background.
    #[serde(default)]
    pub padding: Option<PaddingSpec>,
    /// IANA timezone name (`"Asia/Tokyo"`, `"UTC"`, …) applied to every fetcher and renderer
    /// that emits a human-readable timestamp. Unset (or unrecognised) falls back to the host's
    /// `Local` zone — matching pre-config behaviour. Per-widget options can still override
    /// (for example each `clock_*` widget already carries its own `timezone` knob).
    #[serde(default)]
    pub timezone: Option<String>,
    /// Locale code (`"en_US"`, `"ja_JP"`, …) for chrono's localized month / weekday names.
    /// Requires the `unstable-locales` chrono feature. Unset falls back to `Locale::POSIX`,
    /// which matches chrono's hard-coded English output and keeps unmigrated configs
    /// rendering identical strings.
    #[serde(default)]
    pub locale: Option<String>,
    /// Render the home dashboard automatically on shell startup. Default `true`.
    /// Set to `false` to keep `splash` quiet when CWD doesn't resolve to a project,
    /// while still letting the cd-hook paint the project splash on entry.
    #[serde(default = "default_true")]
    pub auto_home: bool,
    /// Run the cd-hook (`splash --on-cd`) when entering a project directory. Default
    /// `true`. Set to `false` to disable per-`cd` repaints without unwiring the shell rc.
    #[serde(default = "default_true")]
    pub auto_on_cd: bool,
    /// When the configured dashboard is taller than the terminal viewport, push the
    /// overflowing top rows into shell scrollback (so the user can scroll up to see them)
    /// instead of just clipping with a `… +N rows` hint. Default `false`: the flush path
    /// drags every renderer through ratatui's `insert_before` write loop, which doesn't
    /// honour `Cell::skip` and visibly overdraws stateful image protocols (kitty / iTerm2 /
    /// sixel) in the scrollback portion. Opt in only when the dashboard is text-only and
    /// you specifically want the overflow in scrollback.
    #[serde(default)]
    pub flush_to_scrollback: bool,
}

fn default_true() -> bool {
    true
}

impl Default for General {
    fn default() -> Self {
        Self {
            wait_for_fresh: false,
            height: None,
            padding: None,
            timezone: None,
            locale: None,
            auto_home: true,
            auto_on_cd: true,
            flush_to_scrollback: false,
        }
    }
}

/// Uniform or split padding. Short form `padding = 1` applies the same value to all sides;
/// long form `{ x, y }` sets horizontal (left + right) and vertical (top + bottom) separately.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum PaddingSpec {
    Uniform(u16),
    Axes {
        #[serde(default)]
        x: u16,
        #[serde(default)]
        y: u16,
    },
}

impl PaddingSpec {
    /// (`x`, `y`) where `x` is left+right-per-side and `y` is top+bottom-per-side.
    pub fn xy(&self) -> (u16, u16) {
        match self {
            Self::Uniform(n) => (*n, *n),
            Self::Axes { x, y } => (*x, *y),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WidgetConfig {
    pub id: String,
    pub fetcher: String,
    /// Renderer selection. `render = "text_plain"` (short form) or
    /// `render = { type = "text_ascii", pixel_size = "quadrant" }` (full form with options).
    /// Absent = pick the default renderer for the fetcher's shape.
    #[serde(default)]
    pub render: Option<RenderSpec>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub refresh_interval: Option<u64>,
    /// read_store: file format — "json", "toml", or "text". Other fetchers ignore it.
    #[serde(default)]
    pub file_format: Option<String>,
    /// Fetcher-specific options as a TOML sub-table (`[widget.options]`). Each fetcher
    /// deserializes the keys it cares about; unknown keys are ignored so upgrading the
    /// fetcher never invalidates old configs.
    #[serde(default)]
    pub options: Option<toml::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RowConfig {
    #[serde(default)]
    pub height: Option<SizeSpec>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_align: Option<TitleAlignSpec>,
    #[serde(default)]
    pub border: Option<BorderSpec>,
    /// How children are placed along this row's horizontal axis when they don't fill the row.
    /// `center` is the obvious use case: a narrower widget parked in the middle of its row.
    #[serde(default)]
    pub flex: Option<FlexSpec>,
    /// Which semantic background paints this row. `default` (or omitted) inherits the
    /// viewport bg; `subtle` paints `theme.bg_subtle` — use it to lift header/footer bands
    /// visually off the main content.
    #[serde(default)]
    pub bg: Option<BgSpec>,
    /// Horizontal gap inserted between sibling children, in cells. Sugar for the common
    /// 2/3-column case — `gap = 2` auto-splices empty spacers between children so configs
    /// don't have to declare a `basic_static` "gap" widget just to carve whitespace.
    #[serde(default)]
    pub gap: Option<u16>,
    #[serde(default, rename = "child")]
    pub children: Vec<ChildConfig>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BgSpec {
    /// Inherit whatever the viewport painted (usually `theme.bg`).
    Default,
    /// Paint `theme.bg_subtle` behind this slot — for header/footer/callout bands.
    Subtle,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FlexSpec {
    Legacy,
    Start,
    Center,
    End,
    SpaceBetween,
    SpaceAround,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChildConfig {
    /// Referenced widget id. Absent means this child is a pure layout spacer — the slot
    /// reserves its `width` but paints nothing. Preferred over the historical
    /// `basic_static`-with-empty-format hack for gap columns. Mutually exclusive with `rows`.
    #[serde(default)]
    pub widget: Option<String>,
    #[serde(default)]
    pub width: Option<SizeSpec>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_align: Option<TitleAlignSpec>,
    #[serde(default)]
    pub border: Option<BorderSpec>,
    /// Per-child background; see [`RowConfig::bg`].
    #[serde(default)]
    pub bg: Option<BgSpec>,
    /// Nested rows stacked vertically inside this child cell. Lets a single cell host a
    /// sub-layout (e.g. uptime above load-avg in the right gutter of a thermometer row).
    /// Mutually exclusive with `widget`; when both are set, `rows` wins.
    #[serde(default, rename = "row")]
    pub rows: Vec<RowConfig>,
}

/// Title alignment on a `border = "top"` (or any bordered) panel. `left` matches ratatui's
/// default; `center` gives `──  title  ──` style section dividers.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TitleAlignSpec {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeSpec {
    Fill(u16),
    Length(u16),
    Min(u16),
    Max(u16),
    Percentage(u16),
    /// `height = "auto"` — row / child sizes itself to the rendered content.
    /// Resolves at draw time via the renderer's `natural_height`, so a figlet
    /// hero that word-wraps onto a second block gets the row height it needs
    /// while a single-word hero fits in its natural 7-row box.
    Auto,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BorderSpec {
    /// No chrome at all — widget fills its full cell, no title drawn. Useful for hero
    /// elements like a big clock that should own its slot without framing.
    None,
    Plain,
    Rounded,
    Thick,
    Double,
    /// Single top edge only, plain style — `border = "top"` + `title = "..."` paints a
    /// section divider above the row. No side or bottom chrome is drawn, so the row keeps
    /// its full inner width for the body beneath the rule.
    Top,
}

impl Config {
    pub fn from_parts(settings: SettingsConfig, dashboard: DashboardConfig) -> Self {
        Self {
            general: settings.general,
            widgets: dashboard.widgets,
            rows: dashboard.rows,
        }
    }

    pub fn to_layout(&self) -> Layout {
        Layout::rows(self.rows.iter().map(to_row_child).collect())
    }

    /// Sum of row heights — what the inline viewport needs to be to fit every row without
    /// clipping the bottom. Non-Length sizes fall back to reasonable approximations
    /// (Min/Max use their value, Fill contributes a small default, Percentage is ignored
    /// since it can't resolve without a known viewport).
    pub fn computed_height(&self) -> u16 {
        let rows: u16 = self
            .rows
            .iter()
            .map(|r| row_height_estimate(r.height))
            .sum();
        let pad_y = self
            .general
            .padding
            .map(|p| p.xy().1)
            .unwrap_or(0)
            .saturating_mul(2);
        rows.saturating_add(pad_y)
    }
}

impl SettingsConfig {
    pub fn default_baked() -> Self {
        toml::from_str(DEFAULT_SETTINGS).expect("built-in default settings must parse")
    }

    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }

    /// Loads the user's settings file, or the baked default if it's missing. Errors bubble up
    /// for parse failures so users see what's broken instead of silently getting defaults.
    pub fn load_or_default(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::parse(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default_baked()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

impl DashboardConfig {
    pub fn default_home() -> Self {
        toml::from_str(DEFAULT_HOME_DASHBOARD).expect("built-in home dashboard must parse")
    }

    pub fn default_project() -> Self {
        toml::from_str(DEFAULT_PROJECT_DASHBOARD).expect("built-in project dashboard must parse")
    }

    pub fn parse(toml_str: &str) -> Result<Self, String> {
        toml::from_str(toml_str).map_err(|e| e.to_string())
    }
}

/// Where the active dashboard came from. Drives trust gating (local files need explicit trust;
/// the HOME-backed defaults are implicitly trusted) and which baked fallback applies when the
/// file is missing.
#[derive(Debug, Clone)]
pub enum DashboardSource {
    /// Per-dir dashboard discovered in the CWD (`./.splash/dashboard.toml` or
    /// `./.splash.toml`). Trust-gated.
    Local(PathBuf),
    /// `$HOME/.splash/project.dashboard.toml` — used when CWD is a git repo root and no
    /// per-dir dashboard overrides. Baked-in default applies when the file is absent.
    Project,
    /// `$HOME/.splash/home.dashboard.toml` — the ambient fallback. Baked-in default
    /// applies when the file is absent.
    Home,
}

pub const LOCAL_DASHBOARD_CANDIDATES: &[&str] =
    &[".splash/dashboard.toml", ".splash.toml"];

/// Dashboard resolution for a shell-startup render. Returns `Home` as a final fallback so the
/// splash always has something to show.
pub fn resolve_dashboard_source() -> DashboardSource {
    let cwd = std::env::current_dir().ok();
    match cwd.as_deref() {
        Some(c) => resolve_from(c, /* on_cd = */ false).unwrap_or(DashboardSource::Home),
        None => DashboardSource::Home,
    }
}

/// Dashboard resolution for an `--on-cd` render. Returns `None` in the fallback case so the
/// shell hook exits silently instead of repainting the home splash on every `cd`.
pub fn resolve_on_cd_dashboard_source() -> Option<DashboardSource> {
    let cwd = std::env::current_dir().ok()?;
    resolve_from(&cwd, /* on_cd = */ true)
}

/// Nearest project-local dashboard file, for the trust subcommands. Walks up ancestors so
/// running `splash trust` from a subdirectory picks the same dashboard the startup
/// resolver would use at the repo root.
pub fn resolve_local_dashboard_path() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    find_local_walking_up(&cwd)
}

fn resolve_from(cwd: &Path, on_cd: bool) -> Option<DashboardSource> {
    if is_home_dir(cwd) {
        // Dotfiles-in-git case: CWD == $HOME still means "home context", not "project".
        return Some(DashboardSource::Home);
    }
    if let Some(path) = find_local_at(cwd) {
        return Some(DashboardSource::Local(path));
    }
    if is_git_repo_root(cwd) {
        return Some(DashboardSource::Project);
    }
    if on_cd {
        None
    } else {
        Some(DashboardSource::Home)
    }
}

fn is_home_dir(path: &Path) -> bool {
    // Respect `HOME` first so the test suite can redirect the home dir on any platform.
    // On Windows, `dirs::home_dir()` consults `SHGetKnownFolderPath(FOLDERID_Profile)` and
    // ignores env vars; checking `HOME` keeps behavior aligned with shells that set it
    // (git-bash, MSYS, WSL, configured PowerShell profiles).
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir);
    let Some(home) = home else {
        return false;
    };
    canonicalize_or(path) == canonicalize_or(&home)
}

fn canonicalize_or(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn is_git_repo_root(cwd: &Path) -> bool {
    // "CWD == repo root" using direct `.git` presence. `gix::discover` walks upward, so using
    // it here would misclassify subdirectories as project roots and re-render the project
    // dashboard on every sub-cd. Checking for `.git` right here keeps the rule crisp.
    let dot_git = cwd.join(".git");
    dot_git.is_dir() || dot_git.is_file()
}

fn find_local_walking_up(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(dir) = current {
        if let Some(path) = find_local_at(dir) {
            return Some(path);
        }
        current = dir.parent();
    }
    None
}

fn find_local_at(dir: &Path) -> Option<PathBuf> {
    LOCAL_DASHBOARD_CANDIDATES
        .iter()
        .map(|name| dir.join(name))
        .find(|p| p.is_file())
}

fn row_height_estimate(size: Option<SizeSpec>) -> u16 {
    match size {
        Some(SizeSpec::Length(n)) | Some(SizeSpec::Min(n)) | Some(SizeSpec::Max(n)) => n,
        Some(SizeSpec::Fill(_)) => 3,
        Some(SizeSpec::Percentage(_)) => 0,
        // Content-sized rows don't know their height until render, so use the same
        // Fill default estimate used by other flexible sizes for viewport sizing.
        Some(SizeSpec::Auto) => 3,
        None => 3,
    }
}

fn to_row_child(row: &RowConfig) -> Child {
    let cols = intersperse_gaps(row.children.iter().map(to_col_child).collect(), row.gap);
    let mut inner = Layout::cols(cols);
    if let Some(f) = row.flex {
        inner = inner.flexed(to_flex(f));
    }
    let decorated = apply_panel(inner, row.title.as_deref(), row.border, row.title_align);
    let decorated = apply_bg(decorated, row.bg);
    make_child(row.height, decorated)
}

fn intersperse_gaps(children: Vec<Child>, gap: Option<u16>) -> Vec<Child> {
    let Some(n) = gap.filter(|n| *n > 0) else {
        return children;
    };
    let mut out = Vec::with_capacity(children.len() * 2);
    for (i, c) in children.into_iter().enumerate() {
        if i > 0 {
            out.push(Child::length(n, Layout::Empty));
        }
        out.push(c);
    }
    out
}

fn to_flex(f: FlexSpec) -> Flex {
    match f {
        FlexSpec::Legacy => Flex::Legacy,
        FlexSpec::Start => Flex::Start,
        FlexSpec::Center => Flex::Center,
        FlexSpec::End => Flex::End,
        FlexSpec::SpaceBetween => Flex::SpaceBetween,
        FlexSpec::SpaceAround => Flex::SpaceAround,
    }
}

fn to_col_child(c: &ChildConfig) -> Child {
    let leaf = if c.rows.is_empty() {
        match c.widget.as_deref() {
            Some(id) => Layout::widget(id.to_string()),
            None => Layout::Empty,
        }
    } else {
        Layout::rows(c.rows.iter().map(to_row_child).collect())
    };
    let decorated = apply_panel(leaf, c.title.as_deref(), c.border, c.title_align);
    let decorated = apply_bg(decorated, c.bg);
    make_child(c.width, decorated)
}

fn apply_bg(layout: Layout, bg: Option<BgSpec>) -> Layout {
    match bg {
        None | Some(BgSpec::Default) => layout,
        Some(BgSpec::Subtle) => layout.with_bg(BgLevel::Subtle),
    }
}

fn make_child(size: Option<SizeSpec>, layout: Layout) -> Child {
    match size {
        Some(SizeSpec::Fill(w)) => Child::fill(w, layout),
        Some(SizeSpec::Length(n)) => Child::length(n, layout),
        Some(SizeSpec::Min(n)) => Child::min(n, layout),
        Some(SizeSpec::Max(n)) => Child::max(n, layout),
        Some(SizeSpec::Percentage(p)) => Child::percentage(p, layout),
        Some(SizeSpec::Auto) => Child::auto(layout),
        None => Child::fill(1, layout),
    }
}

fn apply_panel(
    layout: Layout,
    title: Option<&str>,
    border: Option<BorderSpec>,
    title_align: Option<TitleAlignSpec>,
) -> Layout {
    // Chrome is opt-in: no border, no panel — even if a title was set. A bare title has no
    // border to render on, so dropping it cleanly is better than pretending. Users who want
    // the framed-with-label look set `border = "rounded"` (or plain/thick/double) explicitly
    // on the widgets that deserve it (charts, tables), keeping hero widgets (clock, greeting)
    // chromeless by default.
    let Some(style) = border.and_then(to_border_style) else {
        return layout;
    };
    let l = match title {
        Some(t) => layout.titled(t).title_aligned(to_title_align(title_align)),
        None => layout,
    };
    l.bordered(style)
}

fn to_title_align(spec: Option<TitleAlignSpec>) -> TitleAlign {
    match spec {
        Some(TitleAlignSpec::Center) => TitleAlign::Center,
        Some(TitleAlignSpec::Right) => TitleAlign::Right,
        Some(TitleAlignSpec::Left) | None => TitleAlign::Left,
    }
}

fn to_border_style(b: BorderSpec) -> Option<BorderStyle> {
    match b {
        BorderSpec::None => None,
        BorderSpec::Plain => Some(BorderStyle::Plain),
        BorderSpec::Rounded => Some(BorderStyle::Rounded),
        BorderSpec::Thick => Some(BorderStyle::Thick),
        BorderSpec::Double => Some(BorderStyle::Double),
        BorderSpec::Top => Some(BorderStyle::Top),
    }
}

