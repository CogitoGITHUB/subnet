mod base;
mod default;

pub use base::PromptHelixMode;
pub use base::{
    Prompt, PromptEditMode, PromptEditModeDiscriminants, PromptHistorySearch,
    PromptHistorySearchStatus, PromptViMode,
};

pub use default::{
    DefaultPrompt, DefaultPromptSegment, DEFAULT_INSERT_PROMPT_INDICATOR,
    DEFAULT_MULTILINE_INDICATOR, DEFAULT_NORMAL_PROMPT_INDICATOR, DEFAULT_PROMPT_INDICATOR,
    DEFAULT_SELECT_PROMPT_INDICATOR,
};
