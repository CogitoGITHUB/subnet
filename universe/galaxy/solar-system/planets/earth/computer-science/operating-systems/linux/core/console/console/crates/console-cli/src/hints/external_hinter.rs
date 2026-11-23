use log::error;
use nu_ansi_term::Style;
use console_engine::ClosureEvalOnce;
use console_protocol::{
    PipelineData, Record, Span, Value,
    engine::{Closure, EngineState, Stack},
};
use reedline::{Hinter, History};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// The result of evaluating the external hinter closure.
///
/// Closures may return a plain string (just the hint) or a record
/// `{hint: string, next_token?: string}` when the default whitespace-based
/// token splitting is not appropriate for the hint text.
struct HintResult {
    /// The hint suffix to display after the cursor.
    hint: String,
    /// Optional override for the next token returned by `next_hint_token`.
    /// When `None`, the default `first_hint_token` splitter is used.
    next_token: Option<String>,
}

pub(crate) struct ExternalHinter {
    engine_state: Arc<EngineState>,
    stack: Arc<Stack>,
    closure: Closure,
    style: Style,
    current_hint: String,
    current_next_token: Option<String>,
}

impl ExternalHinter {
    pub(crate) fn new(
        engine_state: Arc<EngineState>,
        stack: Arc<Stack>,
        closure: Closure,
        style: Style,
    ) -> Self {
        Self {
            engine_state,
            stack,
            closure,
            style,
            current_hint: String::new(),
            current_next_token: None,
        }
    }

    fn evaluate_external_hint(
        &self,
        line: &str,
        pos: usize,
        cwd: &str,
    ) -> Result<Option<HintResult>, String> {
        // No call span available — this is a reedline hinter callback
        let span = Span::unknown();
        let context = Record::from_raw_cols_vals(
            vec!["line".to_string(), "pos".to_string(), "cwd".to_string()],
            vec![
                Value::string(line, span),
                Value::int(pos as i64, span),
                Value::string(cwd, span),
            ],
            span,
            span,
        )
        .map_err(|err| format!("failed to build context record: {err}"))?;

        let stack = Stack::with_parent(self.stack.clone());
        let result = ClosureEvalOnce::try_new(
            self.engine_state.as_ref(),
            &stack,
            self.closure.clone(),
            span,
        )
        .and_then(|eval| eval.add_arg(Value::record(context, span)))
        .and_then(|closure| closure.run_with_input(PipelineData::empty()))
        .and_then(|data| data.into_value(span));

        match result {
            Ok(Value::String { val, .. }) => Ok(Some(HintResult {
                hint: val,
                next_token: None,
            })),
            Ok(Value::Record { val, .. }) => {
                let hint = val
                    .get("hint")
                    .and_then(|v| v.as_str().ok())
                    .map(|s| s.to_string());
                let next_token = val
                    .get("next_token")
                    .and_then(|v| v.as_str().ok())
                    .map(|s| s.to_string());
                match hint {
                    Some(hint) => Ok(Some(HintResult { hint, next_token })),
                    None => {
                        error!("external_hinter: record return must contain a 'hint' string field");
                        Ok(None)
                    }
                }
            }
            Ok(Value::Nothing { .. }) => Ok(None),
            Ok(value) => {
                error!(
                    "external_hinter: unsupported closure return type {}",
                    value.get_type()
                );
                Ok(None)
            }
            Err(err) => Err(format!("closure evaluation failed: {err}")),
        }
    }
}

impl Hinter for ExternalHinter {
    fn handle(
        &mut self,
        line: &str,
        pos: usize,
        _history: &dyn History,
        use_ansi_coloring: bool,
        cwd: &str,
    ) -> String {
        match self.evaluate_external_hint(line, pos, cwd) {
            Ok(Some(result)) => {
                self.current_next_token = result.next_token;
                self.current_hint = result.hint;
            }
            Ok(None) => {
                self.current_next_token = None;
                self.current_hint = String::new();
            }
            Err(err) => {
                error!("external_hinter: {err}");
                self.current_next_token = None;
                self.current_hint = String::new();
            }
        };

        if use_ansi_coloring && !self.current_hint.is_empty() {
            self.style.paint(&self.current_hint).to_string()
        } else {
            self.current_hint.clone()
        }
    }

    fn complete_hint(&self) -> String {
        self.current_hint.clone()
    }

    fn next_hint_token(&self) -> String {
        if let Some(ref next_token) = self.current_next_token {
            next_token.clone()
        } else {
            first_hint_token(&self.current_hint)
        }
    }
}

fn first_hint_token(hint: &str) -> String {
    let mut reached_content = false;
    hint.split_word_bounds()
        .take_while(
            |word| match (word.chars().all(char::is_whitespace), reached_content) {
                (_, true) => false,
                (true, false) => true,
                (false, false) => {
                    reached_content = true;
                    true
                }
            },
        )
        .collect()
}

