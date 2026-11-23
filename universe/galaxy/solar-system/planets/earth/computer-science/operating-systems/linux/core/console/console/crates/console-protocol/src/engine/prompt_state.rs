use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::JobId;

/// Wakes the active interactive line editor and asks it to repaint the prompt in
/// place, without disturbing the line currently being edited.
type Repainter = Arc<dyn Fn() + Send + Sync>;

/// Which prompt segment an asynchronous [`PromptState::set`] targets.
#[derive(Debug, Clone, Copy)]
pub enum PromptSegment {
    /// The main (left) prompt, `$env.PROMPT_COMMAND`.
    Left,

    /// The right prompt, `$env.PROMPT_COMMAND_RIGHT`.
    Right,

    /// The prompt indicator for the default/emacs edit mode, `$env.PROMPT_INDICATOR`.
    Indicator,

    /// The vi insert-mode indicator, `$env.PROMPT_INDICATOR_VI_INSERT`.
    ViInsert,

    /// The vi normal-mode indicator, `$env.PROMPT_INDICATOR_VI_NORMAL`.
    ViNormal,

    /// The multiline continuation indicator, `$env.PROMPT_MULTILINE_INDICATOR`.
    Multiline,
}

/// The full set of rendered prompt strings, as the line editor draws them.
#[derive(Debug, Default, Clone)]
pub struct PromptContents {
    pub left: Option<Arc<str>>,
    pub right: Option<Arc<str>>,
    pub indicator: Option<Arc<str>>,
    pub vi_insert: Option<Arc<str>>,
    pub vi_normal: Option<Arc<str>>,
    pub multiline: Option<Arc<str>>,
    pub render_right_on_last_line: bool,
}

impl PromptContents {
    /// Applies an overriding string to a specific segment.
    pub fn apply_segment_override(&mut self, segment: PromptSegment, content: impl Into<Arc<str>>) {
        let content = content.into();
        match segment {
            PromptSegment::Left => self.left = Some(content),
            PromptSegment::Right => self.right = Some(content),
            PromptSegment::Indicator => self.indicator = Some(content),
            PromptSegment::ViInsert => self.vi_insert = Some(content),
            PromptSegment::ViNormal => self.vi_normal = Some(content),
            PromptSegment::Multiline => self.multiline = Some(content),
        }
    }

    /// Layer `overrides` on top of `self`, preferring the override where set
    /// and falling back to `self` otherwise. `render_right_on_last_line` always
    /// comes from `self`, so a push never changes it.
    ///
    /// This lays pushes over the live contents and over the new baseline, and
    /// renders the transient prompt from the *live* baseline (so late pushes
    /// still show up) with `TRANSIENT_PROMPT_*` values layered on top.
    pub fn overridden_by(&self, overrides: &PromptContents) -> PromptContents {
        PromptContents {
            left: overrides.left.clone().or_else(|| self.left.clone()),
            right: overrides.right.clone().or_else(|| self.right.clone()),
            indicator: overrides
                .indicator
                .clone()
                .or_else(|| self.indicator.clone()),
            vi_insert: overrides
                .vi_insert
                .clone()
                .or_else(|| self.vi_insert.clone()),
            vi_normal: overrides
                .vi_normal
                .clone()
                .or_else(|| self.vi_normal.clone()),
            multiline: overrides
                .multiline
                .clone()
                .or_else(|| self.multiline.clone()),
            render_right_on_last_line: self.render_right_on_last_line,
        }
    }
}

/// The rendered prompt, plus the pushes kept while the REPL rebuilds it.
#[derive(Debug, Default)]
struct PromptData {
    contents: PromptContents,

    /// The pushes kept since [`PromptState::begin_update`]. `None` outside the window between
    /// that call and [`PromptState::set_contents`], which layers them over the new baseline.
    /// Without it, a job that finishes before the REPL rebuilds the prompt would lose its push.
    pending: Option<PendingPushes>,
}

/// The segments pushed for the prompt the REPL is rebuilding.
#[derive(Debug)]
struct PendingPushes {
    /// The id of the first job the main thread started for this prompt. A job whose root is
    /// lower is left over from an earlier prompt, so its result may be out of date (computed for
    /// another directory, say).
    first_job: JobId,

    pushed: PromptContents,
}

impl PendingPushes {
    /// Whether a push belongs to the prompt being rebuilt (see [`PromptState::begin_update`]).
    fn accepts(&self, root_job: JobId) -> bool {
        root_job == JobId::ZERO || root_job >= self.first_job
    }
}

/// Shared, thread-safe home for the interactive prompt's rendered content.
#[derive(derive_more::Debug, Default)]
pub struct PromptState {
    /// Upgraded to RwLock: Enables infinite concurrent reads, locking only for mutations.
    /// The pending pushes share the lock so a push can't slip between them and the contents.
    data: RwLock<PromptData>,

    /// Kept in its own lock: it is installed/cleared by the REPL and read by
    /// [`PromptState::apply`], never together with `data`.
    #[debug(skip)]
    repainter: Mutex<Option<Repainter>>,
}

impl PromptState {
    pub fn new() -> Self {
        Self::default()
    }

    fn acquire_read_lock(&self) -> RwLockReadGuard<'_, PromptData> {
        self.data
            .read()
            .unwrap_or_else(|poisoned_error| poisoned_error.into_inner())
    }

    fn acquire_write_lock(&self) -> RwLockWriteGuard<'_, PromptData> {
        self.data
            .write()
            .unwrap_or_else(|poisoned_error| poisoned_error.into_inner())
    }

    fn acquire_repainter_lock(&self) -> MutexGuard<'_, Option<Repainter>> {
        self.repainter
            .lock()
            .unwrap_or_else(|poisoned_error| poisoned_error.into_inner())
    }

    /// Run an action with shared, read-only access to the current contents.
    pub fn with_contents<ReturnType>(
        &self,
        action: impl FnOnce(&PromptContents) -> ReturnType,
    ) -> ReturnType {
        action(&self.acquire_read_lock().contents)
    }

    /// A snapshot of the current contents.
    pub fn contents(&self) -> PromptContents {
        self.with_contents(PromptContents::clone)
    }

    /// Start rebuilding the prompt. Until [`Self::set_contents`], a push is also kept aside, so the
    /// new baseline doesn't overwrite it, when it comes from the main thread or from a job whose
    /// `CurrentJob::root` is at least `first_job`: a job the main thread started after this call,
    /// or a job one of those spawned. A push from an older job still shows until the new baseline
    /// replaces it.
    ///
    /// The REPL calls this before the hooks that run ahead of each prompt, and passes the id the
    /// next spawned job will get. Until `set_contents`, everything the main thread runs prepares
    /// this prompt: the `env_change` and `pre_prompt` hooks, `color_config` and `ENV_CONVERSIONS`
    /// closures, and the prompt closures themselves.
    pub fn begin_update(&self, first_job: JobId) {
        self.acquire_write_lock().pending = Some(PendingPushes {
            first_job,
            pushed: PromptContents::default(),
        });
    }

    /// Replace all prompt content (the baseline), then lay over it the pushes kept since
    /// [`Self::begin_update`].
    pub fn set_contents(&self, new_contents: PromptContents) {
        let mut data = self.acquire_write_lock();
        data.contents = match data.pending.take() {
            Some(pending) => new_contents.overridden_by(&pending.pushed),
            None => new_contents,
        };
    }

    /// Replace the segments set in `pushed` under a single write lock, then request one in-place
    /// repaint. Callers touching several segments at once should use this as to not flash-update.
    ///
    /// `root_job` is the pushing thread's `CurrentJob::root` (`JobId::ZERO` on the main thread);
    /// it decides whether the push outlives the next [`Self::set_contents`] (see
    /// [`Self::begin_update`]). Segments left `None` keep their current value, and
    /// `render_right_on_last_line` is ignored.
    pub fn apply(&self, pushed: PromptContents, root_job: JobId) {
        let mut data = self.acquire_write_lock();
        data.contents = data.contents.overridden_by(&pushed);
        if let Some(pending) = &mut data.pending
            && pending.accepts(root_job)
        {
            pending.pushed = pending.pushed.overridden_by(&pushed);
        }
        // Release the lock before repainting: the line editor reads the prompt to redraw it.
        drop(data);
        self.request_repaint();
    }

    /// Push an override for a single segment and request an in-place repaint. `root_job` is as
    /// in [`Self::apply`].
    pub fn set(&self, segment: PromptSegment, content: impl Into<Arc<str>>, root_job: JobId) {
        let mut pushed = PromptContents::default();
        pushed.apply_segment_override(segment, content);
        self.apply(pushed, root_job);
    }

    /// Install or remove the line editor's repainter mechanism.
    pub fn set_repainter(&self, new_repainter: Option<Repainter>) {
        *self.acquire_repainter_lock() = new_repainter;
    }

    /// Fire the installed repainter, explicitly dropping the lock before executing.
    fn request_repaint(&self) {
        // Cloning the Option<Arc> locally ensures the MutexGuard drops immediately
        // at the end of this statement, keeping lock contention to a minimum.
        let local_repainter = self.acquire_repainter_lock().clone();

        if let Some(repainter) = local_repainter {
            repainter();
        }
    }
}

