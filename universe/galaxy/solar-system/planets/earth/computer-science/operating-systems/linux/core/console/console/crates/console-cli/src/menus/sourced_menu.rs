use reedline::{Completer, Editor, InputMode, Menu, MenuEvent, MenuSettings, Painter, Suggestion};
use std::sync::Arc;

/// Shared command line for menu source.
///
/// `Arc` is structural: reedline owns the menu and its completer in separate
/// boxes, so the recorded line must live in shared ownership (`Menu: Send`
/// rules out `Rc<RefCell>`). The lock never contends — reedline drives both
/// ends sequentially with `&mut` — so `parking_lot` keeps it infallible with
/// no poisoning branches.
#[derive(Clone, Default, Debug)]
pub struct MenuLine(Arc<parking_lot::Mutex<Option<String>>>);

impl MenuLine {
    /// Record current editor line.
    pub(crate) fn record(&self, line: &str) {
        *self.0.lock() = Some(line.to_string());
    }

    /// Last recorded line.
    pub(crate) fn read(&self) -> Option<String> {
        self.0.lock().clone()
    }
}

/// How a menu source sees the editor line: `Diff` hands only the fragment typed
/// since the menu opened (parse the recorded line), every other mode hands the
/// live buffer. Classified once via [`SourceMode::of`]; both the completer and
/// its wrapper store the result, so they cannot disagree (#19053).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceMode {
    Diff,
    Live,
}

impl SourceMode {
    pub(crate) fn of(mode: &InputMode) -> Self {
        match mode {
            InputMode::Diff => Self::Diff,
            _ => Self::Live,
        }
    }

    /// What the source parses, with a floored/clamped cursor.
    pub(crate) fn buffer(
        self,
        line: &MenuLine,
        handed: &str,
        pos: usize,
        replacing_start: usize,
    ) -> (String, usize) {
        let buffer = match self {
            Self::Diff => line
                .read()
                .unwrap_or_else(|| " ".repeat(replacing_start) + handed),
            Self::Live => handed.to_owned(),
        };
        let cursor = buffer.floor_char_boundary(pos.min(buffer.len()));
        (buffer, cursor)
    }
}

/// Menu wrapper carrying the editor line to its source, and abandoning the menu when
/// the first fresh answer after activation is empty, so a cancelled picker (fzf Esc /
/// `input list` Esc) cannot leave an empty menu that relaunches on type.
pub struct SourcedMenu<M> {
    menu: M,
    line: Option<MenuLine>,
    mode: SourceMode,
    /// Whether the menu has shown values since it was last activated. Once it has,
    /// an empty answer means the typed text matches nothing, and the menu stays open
    /// to refill as the text changes (#19096).
    answered: bool,
}

impl<M> SourcedMenu<M> {
    pub fn new(menu: M, line: MenuLine, mode: SourceMode) -> Self {
        Self {
            menu,
            line: Some(line),
            mode,
            answered: false,
        }
    }

    /// Engine-completer menus have no source line to carry, only abandon-on-empty.
    pub fn abandoning(menu: M) -> Self {
        Self {
            menu,
            line: None,
            mode: SourceMode::Live,
            answered: false,
        }
    }

    fn record(&self, editor: &Editor) {
        if let Some(line) = &self.line {
            line.record(editor.get_buffer());
        }
    }

    /// Close when the first fresh answer is empty; provisional/awaiting emptiness is
    /// still computing, and emptiness after values is a filter that matches nothing.
    fn abandon_if_empty(&mut self)
    where
        M: Menu,
    {
        if !self.menu.get_values().is_empty() {
            self.answered = true;
        } else if self.menu.is_active()
            && !self.answered
            && !self.menu.results_are_provisional()
            && !self.menu.is_awaiting_first_answer()
        {
            self.menu.menu_event(MenuEvent::Deactivate);
        }
    }
}

impl<M: Menu> Menu for SourcedMenu<M> {
    fn settings(&self) -> &MenuSettings {
        self.menu.settings()
    }

    fn name(&self) -> &str {
        self.menu.name()
    }

    fn indicator(&self) -> &str {
        self.menu.indicator()
    }

    fn is_active(&self) -> bool {
        self.menu.is_active()
    }

    fn set_active(&mut self, active: bool) {
        self.menu.set_active(active);
    }

    fn clear_input(&mut self) {
        self.menu.clear_input();
    }

    fn on_activate(&mut self) {
        self.menu.on_activate();
    }

    fn on_deactivate(&mut self) {
        self.menu.on_deactivate();
    }

    fn menu_event(&mut self, event: MenuEvent) {
        if matches!(event, MenuEvent::Activate(_)) {
            self.answered = false;
        }
        self.menu.menu_event(event);
    }

    fn can_quick_complete(&self) -> bool {
        self.menu.can_quick_complete()
    }

    fn can_partially_complete(
        &mut self,
        values_updated: bool,
        editor: &mut Editor,
        completer: &mut dyn Completer,
    ) -> bool {
        self.record(editor);
        let spliced = self
            .menu
            .can_partially_complete(values_updated, editor, completer);
        if spliced && matches!((self.mode, &self.line), (SourceMode::Diff, Some(_))) {
            // The splice ran past this wrapper, so a `Diff` source would answer
            // with pre-splice spans without a refresh (#19053); anything else
            // already saw the spliced buffer through the handed line.
            self.update_values(editor, completer);
        } else {
            if spliced {
                self.record(editor);
            }
            // Reedline probes partial completion as a menu opens, and with quick
            // completions off that probe is the first fetch: a cancelled picker
            // answers empty here. Abandon now, before the prompt paints the menu
            // indicator; a probe that fetched nothing is still awaiting its answer.
            self.abandon_if_empty();
        }
        spliced
    }

    fn update_values(&mut self, editor: &mut Editor, completer: &mut dyn Completer) {
        self.record(editor);
        self.menu.update_values(editor, completer);
        self.abandon_if_empty();
    }

    fn reset_position(&mut self) {
        self.menu.reset_position();
    }

    fn reload(&mut self, updated: bool, editor: &mut Editor, completer: &mut dyn Completer) {
        self.record(editor);
        self.menu.reload(updated, editor, completer);
        self.abandon_if_empty();
    }

    fn update_working_details(
        &mut self,
        editor: &mut Editor,
        completer: &mut dyn Completer,
        painter: &Painter,
    ) {
        self.record(editor);
        self.menu.update_working_details(editor, completer, painter);
        self.abandon_if_empty();
    }

    fn replace_in_buffer(&self, editor: &mut Editor) {
        self.menu.replace_in_buffer(editor);
    }

    fn menu_required_lines(&self, terminal_columns: u16) -> u16 {
        self.menu.menu_required_lines(terminal_columns)
    }

    fn menu_string(&self, available_lines: u16, use_ansi_coloring: bool) -> String {
        self.menu.menu_string(available_lines, use_ansi_coloring)
    }

    fn min_rows(&self) -> u16 {
        self.menu.min_rows()
    }

    fn get_values(&self) -> &[Suggestion] {
        self.menu.get_values()
    }

    fn results_are_provisional(&self) -> bool {
        self.menu.results_are_provisional()
    }

    fn is_awaiting_first_answer(&self) -> bool {
        self.menu.is_awaiting_first_answer()
    }

    fn set_cursor_pos(&mut self, pos: (u16, u16)) {
        self.menu.set_cursor_pos(pos);
    }
}

