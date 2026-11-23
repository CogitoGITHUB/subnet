//! Curated dashboard templates shipped with splashboard. Each entry is a ready-to-run
//! `DashboardConfig` — users pick one via `splash install --template <name>` (#58)
//! and drop it into their `$HOME/.splash/<home|project>.dashboard.toml` or a repo's
//! `./.splash/dashboard.toml`. Embedded via `include_str!` so no filesystem lookup
//! is needed at install time.

/// Where a template is intended to apply. Drives the install-time picker (#58): users
/// pair one `Home` template with one `Project` template so both contexts look deliberate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateContext {
    /// Home ambient splash — rendered when CWD isn't a git repo root.
    Home,
    /// Per-project splash — rendered when CWD is a git repo root and no per-dir override.
    Project,
}

#[derive(Debug, Clone, Copy)]
pub struct Template {
    pub name: &'static str,
    pub context: TemplateContext,
    /// One-line description used by the install-time picker and docs gallery.
    pub description: &'static str,
    /// Raw TOML body. Valid `DashboardConfig` — parse via `DashboardConfig::parse`.
    pub body: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        name: "home_splash",
        context: TemplateContext::Home,
        description: "Terminal name as a giant figlet hero with a postfx reveal, plus ambient weather / moon / year-progress.",
        body: include_str!("home_splash.toml"),
    },
    Template {
        name: "home_daily",
        context: TemplateContext::Home,
        description: "Large time-of-day hero; weather / sunrise / moon on the left and year/month/week/day gauges on the right.",
        body: include_str!("home_daily.toml"),
    },
    Template {
        name: "home_github",
        context: TemplateContext::Home,
        description: "Avatar + handle + contribution heatmap, plus open PRs / review queue and recent activity timeline.",
        body: include_str!("home_github.toml"),
    },
    Template {
        name: "home_minimal",
        context: TemplateContext::Home,
        description: "Quiet three-line preset: a time-of-day greeting, one-line date, and the daily quote — no figlet, lots of whitespace.",
        body: include_str!("home_minimal.toml"),
    },
    Template {
        name: "home_feed",
        context: TemplateContext::Home,
        description: "Morning briefing: time-of-day figlet, Hacker News top stories, on-this-day + sky panel, and today's Wikipedia featured article as the read-more lift.",
        body: include_str!("home_feed.toml"),
    },
    Template {
        name: "project_splash",
        context: TemplateContext::Project,
        description: "Repo name as a giant figlet hero with a postfx reveal, slug / description / license underneath — nothing else. For repo-shipped configs.",
        body: include_str!("project_splash.toml"),
    },
    Template {
        name: "project_github",
        context: TemplateContext::Project,
        description: "Dynamic repo hero + subtitle; branch state, commit / CI sparklines, open PRs, and recent releases.",
        body: include_str!("project_github.toml"),
    },
    Template {
        name: "project_minimal",
        context: TemplateContext::Project,
        description: "Quiet preset: repo name, latest tag, and the last three commits. Fully offline (no GitHub calls) — acknowledges the repo without taking over the screen.",
        body: include_str!("project_minimal.toml"),
    },
    Template {
        name: "project_codebase",
        context: TemplateContext::Project,
        description: "Repo analytics dashboard: project-name figlet at the centre flanked by files-by-dir and largest-files rankings, with LOC / age / comment-density tiles above and churn / TODO list / TODO hotspots below. Local-only — no network or auth.",
        body: include_str!("project_codebase.toml"),
    },
];

/// Lookup by name. Returns `None` for unknown templates so callers can surface a
/// helpful "did you mean …" instead of panicking.
pub fn find(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.name == name)
}

/// Templates matching the given context, in declaration order — stable ordering keeps
/// the install-time picker deterministic.
pub fn for_context(context: TemplateContext) -> impl Iterator<Item = &'static Template> {
    TEMPLATES.iter().filter(move |t| t.context == context)
}

