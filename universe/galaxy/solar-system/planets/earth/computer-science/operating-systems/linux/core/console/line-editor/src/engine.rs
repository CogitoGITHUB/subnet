use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    mem,
    ops::ControlFlow,
    path::{Path, PathBuf},
};

use nu_ansi_term::Style;

use crate::{enums::ReedlineRawEvent, external_printer::ExternalOutput, CursorConfig};
#[cfg(feature = "bashisms")]
use crate::{
    history::SearchFilter,
    menu_functions::{parse_selection_char, ParseAction},
};
use {
    crate::{
        completion::{Completer, CompletionOrigin, CompletionStatus, DefaultCompleter},
        core_editor::{EditCommandStatus, Editor},
        edit_mode::{EditMode, Emacs},
        enums::{EventStatus, ReedlineEvent},
        highlighter::SimpleMatchHighlighter,
        hinter::Hinter,
        history::{
            FileBackedHistory, History, HistoryCursor, HistoryItem, HistoryItemId,
            HistoryNavigationQuery, HistorySessionId, SearchDirection, SearchQuery,
        },
        painting::{
            continue_hint_lines, escape_control, escape_control_keep_sgr, Painter,
            PainterSuspendedState, PromptLines, RenderSnapshot, W,
        },
        prompt::{PromptEditMode, PromptHistorySearchStatus},
        result::{ReedlineError, ReedlineErrorVariants},
        terminal_extensions::{
            bracketed_paste::BracketedPasteGuard,
            kitty::KittyProtocolGuard,
            semantic_prompt::{Osc133ClickEventsMarkers, SemanticPromptMarkers},
            tty_mode::TtyModeGuard,
        },
        utils::text_manipulation,
        AbbrExpandContext, AutoPairAction, AutoPairContext, AutoPairs, EditCommand,
        ExampleHighlighter, Highlighter, LineBuffer, Menu, MenuEvent, MouseButton, Prompt,
        PromptHistorySearch, ReedlineMenu, Signal, StatusLine, UndoBehavior, ValidationResult,
        Validator,
    },
    crossterm::{
        cursor::SetCursorStyle,
        event,
        event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        terminal, QueueableCommand,
    },
    std::{
        fs::File,
        io,
        io::Result,
        io::Write,
        process::Command,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
        time::SystemTime,
    },
};

// The POLL_WAIT is used to specify for how long the POLL should wait for
// events, to accelerate the handling of paste or compound resize events. Having
// a POLL_WAIT of zero means that every single event is treated as soon as it
// arrives. This doesn't allow for the possibility of more than 1 event
// happening at the same time.
const POLL_WAIT: Duration = Duration::from_millis(100);
// Since a paste event is multiple `Event::Key` events happening at the same
// time, we specify how many events should be in the `crossterm_events` vector
// before it is considered a paste. 10 events is conservative enough.
const EVENTS_THRESHOLD: usize = 10;

/// Default maximum time Reedline will block on input before yielding control
/// for features that require periodic processing (e.g., external printer,
/// idle callback).
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Where history navigation leaves the cursor in the entry it shows.
#[derive(Debug, Clone, Copy)]
enum HistoryLanding {
    /// End of the first line, so the next Up keeps walking history.
    FirstLineEnd,
    /// End of the buffer, so the next Down keeps walking history.
    End,
}

/// Determines if inputs should be used to extend the regular line buffer,
/// traverse the history in the standard prompt or edit the search string in the
/// reverse search
#[derive(Debug, PartialEq, Eq)]
enum InputMode {
    /// Regular input by user typing or previous insertion.
    /// Undo tracking is active
    Regular,
    /// Full reverse search mode with different prompt,
    /// editing affects the search string,
    /// suggestions are provided to be inserted in the line buffer
    HistorySearch,
    /// Hybrid mode indicating that history is walked through in the standard prompt
    /// Either bash style up/down history or fish style prefix search,
    /// Edits directly switch to [`InputMode::Regular`]
    HistoryTraversal,
}

/// Configuration for mouse click-to-cursor support.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MouseClickMode {
    /// Disable mouse click handling.
    #[default]
    Disabled,
    /// Enable mouse click handling without emitting OSC 133 markers.
    Enabled,
    /// Enable mouse click handling and emit OSC 133 markers with `click_events=1`.
    EnabledWithOsc133,
}

impl MouseClickMode {
    fn is_enabled(self) -> bool {
        matches!(self, Self::Enabled | Self::EnabledWithOsc133)
    }
}

#[derive(Default)]
enum HistoryLastRun {
    Stored(HistoryItemId),
    /// Outlives its `read_line` for Up to recall, but only the call that
    /// submitted it (`current`) may update its context.
    Excluded {
        item: HistoryItem,
        current: bool,
        recalled: bool,
    },
    #[default]
    Empty,
}

impl HistoryLastRun {
    fn set_recalled(&mut self, recalled: bool) {
        if let Self::Excluded { recalled: slot, .. } = self {
            *slot = recalled;
        }
    }

    fn is_recalled(&self) -> bool {
        matches!(self, Self::Excluded { recalled: true, .. })
    }
}

/// Line editor engine
///
/// ## Example usage
/// ```no_run
/// use reedline::{Reedline, Signal, DefaultPrompt};
/// let mut line_editor = Reedline::create();
/// let prompt = DefaultPrompt::default();
///
/// let out = line_editor.read_line(&prompt).unwrap();
/// match out {
///    Signal::Success(content) => {
///        // process content
///    }
///    _ => {
///        eprintln!("Entry aborted!");
///
///    }
/// }
/// ```
pub struct Reedline {
    editor: Editor,

    // History
    history: Box<dyn History>,
    history_cursor: HistoryCursor,
    history_session_id: Option<HistorySessionId>,
    history_last_run: HistoryLastRun,
    history_exclusion_prefix: Option<String>,
    /// Last failed `history.save`, until [`Reedline::take_history_save_error`].
    history_save_error: Option<ReedlineError>,
    input_mode: InputMode,

    // State of the painter after a `ReedlineEvent::ExecuteHostCommand` was requested, used after
    // execution to decide if we can re-use the previous prompt or paint a new one.
    suspended_state: Option<PainterSuspendedState>,
    last_render_snapshot: Option<RenderSnapshot>,

    // Validator
    validator: Option<Box<dyn Validator>>,

    // Stdout
    painter: Painter,

    transient_prompt: Option<Box<dyn Prompt>>,

    // Edit Mode: Vi, Emacs
    edit_mode: Box<dyn EditMode>,
    /// Standbys: machines a [`ReedlineEvent::SwitchMode`] can swap into
    /// `edit_mode`, offered a target in registration order.
    standby_edit_modes: Vec<Box<dyn EditMode>>,

    // Provides the tab completions
    completer: Box<dyn Completer + Send>,
    quick_completions: bool,
    partial_completions: bool,
    persistent_menus: bool,
    // Completions owed to a menu activation the completer could not answer in time
    deferred_menu_completion: Option<DeferredMenuCompletion>,

    // Optional host hook that intercepts a lone `PasteSystem` paste
    // (see `crate::PasteInterceptor`). `PasteSystem` only exists under
    // `system_clipboard`, so the hook is gated on the same feature.
    #[cfg(feature = "system_clipboard")]
    paste_interceptor: Option<Arc<dyn crate::PasteInterceptor>>,

    // Optional host hook that classifies a rapid key-event stream as a paste
    // burst by timing, reclassifying embedded Enters to newlines and coalescing
    // chunks (see `crate::PasteBurstHook`).
    paste_burst: Option<Arc<dyn crate::PasteBurstHook>>,

    // Highlight the edit buffer
    highlighter: Box<dyn Highlighter>,

    // Style used for visual selection
    visual_selection_style: Style,
    /// A distinct style for the cell under the cursor inside a selection;
    /// `None` leaves the cell on `visual_selection_style`.
    visual_selection_cursor_style: Option<Style>,

    // Showcase hints based on various strategies (history, language-completion, spellcheck, etc)
    hinter: Option<Box<dyn Hinter>>,
    hide_hints: bool,

    // Commentary on the line, drawn below it (errors, signatures, types).
    // Hidden with the hints on submit, but kept in history traversal.
    status_line: Option<Box<dyn StatusLine>>,

    // Use ansi coloring or not
    use_ansi_coloring: bool,

    // Automatically insert and manage configured character pairs.
    auto_pairs: Option<AutoPairs>,

    // Whether to enable mouse click-to-cursor functionality
    mouse_click_mode: MouseClickMode,

    // Current working directory as defined by the application. If set, it will
    // override the actual working directory of the process.
    cwd: Option<String>,

    // Engine Menus
    menus: Vec<ReedlineMenu>,

    abbreviations: HashMap<String, String>,

    // Text editor used to open the line buffer for editing
    buffer_editor: Option<BufferEditor>,

    // Use different cursors depending on the current edit mode
    cursor_shapes: Option<CursorConfig>,

    // Manage bracketed paste mode
    bracketed_paste: BracketedPasteGuard,

    // Manage optional kitty protocol
    kitty_protocol: KittyProtocolGuard,

    // Whether lines should be accepted immediately
    immediately_accept: bool,

    // External break signal: when set to `true`, `read_line()` will return
    // `Signal::ExternalBreak` with the current buffer contents.
    break_signal: Option<Arc<AtomicBool>>,

    // External repaint signal: when triggered, the prompt is re-evaluated
    // and repainted in place while `read_line()` is running.
    repaint_signal: Option<RepaintSignal>,

    // Maximum time to block on input before yielding control for features that
    // require periodic processing (external printer, idle callback).
    // Only used when external_printer or idle_callback is configured.
    poll_interval: Duration,

    external_printer: Option<Box<dyn ExternalOutput>>,

    // Callback function that is called periodically while waiting for input.
    // Useful for processing external events (e.g., GUI updates) during idle time.
    idle_callback: Option<Box<dyn FnMut() + Send>>,
}

/// A command to spawn a buffer editor. The arguments may contain template
/// placeholders for the file path and cursor position.
pub struct EditorCommand(Command);

impl EditorCommand {
    const FILE: &str = "{file}";
    const LINE: &str = "{line}";
    const COL: &str = "{col}";

    /// Creates an editor command.
    pub fn new(command: Command) -> EditorCommand {
        EditorCommand(command)
    }
    fn is_template(&self) -> bool {
        self.0.get_args().any(EditorCommand::is_template_arg)
    }

    fn is_template_arg(arg: &OsStr) -> bool {
        let arg = arg.to_string_lossy();
        arg.contains(Self::FILE) || arg.contains(Self::LINE) || arg.contains(Self::COL)
    }

    /// Returns the editor arguments with `{file}`, `{line}`, and `{col}`
    /// placeholders replaced by the supplied values. If no argument contains
    /// the `{file}` placeholder, or exactly matches the path of the passed
    /// file, the file path is appended.
    ///
    /// Line and column numbers are one-based; zero is treated as one.
    /// Columns are counted as graphemes from the start of the line.
    ///
    /// The returned arguments exclude the executable name. Rendering leaves
    /// the stored command unchanged. Only an argument holding a placeholder
    /// is converted to Unicode, lossily; the others and an appended file path
    /// keep their native encoding.
    pub fn render(&self, file: impl AsRef<Path>, line: usize, col: usize) -> Vec<OsString> {
        let mut rendered = Vec::new();

        let file_os_str = file.as_ref().as_os_str();
        let file_str = file.as_ref().to_string_lossy();
        let mut has_file_arg = false;

        // right now, the API is 1-based, so we silently clamp to 1.
        let line = line.max(1).to_string();
        let col = col.max(1).to_string();

        for arg in self.0.get_args() {
            has_file_arg |= arg == file_os_str;
            if !Self::is_template_arg(arg) {
                rendered.push(arg.to_owned());
                continue;
            }
            let arg = arg.to_string_lossy();
            has_file_arg |= arg.contains(Self::FILE);

            let arg = arg
                .replace(Self::FILE, &file_str)
                .replace(Self::LINE, &line)
                .replace(Self::COL, &col);

            rendered.push(arg.into());
        }

        if !has_file_arg {
            rendered.push(file_os_str.to_owned());
        }

        rendered
    }
}

impl From<Command> for EditorCommand {
    fn from(command: Command) -> EditorCommand {
        EditorCommand::new(command)
    }
}

struct BufferEditor {
    command: EditorCommand,
    temp_file: PathBuf,
}

impl BufferEditor {
    fn new(mut command: EditorCommand, temp_file: PathBuf) -> BufferEditor {
        if !command
            .0
            .get_args()
            .any(|arg| EditorCommand::is_template_arg(arg) || arg == temp_file.as_os_str())
        {
            command.0.arg(&temp_file);
        }

        BufferEditor { command, temp_file }
    }

    pub(crate) fn build_command(&self, line_buffer: &LineBuffer) -> Command {
        let mut rendered = Command::new(self.command.0.get_program());

        for (key, value) in self.command.0.get_envs() {
            match value {
                Some(value) => rendered.env(key, value),
                None => rendered.env_remove(key),
            };
        }

        if let Some(dir) = self.command.0.get_current_dir() {
            rendered.current_dir(dir);
        }

        let args = self.command.render(
            &self.temp_file,
            line_buffer.line() + 1,
            line_buffer.col() + 1,
        );
        rendered.args(args);

        rendered
    }
}

/// The completions the [`Menu`](ReedlineEvent::Menu) event could not decide, because the
/// completer had not answered yet.
///
/// Activating a menu immediately inspects its values twice: quick completions accept a
/// lone suggestion, and partial completions splice in the prefix the suggestions share.
/// A completer computing in the background can answer neither yet.
///
/// The snapshot pins the editor state as of the keystroke this was armed on: a result
/// landing after the user has typed on is discarded rather than rewriting the line
/// underneath them.
struct DeferredMenuCompletion {
    menu: String,
    /// The line this was armed on, stamped as a completer stamps its own results.
    origin: CompletionOrigin,
}

impl DeferredMenuCompletion {
    fn new(menu: &ReedlineMenu, editor: &Editor) -> Self {
        Self {
            menu: menu.name().to_string(),
            origin: CompletionOrigin::new(editor.get_buffer(), editor.insertion_point()),
        }
    }

    /// Whether the line is still exactly as it was when this was armed, on the same menu.
    /// Anything else means the user moved on and the decision is void.
    fn still_applies(&self, menu: &ReedlineMenu, editor: &Editor) -> bool {
        self.menu == menu.name()
            && self
                .origin
                .matches(editor.get_buffer(), editor.insertion_point())
    }
}

/// Call [`request_repaint`](RepaintSignal::request_repaint) once
/// new prompt data is ready! The next iteration of the input loop re-evaluates
/// the [`Prompt`] and redraws it without interrupting the current line edit.
#[derive(Clone, Debug, Default)]
pub struct RepaintSignal {
    flag: Arc<AtomicBool>,
}

impl RepaintSignal {
    /// Request that the prompt is re-evaluated and repainted in place.
    pub fn request_repaint(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    /// Consume a pending repaint request.
    fn take(&self) -> bool {
        self.flag.swap(false, Ordering::Relaxed)
    }
}

impl Drop for Reedline {
    fn drop(&mut self) {
        if self.cursor_shapes.is_some() {
            let _ignore = terminal::enable_raw_mode();
            let mut stdout = std::io::stdout();
            let _ignore = stdout.queue(SetCursorStyle::DefaultUserShape);
            let _ignore = stdout.flush();
        }
        let _ignore = self.painter.show_cursor();

        // Ensures that the terminal is in a good state if we panic semigracefully
        // Calling `disable_raw_mode()` twice is fine with Linux
        let _ignore = terminal::disable_raw_mode();
    }
}

/// Mark the painter's cached prompt anchor stale around a menu running its completer,
/// which may have left the cached row pointing at content that has scrolled. See
/// [`ReedlineMenu::queries_host_completer`] for why only some completers count, and
/// #1130 for the bug.
///
/// Also skipped for a menu the same keystroke deactivated, whose queued event nothing
/// goes on to consume.
///
/// Call this from every event that reaches the completer, which is not the same set as
/// the events that change the menu's selection: `MenuNext` splices a partial completion
/// and queries, while `MenuPrevious` only moves and does not.
fn invalidate_anchor_if_host_completer_runs(menu: &ReedlineMenu, painter: &mut Painter) {
    if menu.is_active() && menu.queries_host_completer() {
        painter.invalidate_prompt_start_row();
    }
}

/// The char a key event contributes to a paste burst: a char press under the
/// modifiers that type text, AltGr's Ctrl-Alt included, since ConPTY reports
/// a pasted `@` or `{` on such layouts that way.
///
/// Release events are skipped. With the kitty keyboard enhancement every key
/// also yields a Release, which reedline's own `try_from` drops, so counting
/// both would double every char.
fn burst_char(event: &Event) -> Option<char> {
    match event {
        Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            kind,
            ..
        }) if *kind != KeyEventKind::Release && crate::edit_mode::is_text_char(*modifiers) => {
            Some(*c)
        }
        _ => None,
    }
}

/// A key press that stands for a newline in pasted text: a bare `Enter` (CR),
/// or `Ctrl-J`, which is how a raw LF arrives in raw mode.
fn is_pasted_newline(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            ..
        }) | Event::Key(KeyEvent {
            code: KeyCode::Char('j'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            ..
        })
    )
}

impl Reedline {
    /// Create a new [`Reedline`] engine with a local [`History`] that is not synchronized to a file:
    /// a [`FileBackedHistory`] without a file, kept in memory only.
    #[must_use]
    pub fn create() -> Self {
        let history = Box::<FileBackedHistory>::default();
        #[cfg(not(test))]
        let painter = Painter::new(W::terminal());
        let buffer_highlighter = Box::<ExampleHighlighter>::default();
        let visual_selection_style = Style::new().on(crate::roles::dim());
        let completer = Box::<DefaultCompleter>::default();
        let hinter = None;
        let validator = None;
        let edit_mode = Box::<Emacs>::default();
        let hist_session_id = None;

        Reedline {
            editor: Editor::default(),
            history,
            history_cursor: HistoryCursor::new(
                HistoryNavigationQuery::Normal(LineBuffer::default()),
                hist_session_id,
            ),
            history_session_id: hist_session_id,
            history_last_run: HistoryLastRun::Empty,
            history_exclusion_prefix: None,
            history_save_error: None,
            input_mode: InputMode::Regular,
            suspended_state: None,
            last_render_snapshot: None,
            painter,
            transient_prompt: None,
            edit_mode,
            standby_edit_modes: Vec::new(),
            completer,
            quick_completions: false,
            partial_completions: false,
            persistent_menus: false,
            deferred_menu_completion: None,
            #[cfg(feature = "system_clipboard")]
            paste_interceptor: None,
            paste_burst: None,
            highlighter: buffer_highlighter,
            visual_selection_style,
            visual_selection_cursor_style: None,
            hinter,
            hide_hints: false,
            status_line: None,
            validator,
            use_ansi_coloring: true,
            auto_pairs: None,
            mouse_click_mode: MouseClickMode::default(),
            cwd: None,
            menus: Vec::new(),
            abbreviations: HashMap::new(),
            buffer_editor: None,
            cursor_shapes: None,
            bracketed_paste: BracketedPasteGuard::default(),
            kitty_protocol: KittyProtocolGuard::default(),
            immediately_accept: false,
            break_signal: None,
            repaint_signal: None,
            poll_interval: DEFAULT_POLL_INTERVAL,
            external_printer: None,
            idle_callback: None,
        }
    }

    /// Get a new history session id based on the current time and the first commit datetime of reedline
    pub fn create_history_session_id() -> Option<HistorySessionId> {
        let nanos = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(n) => n.as_nanos() as i64,
            Err(_) => 0,
        };

        Some(HistorySessionId::new(nanos))
    }

    /// Toggle whether reedline enables bracketed paste to reed copied content
    ///
    /// This currently alters the behavior for multiline pastes as pasting of regular text will
    /// execute after every complete new line as determined by the [`Validator`]. With enabled
    /// bracketed paste all lines will appear in the buffer and can then be submitted with a
    /// separate enter.
    ///
    /// At this point most terminals should support it or ignore the setting of the necessary
    /// flags. For full compatibility, keep it disabled.
    pub fn use_bracketed_paste(mut self, enable: bool) -> Self {
        self.bracketed_paste.set(enable);
        self
    }

    /// Toggle whether reedline uses the kitty keyboard enhancement protocol
    ///
    /// This allows us to disambiguate more events than the traditional standard
    /// Only available with a few terminal emulators.
    /// You can check for that with [`crate::kitty_protocol_available`]
    /// `Reedline` will perform this check internally
    ///
    /// Read more: <https://sw.kovidgoyal.net/kitty/keyboard-protocol/>
    pub fn use_kitty_keyboard_enhancement(mut self, enable: bool) -> Self {
        self.kitty_protocol.set(enable);
        self
    }

    /// Return the previously generated history session id
    pub fn get_history_session_id(&self) -> Option<HistorySessionId> {
        self.history_session_id
    }

    /// Set a new history session id
    /// This should be used in situations where the user initially did not have a history_session_id
    /// and then later realized they want to have one without restarting the application.
    pub fn set_history_session_id(&mut self, session: Option<HistorySessionId>) -> Result<()> {
        self.history_session_id = session;
        Ok(())
    }

    /// A builder to include a [`Hinter`] in your instance of the Reedline engine
    /// # Example
    /// ```rust
    /// //Cargo.toml
    /// //[dependencies]
    /// //nu-ansi-term = "*"
    /// use {
    ///     nu_ansi_term::Style,
    ///     reedline::{DefaultHinter, Reedline},
    /// };
    ///
    /// let mut line_editor = Reedline::create().with_hinter(Box::new(
    ///     DefaultHinter::default()
    ///     .with_style(Style::new().italic().dimmed()),
    /// ));
    /// ```
    #[must_use]
    pub fn with_hinter(mut self, hinter: Box<dyn Hinter>) -> Self {
        self.hinter = Some(hinter);
        self
    }

    /// Remove current [`Hinter`]
    #[must_use]
    pub fn disable_hints(mut self) -> Self {
        self.hinter = None;
        self
    }

    /// A builder that draws the text of `status_line` below the buffer, above
    /// any open menu, and returns the engine with it set.
    ///
    /// Like the hint, it is hidden in history search and erased before the
    /// line is submitted. Unlike the hint, it stays while Up and Down walk
    /// through history, describing each line they recall.
    #[must_use]
    pub fn with_status_line(mut self, status_line: Box<dyn StatusLine>) -> Self {
        self.status_line = Some(status_line);
        self
    }

    /// Remove current [`StatusLine`], returning the engine without one.
    #[must_use]
    pub fn disable_status_line(mut self) -> Self {
        self.status_line = None;
        self
    }

    /// A builder to configure the tab completion
    /// # Example
    /// ```rust
    /// // Create a reedline object with tab completions support
    ///
    /// use reedline::{DefaultCompleter, Reedline};
    ///
    /// let commands = vec![
    ///   "test".into(),
    ///   "hello world".into(),
    ///   "hello world reedline".into(),
    ///   "this is the reedline crate".into(),
    /// ];
    /// let completer = Box::new(DefaultCompleter::new_with_wordlen(commands.clone(), 2));
    ///
    /// let mut line_editor = Reedline::create().with_completer(completer);
    /// ```
    #[must_use]
    pub fn with_completer(mut self, completer: Box<dyn Completer + Send>) -> Self {
        self.completer = completer;
        self
    }

    /// Turn on quick completions. These completions will auto-select if the completer
    /// ever narrows down to a single entry.
    #[must_use]
    pub fn with_quick_completions(mut self, quick_completions: bool) -> Self {
        self.quick_completions = quick_completions;
        self
    }

    /// Install a paste interceptor. When set, a lone
    /// `EditCommand::PasteSystem` (Ctrl+Shift+V by default) calls
    /// [`PasteInterceptor::on_paste`](crate::PasteInterceptor::on_paste)
    /// instead of the default clipboard-read-and-insert, and reedline inserts
    /// whatever [`PasteAction`](crate::PasteAction) the hook returns. Requires
    /// the `system_clipboard` feature, which is what defines `PasteSystem`.
    #[cfg(feature = "system_clipboard")]
    #[must_use]
    pub fn with_paste_interceptor(mut self, interceptor: Arc<dyn crate::PasteInterceptor>) -> Self {
        self.paste_interceptor = Some(interceptor);
        self
    }

    /// Install a paste-burst timing hook. When set, the read loop feeds each
    /// just-read plain char to the hook at read time, keeps draining while
    /// [`PasteBurstHook::is_burst_active`](crate::PasteBurstHook::is_burst_active)
    /// is true, and reclassifies a bare `Enter` to an inserted newline when
    /// [`PasteBurstHook::enter_is_newline`](crate::PasteBurstHook::enter_is_newline)
    /// returns true. When unset, the read loop behaves exactly as before.
    #[must_use]
    pub fn with_paste_burst(mut self, hook: Arc<dyn crate::PasteBurstHook>) -> Self {
        self.paste_burst = Some(hook);
        self
    }

    /// Control whether the cursor crosses line boundaries on left/right motions
    /// in a block caret (vi normal/visual mode). When `true` (the default), `l`
    /// at the end of a line moves to the next line's first character and `h` at
    /// column 0 to the previous line's last; when `false`, both stop at the line
    /// edge (vim's default `h`/`l`). Has no effect on emacs or vi insert mode,
    /// whose bar caret always moves freely across lines.
    ///
    /// Scope: this steers where the **caret rests** on `h`/`l`, not how far an
    /// operator reaches. Operator motions (`d`/`c`/`y`) delete the literal
    /// grapheme span regardless of this flag, so e.g. `dl` always deletes the
    /// char under the caret and never the line break.
    #[must_use]
    pub fn with_cross_line_cursor(mut self, cross_line_cursor: bool) -> Self {
        self.editor.set_cross_line_cursor(cross_line_cursor);
        self
    }

    /// Turn on partial completions. These completions will fill the buffer with the
    /// smallest common string from all the options
    #[must_use]
    pub fn with_partial_completions(mut self, partial_completions: bool) -> Self {
        self.partial_completions = partial_completions;
        self
    }

    /// Make active menus persist while the line is edited: erasing characters
    /// or emptying the line refilters the menu instead of dismissing it.
    ///
    /// When disabled (the default), an active menu is deactivated by a backspace
    /// when quick completions are on, and by any edit that leaves the line
    /// buffer empty. A persistent menu still closes on Esc, Ctrl-C, or when a
    /// value is accepted.
    #[must_use]
    pub fn with_persistent_menus(mut self, persistent_menus: bool) -> Self {
        self.persistent_menus = persistent_menus;
        self
    }

    /// Whether ANSI coloring should be used for the current terminal.
    ///
    /// ANSI coloring requires both the Reedline configuration to enable it and
    /// terminal support; `TERM=dumb` takes precedence over `with_ansi_colors(true)`.
    /// The painter holds the terminal policy, so the colors and the escapes it
    /// skips cannot disagree.
    fn effective_ansi_coloring(&self) -> bool {
        self.use_ansi_coloring && !self.painter.term_is_dumb()
    }

    /// A builder which enables or disables the use of ansi coloring in the prompt
    /// and in the command line syntax highlighting.
    #[must_use]
    pub fn with_ansi_colors(mut self, use_ansi_coloring: bool) -> Self {
        self.use_ansi_coloring = use_ansi_coloring;
        self
    }

    /// A builder that configures automatic pair insertion for the Reedline engine.
    ///
    /// By default, auto-pairing applies at every position for the configured pairs.
    /// To suppress it in certain syntactic positions (e.g. string literals, or in the
    /// middle of a word), override [`Highlighter::should_auto_pair`].
    ///
    /// For `InsertChar`, the closer is looked up before the opener. A character that
    /// is registered as both a closer of one pair and an opener of another therefore
    /// resolves based on whether that closer currently sits at the cursor, not on the
    /// order the pairs were passed to [`AutoPairs::new`].
    ///
    /// This builder does not touch the terminal's bracketed paste setting.
    /// If bracketed paste is enabled (see [`Self::use_bracketed_paste`]), a paste is
    /// delivered as [`EditCommand::InsertString`](crate::EditCommand::InsertString),
    /// which never goes through auto-pairing. If bracketed paste is disabled, pasted
    /// characters arrive the same way as typed ones and are auto-paired like typing:
    /// pasting text that has an opener without its closer (e.g. `foo(bar`) inserts a
    /// closing character that was never in the clipboard content. For this reason it
    /// is recommended to combine `with_auto_pairs` with
    /// `use_bracketed_paste(cfg!(not(target_os = "windows")))`. On Windows,
    /// stock crossterm reads console input through
    /// the Win32 console API and has no ANSI input parser, so there is no
    /// `Event::Paste` path to enable there (see crossterm-rs/crossterm#737).
    ///
    /// Auto-pairing applies to edits in the regular line buffer. Reverse-history
    /// search edits a separate search query and are not passed through the
    /// auto-pairing machinery. Neither is typing while a menu in
    /// [`InputMode::Diff`](crate::InputMode::Diff) is open, such as a
    /// [`ListMenu`](crate::ListMenu) used for history, since that text is the
    /// menu's query.
    #[must_use]
    pub fn with_auto_pairs(mut self, auto_pairs: AutoPairs) -> Self {
        self.auto_pairs = Some(auto_pairs);
        self
    }

    /// Disable automatic pair insertion.
    #[must_use]
    pub fn disable_auto_pairs(mut self) -> Self {
        self.auto_pairs = None;
        self
    }

    /// Configure mouse click-to-cursor support.
    ///
    /// Use [`MouseClickMode::Enabled`] to handle click events when your host shell
    /// emits OSC 133 markers. Use [`MouseClickMode::EnabledWithOsc133`] to have
    /// Reedline emit OSC 133 markers with `click_events=1` so supporting terminals
    /// can send click events.
    /// See: <https://sw.kovidgoyal.net/kitty/shell-integration/#notes-for-shell-developers>
    #[must_use]
    pub fn with_mouse_click(mut self, mode: MouseClickMode) -> Self {
        self.mouse_click_mode = mode;
        if matches!(mode, MouseClickMode::EnabledWithOsc133) {
            self.painter
                .set_semantic_markers(Some(Osc133ClickEventsMarkers::boxed()));
        }
        self
    }

    /// Update current working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd;
        self
    }

    /// A builder that configures the highlighter for your instance of the Reedline engine
    /// # Example
    /// ```rust
    /// // Create a reedline object with highlighter support
    ///
    /// use reedline::{ExampleHighlighter, Reedline};
    ///
    /// let commands = vec![
    ///   "test".into(),
    ///   "hello world".into(),
    ///   "hello world reedline".into(),
    ///   "this is the reedline crate".into(),
    /// ];
    /// let mut line_editor =
    /// Reedline::create().with_highlighter(Box::new(ExampleHighlighter::new(commands)));
    /// ```
    #[must_use]
    pub fn with_highlighter(mut self, highlighter: Box<dyn Highlighter>) -> Self {
        self.highlighter = highlighter;
        self
    }

    /// A builder that configures the style used for visual selection
    #[must_use]
    pub fn with_visual_selection_style(mut self, style: Style) -> Self {
        self.visual_selection_style = style;
        self
    }

    /// A builder that gives the cell under the cursor its own style inside a
    /// visual selection, the way helix styles its primary cursor distinctly
    /// from the selection around it. Left unset, the cell keeps the plain
    /// selection style, which can paint a flat selection (e.g. reverse video)
    /// over the terminal cursor and hide it.
    #[must_use]
    pub fn with_visual_selection_cursor_style(mut self, style: Style) -> Self {
        self.visual_selection_cursor_style = Some(style);
        self
    }

    /// A builder which configures the history for your instance of the Reedline engine
    ///
    /// To keep no history, pass a [`FileBackedHistory`] without a file and with a capacity
    /// of 0, `FileBackedHistory::new(0)`, which saves nothing.
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with history support, including history size limits
    ///
    /// use reedline::{FileBackedHistory, Reedline};
    ///
    /// let history = Box::new(
    /// FileBackedHistory::with_file(5, "history.txt".into())
    ///     .expect("Error configuring history with file"),
    /// );
    /// let mut line_editor = Reedline::create()
    ///     .with_history(history);
    /// ```
    #[must_use]
    pub fn with_history(mut self, history: Box<dyn History>) -> Self {
        self.history = history;
        self
    }

    /// A builder which configures history exclusion for your instance of the Reedline engine
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline instance with history that will *not* include commands starting with a space
    ///
    /// use reedline::{FileBackedHistory, Reedline};
    ///
    /// let history = Box::new(
    /// FileBackedHistory::with_file(5, "history.txt".into())
    ///     .expect("Error configuring history with file"),
    /// );
    /// let mut line_editor = Reedline::create()
    ///     .with_history(history)
    ///     .with_history_exclusion_prefix(Some(" ".into()));
    /// ```
    #[must_use]
    pub fn with_history_exclusion_prefix(mut self, ignore_prefix: Option<String>) -> Self {
        self.history_exclusion_prefix = ignore_prefix;
        self
    }

    /// A builder that configures the validator for your instance of the Reedline engine
    /// # Example
    /// ```rust
    /// // Create a reedline object with validator support
    ///
    /// use reedline::{DefaultValidator, Reedline};
    ///
    /// let mut line_editor =
    /// Reedline::create().with_validator(Box::new(DefaultValidator));
    /// ```
    #[must_use]
    pub fn with_validator(mut self, validator: Box<dyn Validator>) -> Self {
        self.validator = Some(validator);
        self
    }

    /// A builder that configures the alternate text editor used to edit the line buffer
    ///
    /// You are responsible for providing a file path that is unique to this reedline session
    ///
    /// # Example
    /// ```rust,no_run
    /// // Create a reedline object with vim as editor
    ///
    /// use reedline::Reedline;
    /// use std::env::temp_dir;
    /// use std::process::Command;
    ///
    /// let temp = std::env::temp_dir().join("my-random-unique.file");
    /// let mut command = Command::new("vim");
    /// // you can provide additional flags:
    /// command.arg("-p"); // open in a new vim tab
    /// // ...and the filename will be appended at the end of the command
    /// let mut line_editor = Reedline::create().with_buffer_editor(command, temp.clone());
    ///
    /// // optionally, {file}, {line}, and {col} placeholders can be used.
    /// // they will be replaced with the filename and the current cursor position,
    /// // both 1-based, with {col} counting graphemes from the line start
    /// let mut command = Command::new("hx");
    /// command.arg("{file}:{line}:{col}");
    /// let mut line_editor = Reedline::create().with_buffer_editor(command, temp.clone());
    ///
    /// // if {file} is omitted, the filename is still appended at the end,
    /// // as in the above example
    /// let mut command = Command::new("emacs");
    /// command.arg("+{line}:{col}");
    /// let mut line_editor = Reedline::create().with_buffer_editor(command, temp);
    /// ```
    ///
    /// A command with placeholders is rebuilt on every `OpenEditor` from its
    /// program, arguments, environment and working directory. Other settings on
    /// the `Command`, such as stdio redirection or `env_clear`, do not carry over.
    /// A command without placeholders is spawned as given.
    #[must_use]
    pub fn with_buffer_editor(
        mut self,
        command: impl Into<EditorCommand>,
        temp_file: PathBuf,
    ) -> Self {
        self.buffer_editor = Some(BufferEditor::new(command.into(), temp_file));
        self
    }

    /// Remove the current [`Validator`]
    #[must_use]
    pub fn disable_validator(mut self) -> Self {
        self.validator = None;
        self
    }

    /// Set a different prompt to be used after submitting each line
    #[must_use]
    pub fn with_transient_prompt(mut self, transient_prompt: Box<dyn Prompt>) -> Self {
        self.transient_prompt = Some(transient_prompt);
        self
    }

    /// A builder that configures semantic prompt markers for terminal integration.
    ///
    /// This enables semantic prompt support for terminals that support it, such as Ghostty.
    /// Use `Osc133Markers::boxed()` for standard terminal support or `Osc633Markers::boxed()`
    /// for VS Code integrated terminal support.
    #[must_use]
    pub fn with_semantic_markers(
        mut self,
        markers: Option<Box<dyn SemanticPromptMarkers>>,
    ) -> Self {
        self.painter.set_semantic_markers(markers);
        self
    }

    /// A builder which configures the edit mode for your instance of the Reedline engine
    #[must_use]
    pub fn with_edit_mode(mut self, edit_mode: Box<dyn EditMode>) -> Self {
        self.edit_mode = edit_mode;
        self
    }

    /// A builder that registers another edit mode as a standby, which a
    /// [`ReedlineEvent::SwitchMode`] can activate. The mode given to
    /// [`with_edit_mode`](Self::with_edit_mode) stays active until then, and
    /// the mode a switch replaces becomes a standby in turn.
    ///
    /// Registering appends, so a host that rebuilds its modes on every prompt
    /// clears them with [`clear_edit_modes`](Self::clear_edit_modes) first.
    #[must_use]
    pub fn with_additional_edit_mode(mut self, edit_mode: Box<dyn EditMode>) -> Self {
        self.standby_edit_modes.push(edit_mode);
        self
    }

    /// A builder that clears the standby edit modes registered with
    /// [`with_additional_edit_mode`](Self::with_additional_edit_mode), leaving
    /// the active one alone.
    #[must_use]
    pub fn clear_edit_modes(mut self) -> Self {
        self.standby_edit_modes.clear();
        self
    }

    /// A builder that adds a menu to the engine
    ///
    /// Menus are looked up by name (see [`ReedlineEvent::Menu`]), so a menu
    /// whose name is already taken replaces that menu, keeping its place in
    /// the order. The last menu added under a name wins, the same as for a
    /// key bound twice in [`crate::Keybindings`].
    #[must_use]
    pub fn with_menu(mut self, menu: ReedlineMenu) -> Self {
        match self
            .menus
            .iter_mut()
            .find(|existing| existing.name() == menu.name())
        {
            Some(existing) => *existing = menu,
            None => self.menus.push(menu),
        }
        self
    }

    /// A builder that clears the list of menus added to the engine
    #[must_use]
    pub fn clear_menus(mut self) -> Self {
        self.menus = Vec::new();
        self
    }

    /// A builder that adds abbreviations to the Reedline engine
    ///
    /// Overwrites any existing abbreviations with the same key.
    ///
    /// Note, by default abbreviations are expanded everywhere. To suppress expansion in certain
    /// syntactic positions (e.g. string literals), override [`Highlighter::should_expand_abbr`].
    pub fn with_abbreviations(mut self, abbreviations: HashMap<String, String>) -> Self {
        self.abbreviations.extend(abbreviations);
        self
    }

    /// A builder that adds the history item id
    #[must_use]
    pub fn with_history_session_id(mut self, session: Option<HistorySessionId>) -> Self {
        self.history_session_id = session;
        self
    }

    /// A builder that enables reedline changing the cursor shape based on the current edit mode.
    /// The current implementation sets the cursor shape when drawing the prompt.
    /// Do not use this if the cursor shape is set elsewhere, e.g. in the terminal settings or by ansi escape sequences.
    pub fn with_cursor_config(mut self, cursor_shapes: CursorConfig) -> Self {
        self.cursor_shapes = Some(cursor_shapes);
        self
    }

    /// A builder that configures whether reedline should immediately accept the input.
    pub fn with_immediately_accept(mut self, immediately_accept: bool) -> Self {
        self.immediately_accept = immediately_accept;
        self
    }

    /// A builder that configures an external break signal.
    ///
    /// When the [`AtomicBool`] is set to `true` by an external thread,
    /// [`Reedline::read_line()`] will return [`Signal::ExternalBreak`] with the
    /// current buffer contents. The flag is automatically reset to `false`
    /// after being consumed.
    pub fn with_break_signal(mut self, signal: Arc<AtomicBool>) -> Self {
        self.break_signal = Some(signal);
        self
    }

    /// Get a [`RepaintSignal`] handle that can trigger an in-place repaint of
    /// the prompt from another thread while [`Reedline::read_line()`] is
    /// running, avoiding interfering with current line edit.
    pub fn repaint_signal(&mut self) -> RepaintSignal {
        // The handle is created lazily on the first call; subsequent calls return clones
        self.repaint_signal
            .get_or_insert_with(RepaintSignal::default)
            .clone()
    }

    /// Returns the corresponding expected prompt style for the given edit mode
    pub fn prompt_edit_mode(&self) -> PromptEditMode {
        self.edit_mode.edit_mode()
    }

    /// Output the complete [`History`] chronologically with numbering to the terminal
    pub fn print_history(&mut self) -> Result<()> {
        let history: Vec<_> = self
            .history
            .search(SearchQuery::everything(SearchDirection::Forward, None))?;

        for (i, entry) in history.iter().enumerate() {
            self.print_line(&format!("{}\t{}", i, entry.command_line))?;
        }
        Ok(())
    }

    /// Output the complete [`History`] for this session, chronologically with numbering to the terminal
    pub fn print_history_session(&mut self) -> Result<()> {
        let history: Vec<_> = self.history.search(SearchQuery::everything(
            SearchDirection::Forward,
            self.get_history_session_id(),
        ))?;

        for (i, entry) in history.iter().enumerate() {
            self.print_line(&format!("{}\t{}", i, entry.command_line))?;
        }
        Ok(())
    }

    /// Print the history session id
    pub fn print_history_session_id(&mut self) -> Result<()> {
        println!("History Session Id: {:?}", self.get_history_session_id());
        Ok(())
    }

    /// Toggle between having a history that uses the history session id and one that does not
    pub fn toggle_history_session_matching(
        &mut self,
        session: Option<HistorySessionId>,
    ) -> Result<()> {
        self.history_session_id = match self.get_history_session_id() {
            Some(_) => None,
            None => session,
        };
        Ok(())
    }

    /// Read-only view of the history
    pub fn history(&self) -> &dyn History {
        &*self.history
    }

    /// Mutable view of the history
    pub fn history_mut(&mut self) -> &mut dyn History {
        &mut *self.history
    }

    /// Update the underlying [`History`] to/from disk
    pub fn sync_history(&mut self) -> std::io::Result<()> {
        // TODO: check for interactions in the non-submitting events
        self.history.sync()
    }

    /// Check if the last [`Reedline::read_line`] submitted a command.
    ///
    /// The context lasts until the next `read_line` starts. A call that returns
    /// anything but a non-empty [`Signal::Success`] leaves it unset, and calling
    /// [`Self::update_last_command_context`] then fails with a "No command run" error.
    pub fn has_last_command_context(&self) -> bool {
        matches!(
            self.history_last_run,
            HistoryLastRun::Stored(_) | HistoryLastRun::Excluded { current: true, .. }
        )
    }

    /// update the last history item with more information
    pub fn update_last_command_context(
        &mut self,
        f: &dyn Fn(HistoryItem) -> HistoryItem,
    ) -> crate::Result<()> {
        match mem::take(&mut self.history_last_run) {
            HistoryLastRun::Excluded {
                item,
                current: true,
                recalled,
            } => {
                self.history_last_run = HistoryLastRun::Excluded {
                    item: f(item),
                    current: true,
                    recalled,
                };
                Ok(())
            }
            HistoryLastRun::Stored(r) => {
                self.history_last_run = HistoryLastRun::Stored(r);
                self.history.update(r, f)
            }
            stale @ (HistoryLastRun::Excluded { current: false, .. } | HistoryLastRun::Empty) => {
                self.history_last_run = stale;
                Err(ReedlineError(ReedlineErrorVariants::OtherHistoryError(
                    "No command run",
                )))
            }
        }
    }

    /// Take the error of the last failed history save, if any.
    ///
    /// [`read_line`](Self::read_line) still returns the line when the [`History`]
    /// refuses to store it; the entry is then treated like an excluded one.
    /// Cleared on read, set at most once per `read_line`.
    pub fn take_history_save_error(&mut self) -> Option<ReedlineError> {
        self.history_save_error.take()
    }

    /// Wait for input and provide the user with a specified [`Prompt`].
    ///
    /// Returns a [`std::io::Result`] in which the `Err` type is [`std::io::Result`]
    /// and the `Ok` variant wraps a [`Signal`] which handles user inputs.
    pub fn read_line(&mut self, prompt: &dyn Prompt) -> Result<Signal> {
        terminal::enable_raw_mode()?;
        self.bracketed_paste.enter();
        self.kitty_protocol.enter();

        let result = self.read_line_helper(prompt);
        if result.is_err() {
            // The error is what to report, not a failure to show.
            let _ignore = self.painter.show_cursor();
        }

        self.bracketed_paste.exit();
        self.kitty_protocol.exit();
        terminal::disable_raw_mode()?;
        result
    }

    /// Returns the current insertion point of the input buffer.
    pub fn current_insertion_point(&self) -> usize {
        self.editor.insertion_point()
    }

    /// Returns the current contents of the input buffer.
    pub fn current_buffer_contents(&self) -> &str {
        self.editor.get_buffer()
    }

    /// Returns the current selection range of the input buffer, otherwise None.
    pub fn current_selection(&self) -> Option<(usize, usize)> {
        self.editor.get_selection()
    }

    /// Writes `msg` to the terminal with a following carriage return and newline,
    /// its control characters shown rather than sent
    fn print_line(&mut self, msg: &str) -> Result<()> {
        self.painter.paint_line(&escape_control(msg))
    }

    /// Clear the screen by printing enough whitespace to start the prompt or
    /// other output back at the first line of the terminal.
    pub fn clear_screen(&mut self) -> Result<()> {
        self.painter.clear_screen()?;
        self.painter.show_cursor()
    }

    /// Clear the screen and the scrollback buffer of the terminal
    pub fn clear_scrollback(&mut self) -> Result<()> {
        self.painter.clear_scrollback()?;
        self.painter.show_cursor()
    }

    /// Consume a pending external repaint request, returning whether one was
    /// pending. Any number of requests since the last check collapse into one.
    fn take_repaint_request(&self) -> bool {
        self.repaint_signal
            .as_ref()
            .is_some_and(RepaintSignal::take)
    }

    /// Whether the input loop must poll with a timeout instead of blocking
    /// indefinitely, so external triggers (break signal, repaint signal,
    /// external printer, idle callback) are noticed while waiting for input.
    fn input_needs_polling(&self) -> bool {
        #[allow(unused_mut)] // Dependent on feature flags
        let mut poll = self.break_signal.is_some()
            || self
                .repaint_signal
                .as_ref()
                .is_some_and(|sig| Arc::strong_count(&sig.flag) > 1);

        {
            poll |= self.external_printer.is_some();
        }

        poll |= self.idle_callback.is_some();

        poll
    }

    /// Reset the per-call state at the start of a `read_line`, before anything
    /// can fail.
    ///
    /// Returns the state of a suspended editor (ExecuteHostCommand or
    /// ExternalBreak) for the painter to resume from.
    fn begin_read(&mut self) -> Option<PainterSuspendedState> {
        self.hide_hints = false;

        // The last command context belongs to the previous call. Only a
        // submit in this call may set it again, so a host command, Ctrl-C
        // or an empty Enter can't reach back and update an older entry.
        // An excluded item stays, since Up still recalls it.
        if let HistoryLastRun::Excluded { current, .. } = &mut self.history_last_run {
            *current = false;
        } else {
            self.history_last_run = HistoryLastRun::Empty;
        }

        // Repaint requests raised while no read_line was active are stale:
        // the fresh prompt painted below already reflects the latest state.
        self.take_repaint_request();

        self.suspended_state.take()
    }

    /// Helper implementing the logic for [`Reedline::read_line()`] to be wrapped
    /// in a `raw_mode` context.
    fn read_line_helper(&mut self, prompt: &dyn Prompt) -> Result<Signal> {
        let suspended_state = self.begin_read();
        self.painter
            .initialize_prompt_position(suspended_state.as_ref())?;
        self.repaint(prompt)?;

        loop {
            // Call idle callback if set (for processing external events like GUI updates)
            if let Some(ref mut callback) = self.idle_callback {
                callback();
                // The callback owns stdout while it runs and may have
                // written or moved the cursor. Re-verify the anchor on
                // the next paint.
                self.painter.invalidate_prompt_start_row();
            }

            if let Some(ref signal) = self.break_signal {
                if signal.swap(false, std::sync::atomic::Ordering::Relaxed) {
                    let buffer = self.editor.get_buffer().to_string();
                    self.input_mode = InputMode::Regular;
                    self.last_render_snapshot = None;
                    self.suspended_state = Some(self.painter.state_before_suspension());
                    self.editor.reset_undo_stack();
                    return Ok(Signal::ExternalBreak(buffer));
                }
            }

            if self.take_repaint_request() {
                self.repaint(prompt)?;
            }

            if let Some(printer) = self.external_printer.as_mut() {
                let messages = printer.drain()?;
                if !messages.is_empty() {
                    self.painter.print_external_message(messages)?;
                    self.repaint(prompt)?;
                }
            }

            // Determine if we need to poll (non-blocking) or can block on input.
            // We need polling if external_printer or idle_callback is configured,
            // using the shared poll_interval for the timeout.
            let status = self.completer.poll_completion();
            // Anything BUT idle means work is in flight. We need to keep polling.
            let completer_pending = status != CompletionStatus::Idle;

            if status == CompletionStatus::Ready {
                self.settle_completions(prompt)?;
            }

            let mut events: Vec<Event> = vec![];

            if !self.immediately_accept {
                if self.input_needs_polling() || completer_pending {
                    if event::poll(self.poll_interval)? {
                        events.push(crossterm::event::read()?);
                    }
                } else {
                    // Block until we receive an event
                    events.push(crossterm::event::read()?);
                }

                // Receive all events in the queue without blocking. Will stop when
                // a line of input is completed.
                while !Self::line_input_completed(&events) && event::poll(Duration::from_millis(0))?
                {
                    events.push(crossterm::event::read()?);
                }

                // If we believe there's text pasting or resizing going on, batch
                // more events at the cost of a slight delay.
                if events.len() > EVENTS_THRESHOLD
                    || events.iter().any(|e| matches!(e, Event::Resize(_, _)))
                {
                    while !Self::line_input_completed(&events) && event::poll(POLL_WAIT)? {
                        events.push(crossterm::event::read()?);
                    }
                }

                // Paste-burst oracle. Feed the just-collected plain chars to the
                // burst detector (a multi-char poll(0) batch is itself a paste
                // signal — normal typing yields ~1 char per read-loop iteration),
                // then, while a real burst is coalescing, keep draining the queue
                // past the `line_input_completed()`/`EVENTS_THRESHOLD` stops so a multi-line
                // paste lands in one batch. Fully gated on an installed hook, so
                // the no-hook path above is byte-for-byte unchanged.
                if let Some(hook) = self.paste_burst.clone() {
                    for c in events.iter().filter_map(burst_char) {
                        hook.on_char(c);
                    }
                    if hook.is_burst_active() {
                        loop {
                            if event::poll(hook.poll_timeout())? {
                                let event = crossterm::event::read()?;
                                if let Some(c) = burst_char(&event) {
                                    hook.on_char(c);
                                }
                                events.push(event);
                            } else {
                                // Idle: the burst has settled, but do NOT reset
                                // the detector here — `process_input_batch` below
                                // still needs the live burst flag to reclassify
                                // this batch's embedded Enters as newlines. The
                                // reset happens once after the batch is processed
                                // (see `settle_paste_burst`).
                                break;
                            }
                        }
                    }
                }
            }

            // Process the batch unconditionally: in `immediately_accept` mode
            // `events` stays empty, but `process_input_batch` still pushes the
            // synthetic `Submit` and returns the buffer. Gating this call behind
            // `!immediately_accept` would spin the loop forever.
            let batch_result = self.process_input_batch(prompt, events)?;
            self.settle_paste_burst();
            if let ControlFlow::Break(signal) = batch_result {
                return Ok(signal);
            }
        }
    }

    /// Returns true if the last collected event ends a line of input and the
    /// batch can be handed to the hosting application.
    ///
    /// Besides a plain `Enter`, a `Ctrl+J` (what a piped LF parses to in raw
    /// mode) also ends a line: without this, several LF-terminated lines that
    /// arrive in one drain merge into a single batch, and every line after the
    /// first is silently dropped when the batch breaks on the first submit.
    ///
    /// `Ctrl+J` is `Enter` in every insert keymap (emacs, vi and helix
    /// insert). In the other vi and helix modes it is unbound, so ending the
    /// drain there only adds a loop iteration and the line is unaffected.
    fn line_input_completed(events: &[Event]) -> bool {
        if let Some(event) = events.last() {
            matches!(
                event,
                Event::Key(KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    ..
                }) | Event::Key(KeyEvent {
                    code: KeyCode::Char('j'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                })
            )
        } else {
            false
        }
    }

    /// Reset the paste-burst detector once the batch that held a burst has been
    /// processed, which releases the burst flag before the next independent line
    /// (so a later human Enter submits instead of being absorbed).
    ///
    /// A batch without a burst does not reset it: the detector tells typing from
    /// a paste by its own clock. Some event sources hand over a single event per
    /// loop iteration, and resetting after each of those batches would keep the
    /// detector from ever counting up to its threshold.
    ///
    /// Kept out of the input loop so it is reachable from tests, which cannot
    /// drive the loop itself.
    fn settle_paste_burst(&self) {
        if let Some(hook) = &self.paste_burst {
            if hook.is_burst_active() {
                hook.settle();
            }
        }
    }

    /// Fold a finished completion request into the active menu, and honor the completions
    /// owed from when the request was made.
    ///
    /// Called when the completer reports [`CompletionStatus::Ready`]. Kept out of the
    /// input loop so it is reachable from tests, which cannot drive the loop itself.
    fn settle_completions(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let Some(menu_index) = self.menus.iter().position(|menu| menu.is_active()) else {
            // No menu to answer to, so nothing can still be owed.
            self.deferred_menu_completion = None;
            return Ok(());
        };

        // The request that just finished was host code, which may have scrolled the
        // terminal while it ran in the background — after the keystroke that
        // dispatched it had already re-verified the anchor. `update_values` below
        // can also dispatch another request for a line that moved on. Either way
        // the repaint at the end of this settle must not trust the cached row.
        invalidate_anchor_if_host_completer_runs(&self.menus[menu_index], &mut self.painter);

        let menu = &mut self.menus[menu_index];
        // The request this menu was waiting on finished, so repopulate it.
        menu.update_values(
            &mut self.editor,
            self.completer.as_mut(),
            self.history.as_ref(),
        );
        let still_provisional = menu.results_are_provisional();

        let owed = self
            .deferred_menu_completion
            .as_ref()
            .is_some_and(|deferred| deferred.still_applies(menu, &self.editor));

        // Values were just refreshed above, so the menu must not re-fetch them.
        let accept_lone_value =
            owed && !still_provisional && self.decide_menu_completion(menu_index, true);

        // Spent once a final answer had its say, so it cannot act on a later one.
        // Provisional results decided nothing, so the arm outlives them.
        if !still_provisional {
            self.deferred_menu_completion = None;
        }

        if accept_lone_value {
            // With a menu active this replaces in the buffer and deactivates, rather than
            // submitting the line.
            self.handle_editor_event(prompt, ReedlineEvent::Enter)?;
        }

        // One paint for every outcome, since painting the menu and then the accepted or
        // extended line would flicker on each completion.
        self.repaint(prompt)
    }

    /// The completion an opening menu applies to the line: a lone suggestion is accepted
    /// outright, otherwise the prefix the suggestions share is spliced in. Returns whether
    /// the caller should accept that lone suggestion.
    ///
    /// Both the [`Menu`](ReedlineEvent::Menu) event and the deferred replay in
    /// [`settle_completions`](Self::settle_completions) decide this, and they have to
    /// decide it identically. `values_updated` says whether the caller already refreshed
    /// the menu's values, so they are not fetched twice.
    fn decide_menu_completion(&mut self, menu_index: usize, values_updated: bool) -> bool {
        let menu = &mut self.menus[menu_index];

        let accept_lone_value = if self.quick_completions && menu.can_quick_complete() {
            if !values_updated {
                menu.update_values(
                    &mut self.editor,
                    self.completer.as_mut(),
                    self.history.as_ref(),
                );
            }
            // Accepting a lone *stale* value is refused downstream, since its span
            // belongs to another line, so the menu would close over a completion that
            // never happened.
            menu.get_values().len() == 1 && !menu.results_are_provisional()
        } else {
            false
        };

        if !accept_lone_value && self.partial_completions {
            menu.can_partially_complete(
                values_updated || self.quick_completions,
                &mut self.editor,
                self.completer.as_mut(),
                self.history.as_ref(),
            );
        }

        accept_lone_value
    }

    fn process_input_batch(
        &mut self,
        prompt: &dyn Prompt,
        events: Vec<Event>,
    ) -> Result<ControlFlow<Signal>> {
        // Convert `Event` into `ReedlineEvent`. Also, fuse consecutive
        // `ReedlineEvent::EditCommand` into one. Also, if there're multiple
        // `ReedlineEvent::Resize`, only keep the last one.
        let mut reedline_events: Vec<ReedlineEvent> = vec![];
        // A detected paste burst is handled as ONE unit, bypassing the per-event
        // parse below. Routing each pasted char through `parse_event` is fragile:
        // any event that maps to a non-`Edit` reedline event (a `/` slash-menu
        // trigger, a menu/history action, an un-reclassified Enter) hits the
        // fusing branch that FLUSHES the accumulated raw edits mid-loop
        // (`std::mem::take`), committing raw pasted text to the buffer before a
        // placeholder can replace it — the raw-text-leak + stray-newline bug.
        // Instead, collect the whole burst's content directly (chars + embedded
        // newlines, in order) and emit exactly one `InsertString`: a placeholder
        // from the host, or the raw text if the host declines to reference-ify
        // it. Fully gated on `burst_batch` (only true when a hook is installed
        // AND a real burst was detected), so normal typing and the no-hook path
        // are byte-for-byte unchanged.
        let burst_batch = self
            .paste_burst
            .as_ref()
            .is_some_and(|h| h.is_burst_active());
        // A resize is kept whichever arm parses the batch: the burst drain
        // pushes every event it reads, so a resize arriving mid-paste lands in
        // the burst batch too and must not be swallowed there.
        let mut resize = None;
        if burst_batch {
            let mut coalesced = String::new();
            // Every Enter drained into this batch is coalesced as an embedded
            // newline, never a submit. An Enter only reaches this loop by
            // being drained while `is_burst_active` stayed latched true, which
            // only happens inside the poll-timeout idle window that keeps the
            // burst coalescing (see the drain loop above) — i.e. it arrived at
            // machine paste speed, not from a human keypress. A real human
            // submit Enter, typed after the paste, lands past that idle
            // window: the drain loop has already stopped and `settle` has
            // already run by the time it is read, so it starts the *next*
            // batch instead, where it is handled as an ordinary submit.
            for event in &events {
                if let Some(c) = burst_char(event) {
                    coalesced.push(c);
                    continue;
                }
                if is_pasted_newline(event) {
                    coalesced.push('\n');
                    continue;
                }
                match event {
                    // A pasted tab is part of the text. Left to the catch-all
                    // below it would be dropped from the insert.
                    Event::Key(KeyEvent {
                        code: KeyCode::Tab,
                        modifiers: KeyModifiers::NONE,
                        kind: KeyEventKind::Press,
                        ..
                    }) => {
                        coalesced.push('\t');
                    }
                    Event::Resize(x, y) => resize = Some((*x, *y)),
                    // Release events and any other keys are paste artifacts here.
                    _ => {}
                }
            }
            if !coalesced.is_empty() {
                let insert = self
                    .paste_burst
                    .clone()
                    .and_then(|h| h.resolve_burst(&coalesced))
                    .unwrap_or(coalesced);
                reedline_events.push(ReedlineEvent::Edit(vec![EditCommand::InsertString(insert)]));
            }
        } else {
            let mut edits = vec![];
            for event in events {
                // Reclassify a newline key (bare `Enter`, or `Ctrl-J` for a raw
                // LF) that the oracle judges paste-embedded into a newline even
                // when a full burst was NOT detected (a short fast paste, e.g.
                // `aa\nbb`, whose lines never reach the burst char threshold),
                // so it does not submit mid-paste. Only a `Press` is
                // reclassified (the kitty enhancement also emits a Release); a
                // Release is rejected downstream by `ReedlineRawEvent::try_from`
                // and inserts nothing.
                if let Some(hook) = &self.paste_burst {
                    if is_pasted_newline(&event) && hook.enter_is_newline() {
                        edits.push(EditCommand::InsertNewline);
                        continue;
                    }
                }
                if let Ok(event) = ReedlineRawEvent::try_from(event) {
                    match self.edit_mode.parse_event(event) {
                        ReedlineEvent::Edit(edit) => edits.extend(edit),
                        ReedlineEvent::Resize(x, y) => resize = Some((x, y)),
                        event => {
                            if !edits.is_empty() {
                                reedline_events
                                    .push(ReedlineEvent::Edit(std::mem::take(&mut edits)));
                            }
                            reedline_events.push(event);
                        }
                    }
                }
            }
            if !edits.is_empty() {
                reedline_events.push(ReedlineEvent::Edit(edits));
            }
        }
        if let Some((x, y)) = resize {
            reedline_events.push(ReedlineEvent::Resize(x, y));
        }
        // The synthetic `Submit` of `immediately_accept` mode is pushed after
        // both arms: nothing is read in that mode, so `events` is empty and the
        // burst arm would insert nothing, but a hook that reports
        // `is_burst_active()` before anything was fed to it must still not
        // keep `read_line` spinning without a submit.
        if self.immediately_accept {
            reedline_events.push(ReedlineEvent::Submit);
        }

        // The mode machine has parsed this batch, so the rest policy it
        // declares is now final. Relay it to the editor before running the
        // emitted commands so a command a mode transition issued (e.g. the
        // Esc→normal grapheme step-back) resolves under the new policy. This
        // does not commit the cursor — the commands settle it, and the
        // pre-paint `set_edit_mode` below still clamps no-command switches.
        self.editor.sync_edit_mode(self.edit_mode.edit_mode());

        // Handle reedline events.
        let mut need_repaint = false;
        for event in reedline_events {
            match self.handle_event(prompt, event)? {
                EventStatus::Exits(signal) => {
                    // Check if we are merely suspended (to process an ExecuteHostCommand event)
                    // or if we're about to quit the editor.
                    if self.suspended_state.is_none() {
                        // We are about to quit the editor, move the cursor below the input
                        // area, for external commands or new read_line call
                        self.painter.move_cursor_to_end()?;
                    }
                    // A clear earlier in this batch left it hidden and no
                    // paint follows. The exit is what to report, so a failed
                    // show must not turn it into an error.
                    let _ignore = self.painter.show_cursor();
                    return Ok(ControlFlow::Break(signal));
                }
                EventStatus::Handled => {
                    need_repaint = true;
                }
                EventStatus::Inapplicable => {
                    // Nothing changed, no need to repaint
                }
            }
        }
        // A command-less mode transition adopts a new rest policy via
        // `sync_edit_mode` but emits nothing to commit the cursor. Force the
        // settle (and a repaint) so it doesn't stay unsettled until the next
        // command.
        if self.editor.policy_unsettled() {
            need_repaint = true;
        }
        if need_repaint {
            // Sync the editor's edit mode before painting so the cursor is
            // normalized under the current rest policy. A mode change that
            // bypasses the command path (e.g. Esc → Vi normal) otherwise
            // wouldn't clamp until the next command, painting the cursor past
            // the last grapheme for a frame.
            let mode = self.edit_mode.edit_mode();
            self.editor.set_edit_mode(mode);
            self.repaint(prompt)?;
        }
        Ok(ControlFlow::Continue(()))
    }

    fn handle_event(&mut self, prompt: &dyn Prompt, event: ReedlineEvent) -> Result<EventStatus> {
        if self.input_mode == InputMode::HistorySearch {
            self.handle_history_search_event(event)
        } else {
            self.handle_editor_event(prompt, event)
        }
    }

    fn handle_history_search_event(&mut self, event: ReedlineEvent) -> io::Result<EventStatus> {
        match event {
            ReedlineEvent::UntilFound(events) => {
                for event in events {
                    match self.handle_history_search_event(event)? {
                        EventStatus::Inapplicable => {
                            // Try again with the next event handler
                        }
                        success => {
                            return Ok(success);
                        }
                    }
                }
                // No candidate applied, so nothing changed: report that, which
                // also lets an enclosing `UntilFound` keep trying.
                Ok(EventStatus::Inapplicable)
            }
            ReedlineEvent::CtrlD => {
                if self.editor.is_empty() {
                    self.input_mode = InputMode::Regular;
                    self.editor.reset_undo_stack();
                    Ok(EventStatus::Exits(Signal::CtrlD))
                } else {
                    self.run_history_commands(&[EditCommand::Delete])?;
                    Ok(EventStatus::Handled)
                }
            }
            ReedlineEvent::CtrlC => {
                self.input_mode = InputMode::Regular;
                // The line from before the search, not the search query. It is
                // abandoned like any other, so the next read starts empty.
                let buffer = self.editor.get_buffer().to_string();
                self.run_edit_commands(&[EditCommand::Clear]);
                self.editor.reset_undo_stack();
                Ok(EventStatus::Exits(Signal::CtrlC(buffer)))
            }
            ReedlineEvent::ClearScreen => {
                self.painter.clear_screen()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::ClearScrollback => {
                self.painter.clear_scrollback()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Enter
            | ReedlineEvent::HistoryHintComplete
            | ReedlineEvent::Submit
            | ReedlineEvent::SubmitOrNewline => {
                if let Some(string) = self.history_cursor.string_at_cursor() {
                    self.editor
                        .set_buffer(string, UndoBehavior::CreateUndoPoint);
                }

                self.input_mode = InputMode::Regular;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::ExecuteHostCommand(host_command) => {
                self.last_render_snapshot = None;
                self.suspended_state = Some(self.painter.state_before_suspension());
                Ok(EventStatus::Exits(Signal::HostCommand(host_command)))
            }
            ReedlineEvent::Edit(commands) => {
                self.run_history_commands(&commands)?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Mouse {
                column,
                row,
                button,
            } => {
                if button == MouseButton::Left {
                    self.handle_mouse_click(column, row)?;
                }
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Resize(width, height) => {
                self.last_render_snapshot = None;
                self.painter.handle_resize(width, height);
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Repaint => {
                // A handled Event causes a repaint
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::PreviousHistory | ReedlineEvent::Up | ReedlineEvent::SearchHistory => {
                self.history_cursor.back(self.history.as_ref())?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::NextHistory | ReedlineEvent::Down => {
                self.history_cursor.forward(self.history.as_ref())?;
                // Hacky way to ensure that we don't fall of into failed search going forward
                if self.history_cursor.string_at_cursor().is_none() {
                    self.history_cursor.back(self.history.as_ref())?;
                }
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Esc => {
                self.input_mode = InputMode::Regular;
                Ok(EventStatus::Handled)
            }
            // TODO: Check if events should be handled
            ReedlineEvent::Right
            | ReedlineEvent::Left
            | ReedlineEvent::ToStart
            | ReedlineEvent::ToEnd
            | ReedlineEvent::FirstHistory
            | ReedlineEvent::LeaveHistory
            | ReedlineEvent::Multiple(_)
            | ReedlineEvent::None
            | ReedlineEvent::HistoryHintWordComplete
            | ReedlineEvent::OpenEditor
            | ReedlineEvent::Menu(_)
            | ReedlineEvent::MenuAccept
            | ReedlineEvent::MenuNext
            | ReedlineEvent::MenuPrevious
            | ReedlineEvent::MenuUp
            | ReedlineEvent::MenuDown
            | ReedlineEvent::MenuLeft
            | ReedlineEvent::MenuRight
            | ReedlineEvent::MenuPageNext
            | ReedlineEvent::MenuPagePrevious
            | ReedlineEvent::SwitchMode(_) => Ok(EventStatus::Inapplicable),
        }
    }

    fn handle_editor_event(
        &mut self,
        prompt: &dyn Prompt,
        event: ReedlineEvent,
    ) -> io::Result<EventStatus> {
        match event {
            ReedlineEvent::Menu(name) => {
                if self.active_menu().is_none() {
                    if let Some(index) = self.menus.iter().position(|menu| menu.name() == name) {
                        self.menus[index].menu_event(MenuEvent::Activate(self.quick_completions));
                        invalidate_anchor_if_host_completer_runs(
                            &self.menus[index],
                            &mut self.painter,
                        );

                        if self.decide_menu_completion(index, false) {
                            return self.handle_editor_event(prompt, ReedlineEvent::Enter);
                        }

                        // A final answer already had its say above, so only a
                        // provisional one leaves anything owed. With neither option set
                        // nothing queried the completer, so this reads the default
                        // `false` and the menu paints as it always has.
                        if self.menus[index].results_are_provisional() {
                            self.deferred_menu_completion = Some(DeferredMenuCompletion::new(
                                &self.menus[index],
                                &self.editor,
                            ));
                        }

                        return Ok(EventStatus::Handled);
                    }
                }
                Ok(EventStatus::Inapplicable)
            }
            ReedlineEvent::MenuAccept => {
                // Where a menu's claim on a keypress is decided, for the submit
                // events that offer it one as much as for a binding that wants
                // the accept alone. A menu declines the keypress unless it can
                // splice something: with no suggestions there is nothing to
                // take, and a *stale* value is refused downstream because its
                // span belongs to another line, so accepting one would close
                // the menu over a completion that never happened. Declining
                // leaves the key to reach the line instead of appearing dead.
                match self.menus.iter_mut().find(|menu| menu.is_active()) {
                    Some(menu)
                        if !menu.get_values().is_empty() && !menu.results_are_provisional() =>
                    {
                        menu.replace_in_buffer(&mut self.editor);
                        menu.menu_event(MenuEvent::Deactivate);
                        Ok(EventStatus::Handled)
                    }
                    _ => Ok(EventStatus::Inapplicable),
                }
            }
            ReedlineEvent::MenuNext => {
                if let Some(menu) = self.menus.iter_mut().find(|menu| menu.is_active()) {
                    // The second route to the lone-value accept, so it carries the same
                    // provisional guard as `decide_menu_completion`: accepting a lone
                    // *stale* value is refused downstream, but the `Enter` would still
                    // deactivate the menu, closing it over a completion that never
                    // happened.
                    if menu.get_values().len() == 1
                        && menu.can_quick_complete()
                        && !menu.results_are_provisional()
                    {
                        self.handle_editor_event(prompt, ReedlineEvent::Enter)
                    } else {
                        if self.partial_completions {
                            menu.can_partially_complete(
                                self.quick_completions,
                                &mut self.editor,
                                self.completer.as_mut(),
                                self.history.as_ref(),
                            );
                            invalidate_anchor_if_host_completer_runs(menu, &mut self.painter);
                        }
                        menu.menu_event(MenuEvent::NextElement);
                        Ok(EventStatus::Handled)
                    }
                } else {
                    Ok(EventStatus::Inapplicable)
                }
            }
            ReedlineEvent::MenuPrevious => {
                self.active_menu()
                    .map_or(Ok(EventStatus::Inapplicable), |menu| {
                        menu.menu_event(MenuEvent::PreviousElement);
                        Ok(EventStatus::Handled)
                    })
            }
            ReedlineEvent::MenuUp => {
                self.active_menu()
                    .map_or(Ok(EventStatus::Inapplicable), |menu| {
                        menu.menu_event(MenuEvent::MoveUp);
                        Ok(EventStatus::Handled)
                    })
            }
            ReedlineEvent::MenuDown => {
                self.active_menu()
                    .map_or(Ok(EventStatus::Inapplicable), |menu| {
                        menu.menu_event(MenuEvent::MoveDown);
                        Ok(EventStatus::Handled)
                    })
            }
            ReedlineEvent::MenuLeft => {
                self.active_menu()
                    .map_or(Ok(EventStatus::Inapplicable), |menu| {
                        menu.menu_event(MenuEvent::MoveLeft);
                        Ok(EventStatus::Handled)
                    })
            }
            ReedlineEvent::MenuRight => {
                self.active_menu()
                    .map_or(Ok(EventStatus::Inapplicable), |menu| {
                        menu.menu_event(MenuEvent::MoveRight);
                        Ok(EventStatus::Handled)
                    })
            }
            // These two spell out `active_menu()`, since that borrows all of `self` and
            // the painter has to stay reachable alongside the menu.
            ReedlineEvent::MenuPageNext => {
                match self.menus.iter_mut().find(|menu| menu.is_active()) {
                    Some(menu) => {
                        menu.menu_event(MenuEvent::NextPage);
                        invalidate_anchor_if_host_completer_runs(menu, &mut self.painter);
                        Ok(EventStatus::Handled)
                    }
                    None => Ok(EventStatus::Inapplicable),
                }
            }
            ReedlineEvent::MenuPagePrevious => {
                match self.menus.iter_mut().find(|menu| menu.is_active()) {
                    Some(menu) => {
                        menu.menu_event(MenuEvent::PreviousPage);
                        invalidate_anchor_if_host_completer_runs(menu, &mut self.painter);
                        Ok(EventStatus::Handled)
                    }
                    None => Ok(EventStatus::Inapplicable),
                }
            }
            ReedlineEvent::HistoryHintComplete => Ok(self.accept_history_hint(true)),
            ReedlineEvent::HistoryHintWordComplete => Ok(self.accept_history_hint(false)),
            ReedlineEvent::Esc => {
                // Nothing to cancel is reported as such, so a binding like
                // `UntilFound([Esc, ExecuteHostCommand(..)])` can fall through.
                let had_menu = self.active_menu().is_some();
                let had_selection = self.editor.cursor_is_a_selection();
                self.deactivate_menus();
                self.editor.clear_selection();
                Ok(if had_menu || had_selection {
                    EventStatus::Handled
                } else {
                    EventStatus::Inapplicable
                })
            }
            ReedlineEvent::CtrlD => {
                if self.editor.is_empty() {
                    self.editor.reset_undo_stack();
                    Ok(EventStatus::Exits(Signal::CtrlD))
                } else {
                    self.run_edit_commands(&[EditCommand::Delete]);
                    Ok(EventStatus::Handled)
                }
            }
            ReedlineEvent::CtrlC => {
                self.deactivate_menus();
                let buffer = self.editor.get_buffer().to_string();
                self.run_edit_commands(&[EditCommand::Clear]);
                self.editor.reset_undo_stack();
                Ok(EventStatus::Exits(Signal::CtrlC(buffer)))
            }
            ReedlineEvent::ClearScreen => {
                self.deactivate_menus();
                self.painter.clear_screen()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::ClearScrollback => {
                self.deactivate_menus();
                self.painter.clear_scrollback()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Enter => {
                if self.menu_took_the_key(prompt)? {
                    return Ok(EventStatus::Handled);
                }
                #[cfg(feature = "bashisms")]
                if let Some(event) = self.parse_bang_command() {
                    return self.handle_editor_event(prompt, event);
                }
                if let Some(event) = self.try_expand_abbreviation_at_cursor(true) {
                    self.handle_editor_event(prompt, event)?;
                }

                let buffer = self.editor.get_buffer().to_string();
                match self.validator.as_mut().map(|v| v.validate(&buffer)) {
                    None | Some(ValidationResult::Complete) => Ok(self.submit_buffer(prompt)?),
                    Some(ValidationResult::Incomplete) => {
                        self.run_edit_commands(&[EditCommand::InsertNewline]);

                        Ok(EventStatus::Handled)
                    }
                }
            }
            ReedlineEvent::Submit => {
                if self.menu_took_the_key(prompt)? {
                    return Ok(EventStatus::Handled);
                }
                #[cfg(feature = "bashisms")]
                if let Some(event) = self.parse_bang_command() {
                    return self.handle_editor_event(prompt, event);
                }
                if let Some(event) = self.try_expand_abbreviation_at_cursor(true) {
                    self.handle_editor_event(prompt, event)?;
                }

                Ok(self.submit_buffer(prompt)?)
            }
            ReedlineEvent::SubmitOrNewline => {
                if self.menu_took_the_key(prompt)? {
                    return Ok(EventStatus::Handled);
                }
                #[cfg(feature = "bashisms")]
                if let Some(event) = self.parse_bang_command() {
                    return self.handle_editor_event(prompt, event);
                }
                if let Some(event) = self.try_expand_abbreviation_at_cursor(true) {
                    self.handle_editor_event(prompt, event)?;
                }

                let cursor_position_in_buffer = self.editor.insertion_point();
                let buffer = self.editor.get_buffer().to_string();
                if cursor_position_in_buffer < buffer.len() {
                    self.run_edit_commands(&[EditCommand::InsertNewline]);
                    return Ok(EventStatus::Handled);
                }
                match self.validator.as_mut().map(|v| v.validate(&buffer)) {
                    None | Some(ValidationResult::Complete) => Ok(self.submit_buffer(prompt)?),
                    Some(ValidationResult::Incomplete) => {
                        self.run_edit_commands(&[EditCommand::InsertNewline]);

                        Ok(EventStatus::Handled)
                    }
                }
            }
            ReedlineEvent::ExecuteHostCommand(host_command) => {
                self.last_render_snapshot = None;
                self.suspended_state = Some(self.painter.state_before_suspension());
                Ok(EventStatus::Exits(Signal::HostCommand(host_command)))
            }
            ReedlineEvent::Edit(commands) => {
                // Intercept a lone `PasteSystem` when a paste interceptor
                // is installed. The hook reads the clipboard itself and decides
                // what to insert (a reference placeholder, the raw text, or
                // nothing) — bypassing the default clipboard-read-and-insert.
                // Only a lone `PasteSystem` command is intercepted; any other
                // edit (or a compound batch) falls through to the normal path
                // unchanged. `PasteSystem` only exists under `system_clipboard`,
                // so gate the whole interception on that feature. The text the
                // hook returns is run as an ordinary `InsertString` through the
                // rest of this arm, so an open menu sees the edit exactly as it
                // would for a plain `PasteSystem`; a `Noop` changes nothing and
                // is done here.
                #[cfg(feature = "system_clipboard")]
                let commands = match (commands.as_slice(), self.paste_interceptor.clone()) {
                    ([EditCommand::PasteSystem], Some(interceptor)) => match interceptor.on_paste()
                    {
                        crate::PasteAction::InsertText(s) => vec![EditCommand::InsertString(s)],
                        crate::PasteAction::Noop => return Ok(EventStatus::Handled),
                    },
                    _ => commands,
                };

                let status = self.run_edit_commands_with_status(&commands);

                // Ahead of everything below, which would otherwise ask a
                // completer about a word the user has already left, or return
                // through the abbreviation expansion without ever looking.
                if status == EditCommandStatus::Applied {
                    self.close_a_menu_past_its_word(&commands);
                }

                // Check if a space was just inserted and try to expand abbreviations
                if status == EditCommandStatus::Applied
                    && matches!(commands.first(), Some(EditCommand::InsertChar(' ')))
                {
                    if let Some(event) = self.try_expand_abbreviation_at_cursor(false) {
                        return self.handle_editor_event(prompt, event);
                    }
                }

                if let Some(menu) = self.menus.iter_mut().find(|menu| menu.is_active()) {
                    let quick_complete = self.quick_completions && menu.can_quick_complete();
                    // Dismissing a menu handles the event even when the cursor didn't move.
                    if quick_complete
                        && !self.persistent_menus
                        && matches!(
                            commands.first(),
                            Some(EditCommand::Backspace)
                                | Some(EditCommand::BackspaceWord)
                                | Some(EditCommand::MoveToLineStart { select: false })
                        )
                    {
                        menu.menu_event(MenuEvent::Deactivate);
                        return Ok(EventStatus::Handled);
                    }
                    if status == EditCommandStatus::Inapplicable {
                        return Ok(EventStatus::Inapplicable);
                    }
                    if quick_complete {
                        menu.menu_event(MenuEvent::Edit(self.quick_completions));
                        invalidate_anchor_if_host_completer_runs(menu, &mut self.painter);
                        menu.update_values(
                            &mut self.editor,
                            self.completer.as_mut(),
                            self.history.as_ref(),
                        );
                        if let Some(&EditCommand::Complete) = commands.first() {
                            if menu.get_values().len() == 1 {
                                return self.handle_editor_event(prompt, ReedlineEvent::Enter);
                            } else if self.partial_completions
                                && menu.can_partially_complete(
                                    self.quick_completions,
                                    &mut self.editor,
                                    self.completer.as_mut(),
                                    self.history.as_ref(),
                                )
                            {
                                return Ok(EventStatus::Handled);
                            }
                        }
                    }
                    if !self.persistent_menus && self.editor.line_buffer().get_buffer().is_empty() {
                        menu.menu_event(MenuEvent::Deactivate);
                    } else {
                        menu.menu_event(MenuEvent::Edit(self.quick_completions));
                        invalidate_anchor_if_host_completer_runs(menu, &mut self.painter);
                    }
                }
                Ok(status.into())
            }
            ReedlineEvent::OpenEditor => self.open_editor().map(|_| EventStatus::Handled),
            ReedlineEvent::Resize(width, height) => {
                self.last_render_snapshot = None;
                self.painter.handle_resize(width, height);
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Repaint => {
                // A handled Event causes a repaint
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::PreviousHistory => {
                self.previous_history()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::NextHistory => {
                self.next_history()?;
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::FirstHistory => self.first_history(),
            ReedlineEvent::LeaveHistory => Ok(self.leave_history()),
            ReedlineEvent::Up => self.up_command(),
            ReedlineEvent::Down => self.down_command(),
            ReedlineEvent::Left | ReedlineEvent::Right => {
                let command = if event == ReedlineEvent::Left {
                    EditCommand::MoveLeft { select: false }
                } else {
                    EditCommand::MoveRight { select: false }
                };
                Ok(self.run_edit_commands_with_status(&[command]).into())
            }
            ReedlineEvent::ToStart | ReedlineEvent::ToEnd => {
                let initial_cursor = self.editor.line_buffer().cursor();
                // Keep these aliases outside the edit-command path: they do not
                // create undo points or end history traversal.
                if event == ReedlineEvent::ToStart {
                    self.editor.move_to_start(false);
                } else {
                    self.editor.move_to_end(false);
                }
                // Compare after settling under the rest policy, including vi's
                // block caret resting on the last grapheme rather than past it.
                self.editor.commit_cursor();
                Ok(if self.editor.line_buffer().cursor() == initial_cursor {
                    EventStatus::Inapplicable
                } else {
                    EventStatus::Handled
                })
            }
            ReedlineEvent::SearchHistory => {
                self.enter_history_search();
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::Multiple(events) => {
                let mut latest_signal = EventStatus::Inapplicable;
                for event in events {
                    match self.handle_editor_event(prompt, event)? {
                        EventStatus::Handled => {
                            latest_signal = EventStatus::Handled;
                        }
                        EventStatus::Inapplicable => {
                            // NO OP
                        }
                        EventStatus::Exits(signal) => {
                            // TODO: Check if we want to allow execution to
                            // proceed if there are more events after the
                            // terminating
                            return Ok(EventStatus::Exits(signal));
                        }
                    }
                }

                Ok(latest_signal)
            }
            ReedlineEvent::UntilFound(events) => {
                for event in events {
                    match self.handle_editor_event(prompt, event)? {
                        EventStatus::Inapplicable => {
                            // Try again with the next event handler
                        }
                        success => {
                            return Ok(success);
                        }
                    }
                }
                // No candidate applied, so nothing changed: report that, which
                // also lets an enclosing `UntilFound` keep trying.
                Ok(EventStatus::Inapplicable)
            }
            ReedlineEvent::SwitchMode(_) => Ok(self.change_edit_mode(event)),
            ReedlineEvent::Mouse {
                column,
                row,
                button,
            } => {
                if button == MouseButton::Left {
                    self.handle_mouse_click(column, row)?;
                }
                Ok(EventStatus::Handled)
            }
            ReedlineEvent::None => Ok(EventStatus::Inapplicable),
        }
    }

    /// Route a `SwitchMode` event to the machine that accepts it, make that
    /// machine the active one, then repair the cursor the flip left behind.
    ///
    /// Routing: a target naming the state the active machine already reports
    /// is not a move, so it is declined before any machine is asked. Otherwise
    /// the active machine is offered the target first, then the standbys in
    /// registration order; the first to accept is swapped in. When none
    /// accepts nothing changes either. Both `Inapplicable` answers let an
    /// enclosing `UntilFound` keep trying, which is what makes
    /// `UntilFound([SwitchMode(A), SwitchMode(B)])` a toggle.
    ///
    /// Repair: a machine's own transitions emit their repairs as events, the way `i`
    /// collapses the selection on the way into helix insert. An event-driven
    /// flip never reaches that path, so the repair has to happen here. What
    /// bites is a selection arriving in a mode that has none: a block policy
    /// rests as a min-width-1 selection and a bar caret can carry a
    /// shift-selection, and either way `insert_char` and every vi operator
    /// consume a live selection, so the first keystroke would eat text. It
    /// collapses onto the caret, where the machines' own `Esc` leaves it.
    ///
    /// Stated over the `RestPolicy` being entered rather than per machine, so
    /// emacs, both insert modes and vi normal are one rule instead of four
    /// cases, and a future machine inherits it.
    fn change_edit_mode(&mut self, event: ReedlineEvent) -> EventStatus {
        if let ReedlineEvent::SwitchMode(target) = &event {
            if *target == self.edit_mode.edit_mode() {
                return EventStatus::Inapplicable;
            }
        }

        let mut status = self.edit_mode.handle_mode_specific_event(event.clone());
        if matches!(status, EventStatus::Inapplicable) {
            // Offering is not a query: the standby that accepts has already
            // moved into the target state, and `EditMode` asks the ones that
            // decline to stay as they were.
            for standby in &mut self.standby_edit_modes {
                if let EventStatus::Handled = standby.handle_mode_specific_event(event.clone()) {
                    std::mem::swap(&mut self.edit_mode, standby);
                    status = EventStatus::Handled;
                    break;
                }
            }
        }

        let after = self.edit_mode.edit_mode();
        if matches!(status, EventStatus::Handled) && !after.rest_policy().is_block() {
            // Collapse first, while the editor still holds the mode being left
            // and so knows where its caret shows. Adopting the new policy
            // afterwards leaves the settle to the pre-paint commit; a commit
            // under the block policy being left would re-widen the point.
            self.editor.clear_selection();
            self.editor.sync_edit_mode(after);
        }
        status
    }

    fn handle_mouse_click(&mut self, column: u16, row: u16) -> Result<()> {
        let snapshot = match &self.last_render_snapshot {
            Some(snapshot) => snapshot,
            None => return Ok(()),
        };
        if self.input_mode != InputMode::Regular {
            return Ok(());
        }

        // Try to select the entry under the pointer if a menu is active
        if snapshot.menu_active {
            let Some(menu_start_row) = snapshot.menu_start_row else {
                return Ok(());
            };
            let Some(menu_row) = row.checked_sub(menu_start_row) else {
                return Ok(());
            };
            let terminal_columns = snapshot.screen_width;
            let available_lines = snapshot.screen_height.saturating_sub(menu_start_row);
            self.accept_menu_click(column, menu_row, terminal_columns, available_lines);
            return Ok(());
        }

        let buffer = self.editor.get_buffer();
        if let Some(offset) = self.painter.screen_to_buffer_offset(snapshot, column, row) {
            if buffer.is_char_boundary(offset) {
                self.editor.edit_buffer(
                    |buf| buf.set_insertion_point(offset),
                    UndoBehavior::MoveCursor,
                );
            }
        }
        Ok(())
    }

    fn accept_menu_click(
        &mut self,
        column: u16,
        menu_row: u16,
        terminal_columns: u16,
        available_lines: u16,
    ) {
        let Some(menu) = self.menus.iter_mut().find(|menu| menu.is_visible()) else {
            return;
        };
        let Some(index) = menu.selection_at(column, menu_row, terminal_columns, available_lines)
        else {
            return;
        };

        menu.set_selection(index);
    }

    fn active_menu(&mut self) -> Option<&mut ReedlineMenu> {
        self.menus.iter_mut().find(|menu| menu.is_active())
    }

    fn deactivate_menus(&mut self) {
        // Nothing is left to complete into.
        self.deferred_menu_completion = None;
        self.menus
            .iter_mut()
            .for_each(|menu| menu.menu_event(MenuEvent::Deactivate));
    }

    fn previous_history(&mut self) -> io::Result<()> {
        self.history_last_run.set_recalled(false);
        if self.input_mode != InputMode::HistoryTraversal {
            self.input_mode = InputMode::HistoryTraversal;
            self.history_cursor = HistoryCursor::new(
                self.get_history_navigation_based_on_line_buffer(),
                self.get_history_session_id(),
            );

            if matches!(self.history_last_run, HistoryLastRun::Excluded { .. }) {
                self.history_last_run.set_recalled(true);
            }
        }

        if !self.history_last_run.is_recalled() {
            // On `Err` the next press retries on the fresh cursor; no rollback.
            self.history_cursor.back(self.history.as_ref())?;
        }
        self.show_history_entry(HistoryLanding::FirstLineEnd);
        Ok(())
    }

    /// Puts the history cursor's entry (or the draft past the newest one) in
    /// the buffer, with the cursor where `landing` says.
    fn show_history_entry(&mut self, landing: HistoryLanding) {
        self.update_buffer_from_history();
        match landing {
            HistoryLanding::FirstLineEnd => {
                self.editor.move_to_start(false);
                self.editor.move_to_line_end(false);
            }
            HistoryLanding::End => self.editor.move_to_end(false),
        }
        // History navigation positions the cursor outside the command path, so
        // settle it under the rest policy (vi-normal must not rest past the line).
        self.editor.commit_cursor();
        self.editor
            .update_undo_state(UndoBehavior::HistoryNavigation);
    }

    fn first_history(&mut self) -> io::Result<EventStatus> {
        if self.input_mode != InputMode::HistoryTraversal {
            self.history_cursor = HistoryCursor::new(
                self.get_history_navigation_based_on_line_buffer(),
                self.get_history_session_id(),
            );
        }
        if !self.history_cursor.jump_to_oldest(self.history.as_ref())? {
            return Ok(EventStatus::Inapplicable);
        }

        self.input_mode = InputMode::HistoryTraversal;
        self.history_last_run.set_recalled(false);
        self.show_history_entry(HistoryLanding::FirstLineEnd);
        Ok(EventStatus::Handled)
    }

    fn leave_history(&mut self) -> EventStatus {
        if self.input_mode != InputMode::HistoryTraversal {
            return EventStatus::Inapplicable;
        }

        self.history_cursor.past_newest();
        self.history_last_run.set_recalled(false);
        self.input_mode = InputMode::Regular;
        self.show_history_entry(HistoryLanding::End);
        EventStatus::Handled
    }

    fn next_history(&mut self) -> io::Result<()> {
        if self.input_mode != InputMode::HistoryTraversal {
            self.input_mode = InputMode::HistoryTraversal;
            self.history_cursor = HistoryCursor::new(
                self.get_history_navigation_based_on_line_buffer(),
                self.get_history_session_id(),
            );
        }

        if self.history_last_run.is_recalled() {
            self.history_last_run.set_recalled(false);
        } else {
            let cursor_was_on_item = self.history_cursor.string_at_cursor().is_some();
            self.history_cursor.forward(self.history.as_ref())?;

            if cursor_was_on_item
                && self.history_cursor.string_at_cursor().is_none()
                && matches!(self.history_last_run, HistoryLastRun::Excluded { .. })
            {
                self.history_last_run.set_recalled(true);
            }
        }

        if self.history_cursor.string_at_cursor().is_none() && !self.history_last_run.is_recalled()
        {
            self.input_mode = InputMode::Regular;
        }
        self.show_history_entry(HistoryLanding::End);
        Ok(())
    }

    /// Enable the search and navigation through the history from the line buffer prompt
    ///
    /// Enables either prefix search with output in the line buffer or simple traversal
    fn get_history_navigation_based_on_line_buffer(&self) -> HistoryNavigationQuery {
        if self.editor.is_empty() || !self.editor.is_cursor_at_buffer_end() {
            // Perform bash-style basic up/down entry walking
            HistoryNavigationQuery::Normal(
                // Hack: Tight coupling point to be able to restore previously typed input
                self.editor.line_buffer().clone(),
            )
        } else {
            // Prefix search like found in fish, zsh, etc.
            // Search string is set once from the current buffer
            // Current setup (code in other methods)
            // Continuing with typing will leave the search
            // but next invocation of this method will start the next search
            let buffer = self.editor.get_buffer().to_string();
            HistoryNavigationQuery::PrefixSearch(buffer)
        }
    }

    /// Switch into reverse history search mode
    ///
    /// This mode uses a separate prompt and handles keybindings slightly differently!
    fn enter_history_search(&mut self) {
        self.history_cursor = HistoryCursor::new(
            HistoryNavigationQuery::SubstringSearch("".to_string()),
            self.get_history_session_id(),
        );
        self.input_mode = InputMode::HistorySearch;
    }

    /// Dispatches the applicable [`EditCommand`] actions for editing the history search string.
    ///
    /// Only modifies internal state, does not perform regular output!
    fn run_history_commands(&mut self, commands: &[EditCommand]) -> io::Result<()> {
        for command in commands {
            match command {
                EditCommand::InsertChar(c) => {
                    let navigation = self.history_cursor.get_navigation();
                    if let HistoryNavigationQuery::SubstringSearch(mut substring) = navigation {
                        substring.push(*c);
                        self.history_cursor = HistoryCursor::new(
                            HistoryNavigationQuery::SubstringSearch(substring),
                            self.get_history_session_id(),
                        );
                    } else {
                        self.history_cursor = HistoryCursor::new(
                            HistoryNavigationQuery::SubstringSearch(String::from(*c)),
                            self.get_history_session_id(),
                        );
                    }
                    self.history_cursor.back(self.history.as_mut())?;
                }
                EditCommand::Backspace => {
                    let navigation = self.history_cursor.get_navigation();

                    if let HistoryNavigationQuery::SubstringSearch(substring) = navigation {
                        let new_substring = text_manipulation::remove_last_grapheme(&substring);

                        self.history_cursor = HistoryCursor::new(
                            HistoryNavigationQuery::SubstringSearch(new_substring.to_string()),
                            self.get_history_session_id(),
                        );
                        self.history_cursor.back(self.history.as_mut())?
                    }
                }
                _ => {
                    self.input_mode = InputMode::Regular;
                }
            }
        }
        Ok(())
    }

    /// Set the buffer contents for history traversal/search in the standard prompt
    ///
    /// When using the up/down traversal or fish/zsh style prefix search update the main line buffer accordingly.
    /// Not used for the separate modal reverse search!
    fn update_buffer_from_history(&mut self) {
        if self.history_last_run.is_recalled() {
            if let HistoryLastRun::Excluded { item, .. } = &self.history_last_run {
                self.editor
                    .set_buffer(item.command_line.clone(), UndoBehavior::HistoryNavigation);
            }
            return;
        }

        match self.history_cursor.get_navigation() {
            HistoryNavigationQuery::Normal(original) => {
                if let Some(buffer_to_paint) = self.history_cursor.string_at_cursor() {
                    self.editor
                        .set_buffer(buffer_to_paint, UndoBehavior::HistoryNavigation);
                } else {
                    // Hack
                    self.editor
                        .set_line_buffer(original, UndoBehavior::HistoryNavigation);
                }
            }
            HistoryNavigationQuery::PrefixSearch(prefix)
            | HistoryNavigationQuery::SubstringSearch(prefix) => {
                let buffer = self.history_cursor.string_at_cursor().unwrap_or(prefix);
                self.editor
                    .set_buffer(buffer, UndoBehavior::HistoryNavigation);
            }
        }
    }

    /// Executes [`EditCommand`] actions by modifying the internal state appropriately. Does not output itself.
    pub fn run_edit_commands(&mut self, commands: &[EditCommand]) {
        self.run_edit_commands_with_status(commands);
    }

    fn run_edit_commands_with_status(&mut self, commands: &[EditCommand]) -> EditCommandStatus {
        let status = self.apply_edit_commands(commands);
        if status == EditCommandStatus::Applied && self.input_mode == InputMode::HistoryTraversal {
            self.input_mode = InputMode::Regular;
        }
        status
    }

    /// [`run_edit_commands`](Self::run_edit_commands) without ending history
    /// traversal, for the engine's own line moves inside a recalled entry.
    fn apply_edit_commands(&mut self, commands: &[EditCommand]) -> EditCommandStatus {
        // Adopt the current edit mode's rest policy so these commands resolve
        // under it (e.g. block-caret selection geometry) — but *without*
        // committing the cursor first. A commit here would apply the policy's
        // resting rule (e.g. `OnGrapheme` pulling an at-end point back) before
        // the commands run, double-stepping a mode-transition backstep like the
        // vi `Esc`→normal `MoveLeft`. The commands settle the cursor themselves,
        // and the pre-paint `set_edit_mode` makes the final commit.
        self.editor.sync_edit_mode(self.edit_mode.edit_mode());

        // Run every command, but report the edit as applied if any command applied.
        // This mirrors `Multiple`, where any handled event wins.
        let mut status = EditCommandStatus::Inapplicable;

        for command in commands {
            if let Some(command) = self.auto_pair_command(command) {
                if self.editor.run_edit_command(&command) == EditCommandStatus::Applied {
                    status = EditCommandStatus::Applied;
                }
                continue;
            }

            if self.editor.run_edit_command(command) == EditCommandStatus::Applied {
                status = EditCommandStatus::Applied;
            }
        }

        status
    }

    fn auto_pair_command(&self, command: &EditCommand) -> Option<EditCommand> {
        let auto_pairs = self.auto_pairs.as_ref()?;

        // A menu in `Diff` mode reads what is typed after it opened as its
        // query, not as code, so a pair would end up in the search.
        let query_menu_open = self.menus.iter().any(|menu| {
            menu.is_active()
                && menu.settings().effective_input_mode() == crate::menu::InputMode::Diff
        });
        if query_menu_open {
            return None;
        }

        // Resolve which auto-pair action (if any) `command` would trigger, along
        // with the pair it acts on and the `EditCommand` that would replace it.
        // The search order matters: for `InsertChar`, closers are checked before
        // openers, so a character that is configured as both a closer of one pair
        // and an opener of another resolves based on whether the cursor currently
        // sits on the closer, not on the order pairs were registered in.
        let (pair, action, converted) = match command {
            EditCommand::InsertChar(ch) => {
                let closer_at_cursor = auto_pairs
                    .closing_pair(*ch)
                    .filter(|(_, close)| self.editor.is_auto_pair_closer_at_cursor(*close));

                if let Some(pair) = closer_at_cursor {
                    (
                        pair,
                        AutoPairAction::SkipExistingCloser,
                        EditCommand::MoveRight { select: false },
                    )
                } else if let Some((open, close)) = auto_pairs.opening_pair(*ch) {
                    (
                        (open, close),
                        AutoPairAction::Open,
                        EditCommand::InsertPair { open, close },
                    )
                } else {
                    return None;
                }
            }
            EditCommand::Backspace => {
                let pair = auto_pairs.pairs().find(|(open, close)| {
                    self.editor.is_empty_auto_pair_at_cursor(*open, *close)
                })?;
                (
                    pair,
                    AutoPairAction::BackspacePair,
                    EditCommand::BackspacePair {
                        open: pair.0,
                        close: pair.1,
                    },
                )
            }
            _ => return None,
        };

        // Give the highlighter a chance to veto the action before committing to
        // it. All three actions pass through this same gate: returning `false`
        // means "run the original command verbatim", handled by the caller
        // treating `None` from this function as a pass-through.
        let buffer = self.editor.get_buffer();
        let insertion_point = self.editor.insertion_point();
        let selection = self.editor.get_selection().map(|(start, end)| start..end);
        let context = AutoPairContext::new(buffer, insertion_point, pair, selection, action);

        if self.highlighter.should_auto_pair(&context) {
            Some(converted)
        } else {
            None
        }
    }

    /// Move the cursor up a line, or from the first line walk back into
    /// history. Reports `Inapplicable` when there was nowhere to go: the
    /// cursor on the first line and history already at its oldest entry, or
    /// empty. An honest report is what lets a binding built on
    /// [`ReedlineEvent::UntilFound`] fall through to its next event, as
    /// `Left` and `Right` already do at the edges of the line.
    fn up_command(&mut self) -> io::Result<EventStatus> {
        // If we're at the top, then:
        if self.editor.is_cursor_at_first_line() {
            // If we're at the top, move to previous history
            self.walk_and_report(Self::previous_history)
        } else {
            // Through `apply_edit_commands` so the cursor settles under the mode's
            // rest policy — a bare `editor.move_line_up` skips the commit boundary,
            // leaving a vi-normal caret past the last grapheme on a short line.
            Ok(self
                .apply_edit_commands(&[EditCommand::MoveLineUp { select: false }])
                .into())
        }
    }

    /// Move the cursor down a line, or from the last line walk forward through
    /// history. Reports `Inapplicable` when there was nowhere to go: the cursor
    /// on the last line of the line being typed, with nothing newer below it.
    /// See [`up_command`](Self::up_command).
    fn down_command(&mut self) -> io::Result<EventStatus> {
        // If we're at the bottom, then:
        if self.editor.is_cursor_at_last_line() {
            // If we're at the bottom, move to next history
            self.walk_and_report(Self::next_history)
        } else {
            // See `up_command`: settle under the rest policy via the commit boundary.
            Ok(self
                .apply_edit_commands(&[EditCommand::MoveLineDown { select: false }])
                .into())
        }
    }

    /// Run a history walk and report whether it moved anything: the buffer's
    /// text, or the cursor within it. The walk itself says nothing, since at
    /// either end of history it is a no-op by design, so the answer is read
    /// off the editor before and after.
    fn walk_and_report(
        &mut self,
        walk: fn(&mut Self) -> io::Result<()>,
    ) -> io::Result<EventStatus> {
        let text_before = self.editor.get_buffer().to_string();
        let cursor_before = self.editor.insertion_point();
        walk(self)?;
        let unmoved = self.editor.get_buffer() == text_before
            && self.editor.insertion_point() == cursor_before;
        Ok(if unmoved {
            EventStatus::Inapplicable
        } else {
            EventStatus::Handled
        })
    }

    /// Checks if hints should be displayed and are able to be completed
    fn hints_active(&self) -> bool {
        !self.hide_hints && matches!(self.input_mode, InputMode::Regular)
    }

    /// Accept the history hint, all of it when `whole`, else its next word.
    /// `Handled` only when a non-empty hint applies: hints active, no menu
    /// open, and the cursor either at the buffer end or, for a hinter that
    /// continues the text before the cursor, followed only by auto-pair
    /// closers the hint closes.
    ///
    /// At the buffer end the text is appended, positioned past the last
    /// grapheme first: a block caret (vi normal) rests *on* it, so a plain
    /// insert would split it. Before trailing closers the text goes in at the
    /// cursor: the closers it closes, as the highlighter says, are replaced by
    /// the ones it contains, and the others stay after the cursor.
    fn accept_history_hint(&mut self, whole: bool) -> EventStatus {
        let Some(hinter) = self.hinter.as_ref() else {
            return EventStatus::Inapplicable;
        };
        let full = hinter.complete_hint();
        let accepted = if whole {
            full.clone()
        } else {
            hinter.next_hint_token()
        };
        if !self.hints_active() || accepted.is_empty() || self.active_menu().is_some() {
            return EventStatus::Inapplicable;
        }

        // Checked before the buffer end: a block caret resting on the last
        // closer counts as at the end, but the text belongs before it.
        if let Some(closers) = self.trailing_closers_for_hint().map(str::to_owned) {
            let offsets = closer_offsets_for_hint(
                self.highlighter.as_ref(),
                self.editor.get_buffer(),
                self.editor.insertion_point(),
                &full,
                &closers,
            );
            // A partial acceptance must be a prefix of the hint to know which
            // closers it has reached.
            let Some(offsets) = offsets.filter(|_| full.starts_with(&accepted)) else {
                return EventStatus::Inapplicable;
            };
            let closed = offsets.iter().filter(|&&at| at < accepted.len()).count();
            let left_over = closers
                .char_indices()
                .nth(closed)
                .map_or("", |(index, _)| &closers[index..]);
            self.editor.sync_edit_mode(self.edit_mode.edit_mode());
            self.editor.replace_to_line_end(&accepted, left_over);
            return EventStatus::Handled;
        }

        if self.editor.is_cursor_at_buffer_end() {
            self.editor.prepare_append_at_buffer_end();
            self.run_edit_commands(&[EditCommand::InsertString(accepted)]);
            EventStatus::Handled
        } else {
            EventStatus::Inapplicable
        }
    }

    /// The text after the cursor when it is nothing but closers of the
    /// configured auto-pairs, like the `)` in `(gs|)`.
    fn trailing_auto_pair_closers(&self) -> Option<&str> {
        let auto_pairs = self.auto_pairs.as_ref()?;
        let after = &self.editor.get_buffer()[self.editor.insertion_point()..];
        let only_closers = after.chars().all(|c| auto_pairs.closing_pair(c).is_some());
        (!after.is_empty() && only_closers).then_some(after)
    }

    /// The trailing auto-pair closers a history hint may stand in for: those
    /// of [`Self::trailing_auto_pair_closers`], when the hinter continues the
    /// text before the cursor and nothing else claims the cursor area, which a
    /// selection or an open menu does.
    fn trailing_closers_for_hint(&self) -> Option<&str> {
        let hinter = self.hinter.as_ref()?;
        let free = hinter.continues_text_before_cursor()
            && self.hints_active()
            && !self.menus.iter().any(|menu| menu.is_active())
            && !self.editor.cursor_is_a_selection();
        free.then(|| self.trailing_auto_pair_closers()).flatten()
    }

    /// Repaint of either the buffer or the parts for reverse history search
    fn repaint(&mut self, prompt: &dyn Prompt) -> io::Result<()> {
        // Repainting
        if self.input_mode == InputMode::HistorySearch {
            self.history_search_paint(prompt)
        } else {
            self.buffer_paint(prompt)
        }
    }

    /// Expands an abbreviation at the word before the cursor, if any exists
    ///
    /// Calls [`Highlighter::should_expand_abbr`] with [`AbbrExpandContext::WordAbbreviation`]
    /// to decide whether expansion is permitted at the cursor position
    fn try_expand_abbreviation_at_cursor(&mut self, submitted: bool) -> Option<ReedlineEvent> {
        let buffer = self.editor.get_buffer();
        let cursor_position_in_buffer = self.editor.insertion_point();
        if cursor_position_in_buffer == 0 {
            return None;
        }

        let (offset, suffix) = match submitted {
            true => (0, ""),   // expand on <enter>
            false => (1, " "), // expand on <space>
        };

        // `offset` is a raw byte count (0 on <enter>, 1 on <space>), so
        // `cursor_position_in_buffer - offset` can land inside a multi-byte
        // UTF-8 char sitting just before the cursor (e.g. pasted CJK text).
        // Floor it down to the nearest char boundary before slicing to avoid
        // a panic.
        let word_end =
            crate::menu_functions::floor_char_boundary(buffer, cursor_position_in_buffer - offset);
        let prefix = &buffer[..word_end];
        let word_start = prefix
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map(|(idx, ch)| idx + ch.len_utf8())
            .unwrap_or(0); // byte offset of word start

        if word_start >= word_end {
            // The first char in the buffer is a space or there are consecutive spaces
            return None;
        }

        if submitted
            && buffer[word_end..]
                .chars()
                .next()
                .is_some_and(|ch| !ch.is_whitespace())
        {
            // The cursor is in the middle of a word, e.g. "hello|world"
            return None;
        }

        if !self.highlighter.should_expand_abbr(
            buffer,
            word_start,
            AbbrExpandContext::WordAbbreviation,
        ) {
            return None;
        }

        let word = &buffer[word_start..word_end];
        if let Some(expansion) = self.abbreviations.get(word) {
            return Some(ReedlineEvent::Edit(vec![
                EditCommand::MoveToPosition {
                    position: word_start,
                    select: false,
                },
                EditCommand::MoveToPosition {
                    // Select through the cursor, not just the end of the word, so
                    // the triggering space (already inserted on a <space> expansion)
                    // is replaced rather than left beside the inserted suffix.
                    position: cursor_position_in_buffer,
                    select: true,
                },
                EditCommand::InsertString(format!("{}{}", expansion, suffix)),
            ]));
        }

        None
    }

    #[cfg(feature = "bashisms")]
    /// Parses the ! command to replace entries from the history
    fn parse_bang_command(&mut self) -> Option<ReedlineEvent> {
        let buffer = self.editor.get_buffer();
        let parsed = parse_selection_char(buffer, '!');
        let parsed_prefix = parsed.prefix.unwrap_or_default().to_string();
        let parsed_marker = parsed.marker.unwrap_or_default().to_string();

        if let Some(last) = parsed.remainder.chars().last() {
            if last != ' ' {
                return None;
            }
        }

        if !self.highlighter.should_expand_abbr(
            buffer,
            parsed.remainder.len(),
            AbbrExpandContext::BangExpansion,
        ) {
            return None;
        }

        let history_result = parsed
            .index
            .zip(parsed.marker)
            .and_then(|(index, indicator)| match parsed.action {
                ParseAction::LastCommand => self
                    .history
                    .search(SearchQuery {
                        direction: SearchDirection::Backward,
                        start_time: None,
                        end_time: None,
                        start_id: None,
                        end_id: None,
                        limit: Some(1), // fetch the latest one entries
                        filter: SearchFilter::anything(self.get_history_session_id()),
                    })
                    .unwrap_or_else(|_| Vec::new())
                    .get(index.saturating_sub(1))
                    .map(|history| {
                        (
                            parsed.remainder.len(),
                            indicator.len(),
                            history.command_line.clone(),
                        )
                    }),
                ParseAction::BackwardSearch => self
                    .history
                    .search(SearchQuery {
                        direction: SearchDirection::Backward,
                        start_time: None,
                        end_time: None,
                        start_id: None,
                        end_id: None,
                        limit: Some(index as i64), // fetch the latest n entries
                        filter: SearchFilter::anything(self.get_history_session_id()),
                    })
                    .unwrap_or_else(|_| Vec::new())
                    .get(index.saturating_sub(1))
                    .map(|history| {
                        (
                            parsed.remainder.len(),
                            indicator.len(),
                            history.command_line.clone(),
                        )
                    }),
                ParseAction::BackwardPrefixSearch => {
                    let history_search_by_session = self
                        .history
                        .search(SearchQuery::last_with_prefix_and_cwd(
                            parsed.prefix.unwrap_or_default().to_string(),
                            self.cwd.clone().unwrap_or_else(|| {
                                std::env::current_dir()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string()
                            }),
                            self.get_history_session_id(),
                        ))
                        .unwrap_or_else(|_| Vec::new())
                        .get(index.saturating_sub(1))
                        .map(|history| {
                            (
                                parsed.remainder.len(),
                                parsed_prefix.len() + parsed_marker.len(),
                                history.command_line.clone(),
                            )
                        });
                    // If we don't find any history searching by session id, then let's
                    // search everything, otherwise use the result from the session search
                    if history_search_by_session.is_none() {
                        self.history
                            .search(SearchQuery::last_with_prefix(
                                parsed_prefix.clone(),
                                self.get_history_session_id(),
                            ))
                            .unwrap_or_else(|_| Vec::new())
                            .get(index.saturating_sub(1))
                            .map(|history| {
                                (
                                    parsed.remainder.len(),
                                    parsed_prefix.len() + parsed_marker.len(),
                                    history.command_line.clone(),
                                )
                            })
                    } else {
                        history_search_by_session
                    }
                }
                ParseAction::ForwardSearch => self
                    .history
                    .search(SearchQuery {
                        direction: SearchDirection::Forward,
                        start_time: None,
                        end_time: None,
                        start_id: None,
                        end_id: None,
                        limit: Some((index + 1) as i64), // fetch the oldest n entries
                        filter: SearchFilter::anything(self.get_history_session_id()),
                    })
                    .unwrap_or_else(|_| Vec::new())
                    .get(index)
                    .map(|history| {
                        (
                            parsed.remainder.len(),
                            indicator.len(),
                            history.command_line.clone(),
                        )
                    }),
                ParseAction::LastToken => self
                    .history
                    .search(SearchQuery::last_with_search(SearchFilter::anything(
                        self.get_history_session_id(),
                    )))
                    .unwrap_or_else(|_| Vec::new())
                    .first()
                    //BUGBUG: This returns the wrong results with paths with spaces in them
                    .and_then(|history| history.command_line.split_whitespace().next_back())
                    .map(|token| (parsed.remainder.len(), indicator.len(), token.to_string())),
            });

        if let Some((start, size, history)) = history_result {
            let edits = vec![
                EditCommand::MoveToPosition {
                    position: start,
                    select: false,
                },
                EditCommand::ReplaceChars(size, history),
            ];

            Some(ReedlineEvent::Edit(edits))
        } else {
            None
        }
    }

    fn open_editor(&mut self) -> Result<()> {
        let Some(buffer_editor) = &mut self.buffer_editor else {
            return Ok(());
        };

        let mut file = File::create(&buffer_editor.temp_file)?;
        write!(file, "{}", self.editor.get_buffer())?;
        drop(file);

        // Capture the prompt's screen range so that an editor
        // that leaves the cursor untouched (e.g. an editor that
        // uses the alternate screen only) re-uses the existing
        // prompt rows instead of starting a new prompt a row
        // below the old one.
        let suspended_state = self.painter.state_before_suspension();
        {
            // Put back the raw mode the editor found once it exits,
            // whatever mode it leaves behind. Dropped at the end of
            // this block, before the position query below.
            let _tty_mode = TtyModeGuard::save();

            let mut child = if buffer_editor.command.is_template() {
                buffer_editor
                    .build_command(self.editor.line_buffer())
                    .spawn()?
            } else {
                buffer_editor.command.0.spawn()?
            };

            // The child owns the tty now; invalidate eagerly so
            // any `?` early-return below still leaves the
            // painter in a safe state.
            self.painter.invalidate_prompt_start_row();
            child.wait()?;
        }

        // On the success path, re-initialize position and size
        // (covers a resize-during-editor with no SIGWINCH). If
        // the editor moved the cursor out of the prompt's rows
        // (it printed output), a fresh prompt starts below that
        // output. On query failure, the eager invalidate above
        // is our floor — losing the size refresh is acceptable;
        // losing the user's edited buffer below is not.
        let _ = self
            .painter
            .initialize_prompt_position(Some(&suspended_state));

        let mut res = std::fs::read_to_string(&buffer_editor.temp_file)?;
        let content_len = res.trim_end().len();
        res.truncate(content_len);

        self.editor.set_buffer(res, UndoBehavior::CreateUndoPoint);

        Ok(())
    }

    /// Repaint logic for the history reverse search
    ///
    /// Overwrites the prompt indicator and highlights the search string
    /// separately from the result buffer.
    fn history_search_paint(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let use_ansi_coloring = self.effective_ansi_coloring();
        let navigation = self.history_cursor.get_navigation();

        if let HistoryNavigationQuery::SubstringSearch(substring) = navigation {
            let status =
                if !substring.is_empty() && self.history_cursor.string_at_cursor().is_none() {
                    PromptHistorySearchStatus::Failing
                } else {
                    PromptHistorySearchStatus::Passing
                };

            // The prompt passes through raw, but the search term is typed or
            // pasted, not the host's.
            let prompt_history_search =
                PromptHistorySearch::new(status, escape_control(&substring).into_owned());

            let res_string = self.history_cursor.string_at_cursor().unwrap_or_default();

            // Highlight matches
            let res_string = if use_ansi_coloring {
                let match_highlighter = SimpleMatchHighlighter::new(substring);
                let styled = match_highlighter.highlight(&res_string, 0);
                styled.render_simple()
            } else {
                escape_control(&res_string).into_owned()
            };

            let lines = PromptLines::new(
                prompt,
                self.prompt_edit_mode(),
                Some(prompt_history_search),
                &res_string,
                "",
                "",
            );

            self.painter.repaint_buffer(
                prompt,
                &lines,
                self.prompt_edit_mode(),
                None,
                use_ansi_coloring,
                &self.cursor_shapes,
            )?;
        } else {
            // Nothing to paint, but a commit just before may have left its
            // frame open and its bytes unsent, waiting for this repaint.
            self.painter.finish_frame()?;
        }

        Ok(())
    }

    /// Triggers a full repaint including the prompt parts
    ///
    /// Includes the highlighting and hinting calls.
    fn buffer_paint(&mut self, prompt: &dyn Prompt) -> Result<()> {
        let use_ansi_coloring = self.effective_ansi_coloring();
        let cursor_position_in_buffer = self.editor.insertion_point();
        let buffer_to_paint = self.editor.get_buffer();

        let mut styled_text = self
            .highlighter
            .highlight(buffer_to_paint, cursor_position_in_buffer);
        if let Some((from, to)) = self.editor.get_selection() {
            // With a cursor-cell style configured, the head cell gets it and
            // the selection style covers the rest of the range.
            match self
                .visual_selection_cursor_style
                .zip(self.editor.selection_head_cell())
            {
                Some((cursor_style, (cell_start, cell_end))) => {
                    if from < cell_start {
                        styled_text.style_range(from, cell_start, self.visual_selection_style);
                    }
                    styled_text.style_range(cell_start, cell_end, cursor_style);
                    if cell_end < to {
                        styled_text.style_range(cell_end, to, self.visual_selection_style);
                    }
                }
                None => styled_text.style_range(from, to, self.visual_selection_style),
            }
        }

        let (before_cursor, after_cursor) = styled_text.render_around_insertion_point(
            cursor_position_in_buffer,
            prompt,
            use_ansi_coloring,
            self.painter.semantic_markers(),
        );

        // With only auto-pair closers after the cursor, as in `(gs|)`, a
        // hinter that continues the text before it hints from that text and
        // its hint is drawn in place of the closers. A hint the highlighter
        // does not see closing them would drop them on accept, so none shows.
        let trailing_closers = self.trailing_closers_for_hint().map(str::to_owned);
        let hinted_line = match trailing_closers {
            Some(_) => &buffer_to_paint[..cursor_position_in_buffer],
            None => buffer_to_paint,
        };

        // Decided from the hint this paint looked up, never a stale one: the
        // paint that submit makes with hints hidden must draw the closers.
        let mut hint_replaces_closers = false;
        // Escaped here rather than in each hinter, so a host's own hinter is
        // covered too. SGR is kept only while coloring is on: hinters style
        // their own output, and with coloring off any SGR came from history.
        let hint: String = if self.hints_active() {
            self.hinter.as_mut().map_or_else(String::new, |hinter| {
                let hint = hinter.handle(
                    hinted_line,
                    cursor_position_in_buffer,
                    self.history.as_ref(),
                    use_ansi_coloring,
                    &self.cwd.clone().unwrap_or_else(|| {
                        std::env::current_dir()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    }),
                );
                let hint = if use_ansi_coloring {
                    escape_control_keep_sgr(&hint).into_owned()
                } else {
                    escape_control(&hint).into_owned()
                };
                let hint = continue_hint_lines(
                    hint,
                    prompt,
                    use_ansi_coloring,
                    self.painter.semantic_markers(),
                );
                let Some(closers) = trailing_closers.as_deref() else {
                    return hint;
                };
                // The raw hint, not the escaped one: the closers' offsets
                // index what accepting inserts.
                let full = hinter.complete_hint();
                hint_replaces_closers = !full.is_empty()
                    && closer_offsets_for_hint(
                        self.highlighter.as_ref(),
                        buffer_to_paint,
                        cursor_position_in_buffer,
                        &full,
                        closers,
                    )
                    .is_some();
                if hint_replaces_closers {
                    hint
                } else {
                    String::new()
                }
            })
        } else {
            String::new()
        };

        // Needs to add return carriage to newlines because when not in raw mode
        // some OS don't fully return the carriage

        let lines = if hint_replaces_closers {
            // The exit path reprints what follows the cursor, which is still
            // the closers, not the hint drawn over them.
            PromptLines::new(
                prompt,
                self.prompt_edit_mode(),
                None,
                &before_cursor,
                "",
                &hint,
            )
            .with_exit_after_cursor(&after_cursor)
        } else {
            PromptLines::new(
                prompt,
                self.prompt_edit_mode(),
                None,
                &before_cursor,
                &after_cursor,
                &hint,
            )
        };
        // History search paints elsewhere, so only submit hides the status
        // line here. Measured only with one to show, since the measure walks
        // the whole buffer.
        let screen_width = self.painter.screen_width();
        let status_room = if self.status_line.is_some() && !self.hide_hints {
            // An open menu hides the hint, so measure the block the painter
            // draws above it. Active, not visible: a menu opening on this
            // paint turns visible only in its update below. One that stays
            // hidden gets its status line checked again by the painter.
            let block_rows = match self.menus.iter().find(|menu| menu.is_active()) {
                Some(menu) => lines
                    .required_lines(screen_width, false, Some(menu))
                    .saturating_sub(menu.menu_required_lines(screen_width)),
                None => lines.required_lines(screen_width, false, None),
            };
            self.painter.status_room(block_rows)
        } else {
            0
        };
        // Not asked without a row to draw it on: it gives way to a buffer
        // that fills the screen.
        let status = match self.status_line.as_mut() {
            Some(status_line) if status_room > 0 => status_line.status(
                buffer_to_paint,
                cursor_position_in_buffer,
                screen_width,
                use_ansi_coloring,
            ),
            _ => String::new(),
        };
        let mut lines = lines.with_status(&status, use_ansi_coloring);
        // Before the menus size themselves below the status line.
        self.painter
            .reserve_status_rows(lines.status_rows(screen_width), status_room);

        // Updating the working details of the active menu
        for menu in self.menus.iter_mut() {
            if menu.is_active() {
                // A menu still waiting on its first answer stays off screen, so a Tab
                // resolving to one suggestion never draws a menu it takes away again.
                if menu.is_visible() {
                    lines.prompt_indicator = menu.indicator().to_owned().into();
                }
                // If the menu requires the cursor position, update it (ide menu)
                let cursor_pos = lines.cursor_pos(screen_width);
                menu.set_cursor_pos(cursor_pos);

                menu.update_working_details(
                    &mut self.editor,
                    self.completer.as_mut(),
                    self.history.as_ref(),
                    &self.painter,
                );

                // That update is where a first answer lands and ends the opening phase,
                // so ask again: the painter picks the menu to draw below, and an
                // indicator saying otherwise would draw its rows under the ordinary
                // prompt. Reading it twice is the price of the loop: the indicator sets
                // the prompt width that positions the cursor, which the update consumes,
                // so on the frame a menu opens that width lags by one paint.
                if menu.is_visible() {
                    lines.prompt_indicator = menu.indicator().to_owned().into();
                }
            }
        }

        let menu = self.menus.iter().find(|menu| menu.is_visible());

        self.painter.repaint_buffer(
            prompt,
            &lines,
            self.prompt_edit_mode(),
            menu,
            use_ansi_coloring,
            &self.cursor_shapes,
        )?;

        if self.mouse_click_mode.is_enabled() {
            if let Some(layout) = &self.painter.last_layout {
                let buffer = self.editor.get_buffer();
                let (raw_before, raw_after) = buffer.split_at(cursor_position_in_buffer);
                self.last_render_snapshot = Some(
                    self.painter
                        .render_snapshot(&lines, menu, raw_before, raw_after, layout),
                );
            }
        } else {
            self.last_render_snapshot = None;
        }

        Ok(())
    }

    /// Adds an external printer
    pub fn with_external_printer(mut self, printer: impl ExternalOutput + 'static) -> Self {
        self.external_printer = Some(Box::new(printer));
        self
    }

    /// Sets the poll interval used when features that require periodic processing
    /// are active (e.g., external printer, idle callback).
    ///
    /// This controls how frequently Reedline yields control back to these features
    /// while waiting for user input. The default is 100ms.
    ///
    /// Common values are 33ms (~30fps) for UI updates or 100ms for less frequent tasks.
    ///
    /// Note: This setting only takes effect when an external printer or idle callback
    /// is configured. Without these features, Reedline blocks until input is received.
    ///
    /// # Example
    /// ```no_run
    /// use std::time::Duration;
    /// use reedline::Reedline;
    ///
    /// let editor = Reedline::create()
    ///     .with_poll_interval(Duration::from_millis(50));
    /// ```
    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    /// Sets an idle callback that is called periodically while waiting for user input.
    ///
    /// This is useful for applications that need to process external events
    /// (such as GUI updates, network events, or timer-based operations) while
    /// the user is typing or the editor is waiting for input.
    ///
    /// Use [`with_poll_interval`](Self::with_poll_interval) to control how frequently
    /// the callback is invoked (default: 100ms).
    ///
    /// # Example
    /// ```no_run
    /// use std::time::Duration;
    /// use reedline::Reedline;
    ///
    /// let editor = Reedline::create()
    ///     .with_poll_interval(Duration::from_millis(33))
    ///     .with_idle_callback(Box::new(|| {
    ///         // Process external events here
    ///     }));
    /// ```
    pub fn with_idle_callback(mut self, callback: Box<dyn FnMut() + Send>) -> Self {
        self.idle_callback = Some(callback);
        self
    }

    /// Offer an open menu first refusal on a keypress that would otherwise
    /// submit, and report whether it took it.
    fn menu_took_the_key(&mut self, prompt: &dyn Prompt) -> io::Result<bool> {
        Ok(matches!(
            self.handle_editor_event(prompt, ReedlineEvent::MenuAccept)?,
            EventStatus::Handled
        ))
    }

    /// Close an active menu when a typed character ends the word it was opened
    /// for. Menus without word characters, and persistent menus, live on as
    /// they always have.
    ///
    /// Every command in the batch is read, not just the first: a burst of
    /// typing arrives as one `Edit` with several `InsertChar`s, and the word
    /// can end anywhere in it. A newline ends the word too, whether it came
    /// as `InsertChar('\n')` or as `InsertNewline`, the command the default
    /// `Alt+Enter` and `Shift+Enter` bindings emit.
    fn close_a_menu_past_its_word(&mut self, commands: &[EditCommand]) {
        if self.persistent_menus {
            return;
        }
        let Some(menu) = self.menus.iter_mut().find(|menu| menu.is_active()) else {
            return;
        };
        let word_ended = commands.iter().any(|command| match command {
            EditCommand::InsertChar(c) => menu.settings().word_ends_at(*c),
            EditCommand::InsertNewline => menu.settings().word_ends_at('\n'),
            _ => false,
        });
        if word_ended {
            menu.menu_event(MenuEvent::Deactivate);
        }
    }

    fn submit_buffer(&mut self, prompt: &dyn Prompt) -> io::Result<EventStatus> {
        // A menu that let the submit through (no suggestions to accept) must
        // not stay active into the next line's editing.
        self.deactivate_menus();
        #[cfg_attr(not(feature = "system_clipboard"), allow(unused_mut))]
        let mut buffer = self.editor.get_buffer().to_string();
        // Let an installed paste interceptor replace the line at submit (e.g.
        // expand paste-reference placeholders — the compact form is only for
        // composing). Done before the repaint below so reedline itself paints
        // the expanded (possibly multi-line) buffer with correct
        // wrapping/continuation; the replacement is also what `Signal::Success`
        // returns and what history records. A host may leave non-text
        // placeholders (e.g. an image reference) intact, since a terminal
        // cannot render them. The `and_then` yields an owned `Option<String>`,
        // releasing the immutable borrow of `self.paste_interceptor` before the
        // `&mut self` `run_edit_commands`.
        #[cfg(feature = "system_clipboard")]
        if let Some(expanded) = self
            .paste_interceptor
            .as_ref()
            .and_then(|i| i.expand_on_submit(&buffer))
        {
            self.run_edit_commands(&[
                EditCommand::Clear,
                EditCommand::InsertString(expanded.clone()),
            ]);
            buffer = expanded;
        }
        self.hide_hints = true;
        // Additional repaint to show the content without hints etc.
        if let Some(transient_prompt) = self.transient_prompt.take() {
            self.repaint(transient_prompt.as_ref())?;
            self.transient_prompt = Some(transient_prompt);
        } else {
            self.repaint(prompt)?;
        }
        if !buffer.is_empty() {
            let mut entry = HistoryItem::from_command_line(&buffer);
            entry.session_id = self.get_history_session_id();

            let excluded = self
                .history_exclusion_prefix
                .as_ref()
                .is_some_and(|prefix| buffer.starts_with(prefix));

            let saved = if excluded {
                None
            } else {
                match self.history.save(entry.clone()) {
                    Ok(saved) => Some(saved),
                    Err(err) => {
                        // Ran but not stored: the excluded shape. Keep the line,
                        // stash the error for `take_history_save_error`.
                        self.history_save_error = Some(err);
                        None
                    }
                }
            };

            match saved {
                Some(saved) => {
                    self.history_last_run = saved
                        .id
                        .map_or(HistoryLastRun::Empty, HistoryLastRun::Stored)
                }
                None => {
                    self.history_last_run = HistoryLastRun::Excluded {
                        item: entry,
                        current: true,
                        recalled: false,
                    }
                }
            }
        }
        self.run_edit_commands(&[EditCommand::Clear]);
        self.editor.reset_undo_stack();

        Ok(EventStatus::Exits(Signal::Success(buffer)))
    }
}

/// Asks `highlighter` how `candidate`, a hint for the text before `cursor` in
/// `line`, closes `closers`, the text after the cursor. The answer counts only
/// if it is sound: one strictly increasing offset per closer, each at a
/// character of `candidate` that is that closer.
fn closer_offsets_for_hint(
    highlighter: &dyn Highlighter,
    line: &str,
    cursor: usize,
    candidate: &str,
    closers: &str,
) -> Option<Vec<usize>> {
    let offsets = highlighter.hint_closer_offsets(line, cursor, candidate, closers)?;
    let mut previous = None;
    let sound = offsets.len() == closers.chars().count()
        && offsets
            .iter()
            .zip(closers.chars())
            .all(|(&offset, closer)| {
                let ascending = previous.is_none_or(|previous| offset > previous);
                previous = Some(offset);
                ascending
                    && candidate.get(offset..).and_then(|rest| rest.chars().next()) == Some(closer)
            });
    sound.then_some(offsets)
}

