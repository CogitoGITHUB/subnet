//! Fullscreen gallery picker used when the user runs `splash install` in a TTY
//! without supplying the matching flags. Three screens, each optional depending on what
//! the caller still needs: home template, project template, and a toggle
//! screen for wait. Screens share a single alternate-screen session so the user
//! sees a continuous flow rather than multiple open / close flickers.

use std::io::{self, Stdout, stdout};
use std::path::PathBuf;
use std::time::Duration;

use ratatui::Frame;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Alignment, Constraint, Direction, Layout as UiLayout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::templates::{Template, TemplateContext, for_context};

use super::SettingsChoices;
use super::preview::TemplatePreview;

/// What the caller still needs the picker to fill in. Any combination may be off — if
/// the user pre-supplied templates via flags but skipped `--wait`, only
/// the toggle screen runs.
pub(crate) struct Needs {
    pub(crate) templates: bool,
    pub(crate) settings: bool,
}

/// Fully-resolved picker output. Each field is `None` when the corresponding screen was
/// skipped (i.e. the caller already has that value).
pub(crate) struct Selection {
    pub(crate) templates: Option<TemplatePair>,
    pub(crate) settings: Option<SettingsChoices>,
}

pub(crate) struct TemplatePair {
    pub(crate) home: &'static Template,
    pub(crate) project: &'static Template,
}

/// Runs whatever subset of screens `needs` requests, returning `None` if the user
/// cancels at any point so the caller can bail without touching any files.
pub(crate) fn run(needs: Needs) -> io::Result<Option<Selection>> {
    let mut terminal = enter_terminal()?;
    let result = run_screens(&mut terminal, needs);
    exit_terminal(terminal)?;
    result
}

fn run_screens(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    needs: Needs,
) -> io::Result<Option<Selection>> {
    let templates = if needs.templates {
        let Some(pair) = run_template_picker_pair(terminal)? else {
            return Ok(None);
        };
        Some(pair)
    } else {
        None
    };

    let settings = if needs.settings {
        let Some(choices) = run_settings_pickers(terminal)? else {
            return Ok(None);
        };
        Some(choices)
    } else {
        None
    };

    Ok(Some(Selection {
        templates,
        settings,
    }))
}

fn enter_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    Terminal::new(backend)
}

fn exit_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

// ── Confirmation ──────────────────────────────────────────────────────────────────

/// Snapshot of the resolved install plan shown on the final confirmation page. Built by
/// `install::run` after merging flag defaults with picker output so every line reflects
/// what would actually land on disk, not the intermediate picker state.
pub(crate) struct Summary {
    pub(crate) shell: String,
    pub(crate) home_template: String,
    pub(crate) project_template: String,
    pub(crate) wait: bool,
    pub(crate) files: Vec<PathBuf>,
    pub(crate) rc_path: Option<PathBuf>,
}

/// Opens a new alt-screen session and shows the final confirmation page. Returns `true`
/// when the user confirms (Enter), `false` on cancel (Esc / q / Ctrl-C). The caller
/// should only show this when pickers actually ran — scripted invocations skip it.
pub(crate) fn confirm(summary: Summary) -> io::Result<bool> {
    let mut terminal = enter_terminal()?;
    let result = confirm_loop(&mut terminal, &summary);
    exit_terminal(terminal)?;
    result
}

fn confirm_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    summary: &Summary,
) -> io::Result<bool> {
    loop {
        terminal.draw(|frame| draw_confirmation(frame, summary))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) | (KeyCode::Char('y'), _) | (KeyCode::Char('Y'), _) => {
                return Ok(true);
            }
            (KeyCode::Esc, _)
            | (KeyCode::Char('q'), _)
            | (KeyCode::Char('n'), _)
            | (KeyCode::Char('N'), _)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(false),
            _ => {}
        }
    }
}

fn draw_confirmation(frame: &mut Frame, summary: &Summary) {
    let meta = SlotMeta {
        title: "Review and confirm",
        subtitle: "Existing files that differ get moved to a `.bak` sidecar before the new content lands.",
    };
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(8),    // body
            Constraint::Length(1), // footer
        ])
        .split(area);
    draw_header(frame, layout[0], &meta, 1, 1);

    let block = Block::default().borders(Borders::ALL).title(" summary ");
    let inner = block.inner(layout[1]);
    frame.render_widget(block, layout[1]);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(summary_row("Shell", &summary.shell));
    lines.push(summary_row("Home dashboard", &summary.home_template));
    lines.push(summary_row("Project dashboard", &summary.project_template));
    lines.push(summary_row(
        "Wait for fresh data",
        if summary.wait { "on" } else { "off" },
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Files to write:",
        Style::default().fg(crate::theme::Theme::from_style().text),
    )));
    for path in &summary.files {
        lines.push(Line::from(Span::styled(
            format!("  · {}", path.display()),
            Style::default().fg(crate::theme::Theme::from_style().text_dim),
        )));
    }
    if let Some(rc) = &summary.rc_path {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Shell rc block:",
            Style::default().fg(crate::theme::Theme::from_style().text),
        )));
        lines.push(Line::from(Span::styled(
            format!("  · {}", rc.display()),
            Style::default().fg(crate::theme::Theme::from_style().text_dim),
        )));
    }

    frame.render_widget(Paragraph::new(lines), inner);
    draw_footer(frame, layout[2], "Enter / y: write · Esc / q / n: cancel");
}

fn summary_row(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<22}"), Style::default().fg(crate::theme::Theme::from_style().text)),
        Span::styled(
            value.to_string(),
            Style::default()
                .fg(crate::theme::Theme::from_style().text)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

// ── Template pickers ──────────────────────────────────────────────────────────────

fn run_template_picker_pair(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<Option<TemplatePair>> {
    let home_templates: Vec<&'static Template> = for_context(TemplateContext::Home).collect();
    let project_templates: Vec<&'static Template> = for_context(TemplateContext::Project).collect();

    if home_templates.is_empty() || project_templates.is_empty() {
        return Ok(None);
    }

    let Some(home) = run_template_picker(terminal, &home_templates, SlotMeta::home())? else {
        return Ok(None);
    };
    let Some(project) = run_template_picker(terminal, &project_templates, SlotMeta::project())?
    else {
        return Ok(None);
    };
    Ok(Some(TemplatePair { home, project }))
}

struct SlotMeta {
    title: &'static str,
    subtitle: &'static str,
}

impl SlotMeta {
    fn home() -> Self {
        Self {
            title: "Pick your home dashboard",
            subtitle: "Shown on new shells and when you cd outside any git project.",
        }
    }

    fn project() -> Self {
        Self {
            title: "Pick your default project dashboard",
            subtitle: "Shown automatically when you cd into the top of any git repo that doesn't ship its own .splash/.",
        }
    }
}

fn run_template_picker(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    templates: &[&'static Template],
    meta: SlotMeta,
) -> io::Result<Option<&'static Template>> {
    let previews: Vec<Option<TemplatePreview>> = templates
        .iter()
        .map(|t| TemplatePreview::from_body(t.body))
        .collect();

    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| {
            draw_template_frame(
                frame,
                &meta,
                templates,
                selected,
                previews[selected].as_ref(),
            );
        })?;

        match wait_for_action(templates.len(), selected)? {
            Action::Cancel => return Ok(None),
            Action::Confirm => return Ok(Some(templates[selected])),
            Action::Move(to) => selected = to,
            Action::Toggle | Action::Tick => {}
        }
    }
}

fn draw_template_frame(
    frame: &mut Frame,
    meta: &SlotMeta,
    templates: &[&'static Template],
    selected: usize,
    preview: Option<&TemplatePreview>,
) {
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),                          // header
            Constraint::Min(8),                             // preview
            Constraint::Length(templates.len() as u16 + 2), // list
            Constraint::Length(1),                          // footer
        ])
        .split(area);

    draw_header(frame, layout[0], meta, selected + 1, templates.len());
    draw_preview(frame, layout[1], templates[selected].name, preview);
    draw_template_list(frame, layout[2], templates, selected);
    draw_footer(frame, layout[3], "↑/↓ navigate · Enter select · Esc cancel");
}

fn draw_template_list(
    frame: &mut Frame,
    area: Rect,
    templates: &[&'static Template],
    selected: usize,
) {
    let block = Block::default().borders(Borders::ALL).title(" templates ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines: Vec<Line> = templates
        .iter()
        .enumerate()
        .map(|(i, t)| render_name_desc_row(t.name, t.description, i == selected))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Settings pickers ──────────────────────────────────────────────────────────────────

fn run_settings_pickers(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<Option<SettingsChoices>> {
    let Some(wait) = run_toggle_picker(terminal)? else {
        return Ok(None);
    };
    Ok(Some(SettingsChoices { wait }))
}

// ── Toggle picker ─────────────────────────────────────────────────────────────────

const TOGGLE_WAIT: usize = 0;

fn run_toggle_picker(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> io::Result<Option<bool>> {
    // Default matches `SettingsChoices::DEFAULT`.
    let mut values = [false];
    let mut selected = 0usize;
    loop {
        terminal.draw(|frame| {
            draw_toggle_frame(frame, selected, values);
        })?;
        match wait_for_action(values.len(), selected)? {
            Action::Cancel => return Ok(None),
            Action::Confirm => return Ok(Some(values[TOGGLE_WAIT])),
            Action::Move(to) => selected = to,
            Action::Toggle => values[selected] = !values[selected],
            Action::Tick => {}
        }
    }
}

fn draw_toggle_frame(frame: &mut Frame, selected: usize, values: [bool; 1]) {
    let meta = SlotMeta {
        title: "Pick your defaults",
        subtitle: "Applied to every splash; edit settings.toml later to change.",
    };
    let area = frame.area();
    let layout = UiLayout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(4),    // list
            Constraint::Length(1), // footer
        ])
        .split(area);
    draw_header(frame, layout[0], &meta, selected + 1, values.len());

    let block = Block::default().borders(Borders::ALL).title(" options ");
    let inner = block.inner(layout[1]);
    frame.render_widget(block, layout[1]);

    let lines = vec![render_toggle_row(
        "wait for fresh data",
        "on = block the first render until every widget has fetched at least once",
        values[TOGGLE_WAIT],
        selected == TOGGLE_WAIT,
    )];
    frame.render_widget(Paragraph::new(lines), inner);

    draw_footer(
        frame,
        layout[2],
        "space toggle · Enter continue · Esc cancel",
    );
}

fn render_toggle_row(label: &str, desc: &str, value: bool, active: bool) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let checkbox = if value { "[x] " } else { "[ ] " };
    let name_style = if active {
        Style::default()
            .fg(crate::theme::Theme::from_style().text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(crate::theme::Theme::from_style().text)
    };
    let desc_style = Style::default().fg(crate::theme::Theme::from_style().text_dim);
    Line::from(vec![
        Span::styled(format!("{marker}{checkbox}{label:<26}"), name_style),
        Span::styled(format!("  {desc}"), desc_style),
    ])
}

// ── Shared helpers ────────────────────────────────────────────────────────────────

enum Action {
    Cancel,
    Confirm,
    Toggle,
    Move(usize),
    /// No input came in this tick. Loops keep re-drawing so animations (not used today,
    /// but the preview pipeline supports them) stay visible.
    Tick,
}

fn wait_for_action(total: usize, current: usize) -> io::Result<Action> {
    if !event::poll(Duration::from_millis(250))? {
        return Ok(Action::Tick);
    }
    let Event::Key(key) = event::read()? else {
        return Ok(Action::Tick);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(Action::Tick);
    }
    Ok(match (key.code, key.modifiers) {
        (KeyCode::Esc, _)
        | (KeyCode::Char('q'), _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => Action::Cancel,
        (KeyCode::Enter, _) => Action::Confirm,
        (KeyCode::Char(' '), _) => Action::Toggle,
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) if current > 0 => Action::Move(current - 1),
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) if current + 1 < total => {
            Action::Move(current + 1)
        }
        _ => Action::Tick,
    })
}

fn draw_header(frame: &mut Frame, area: Rect, meta: &SlotMeta, n: usize, total: usize) {
    let title = Line::from(vec![
        Span::styled(
            meta.title,
            Style::default()
                .fg(crate::theme::Theme::from_style().text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(format!("{n}/{total}"), Style::default().fg(crate::theme::Theme::from_style().text_dim)),
    ]);
    let subtitle = Line::from(Span::styled(
        meta.subtitle,
        Style::default().fg(crate::theme::Theme::from_style().text_dim),
    ));
    frame.render_widget(
        Paragraph::new(vec![title, subtitle]).alignment(Alignment::Left),
        area,
    );
}

fn draw_preview(frame: &mut Frame, area: Rect, label: &str, preview: Option<&TemplatePreview>) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" preview · {label} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    match preview {
        Some(p) => p.draw(frame, inner),
        None => {
            let msg = Paragraph::new("(preview unavailable)")
                .alignment(Alignment::Center)
                .style(Style::default().fg(crate::theme::Theme::from_style().text_dim));
            frame.render_widget(msg, inner);
        }
    }
}

fn render_name_desc_row(name: &str, desc: &str, active: bool) -> Line<'static> {
    let marker = if active { "▶ " } else { "  " };
    let name_style = if active {
        Style::default()
            .fg(crate::theme::Theme::from_style().text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(crate::theme::Theme::from_style().text)
    };
    let desc_style = Style::default().fg(crate::theme::Theme::from_style().text_dim);
    Line::from(vec![
        Span::styled(format!("{marker}{name:<28}"), name_style),
        Span::styled(format!("  {desc}"), desc_style),
    ])
}

fn draw_footer(frame: &mut Frame, area: Rect, hint: &str) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {hint}"),
            Style::default().fg(crate::theme::Theme::from_style().text_dim),
        ))),
        area,
    );
}

