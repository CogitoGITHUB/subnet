use crate::{
    completions::{
        CompletionEngine, DeclaredInputs, LegacyInputs, Returned, SpanClamp, bind_declared_inputs,
        catch_completion_panic, map_value_completions, panic_to_shell_error, report,
    },
    menus::{MenuLine, sourced_menu::SourceMode},
};
use console_engine::eval_block;
use console_protocol::{
    BlockId, IntoPipelineData, Span, Value,
    debugger::WithoutDebug,
    engine::{EngineState, Stack},
};
use reedline::{Completer, CompletionResult, Suggestion, menu_functions::parse_selection_char};
use std::sync::Arc;

const SELECTION_CHAR: char = '!';

pub struct NuMenuCompleter {
    block_id: BlockId,
    span: Span,
    stack: Stack,
    engine_state: Arc<EngineState>,
    mode: SourceMode,
    line: MenuLine,
}

impl NuMenuCompleter {
    pub fn new(
        block_id: BlockId,
        span: Span,
        stack: Stack,
        engine_state: Arc<EngineState>,
        mode: SourceMode,
        line: MenuLine,
    ) -> Self {
        Self {
            block_id,
            span,
            stack: stack.reset_out_dest().collect_value(),
            engine_state,
            mode,
            line,
        }
    }
}

impl Completer for NuMenuCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> CompletionResult {
        let parsed = parse_selection_char(line, SELECTION_CHAR);
        let handed = parsed.remainder;

        let block = self.engine_state.get_block(self.block_id).clone();
        // Reedline replacement range — never fed into `$token`/`$place`.
        let replacing = default_span(handed, pos, self.mode);
        let (buffer, cursor) = self.mode.buffer(&self.line, handed, pos, replacing.start);

        // Menu sources opt into inputs; parsed per keystroke only if declared.
        let declares_positional = !block.signature.required_positional.is_empty()
            || !block.signature.optional_positional.is_empty();
        if declares_positional {
            let wanted = DeclaredInputs::from_signature(&block.signature);
            let record = CompletionEngine::new(&self.engine_state, &self.stack)
                .completer_input_at(&buffer, cursor, wanted);
            bind_declared_inputs(
                &mut self.stack,
                &block.signature,
                record,
                LegacyInputs::menu(&block, &buffer, cursor, self.span),
            );
        }

        let input = Value::nothing(self.span).into_pipeline_data();

        let res = catch_completion_panic(|| {
            eval_block::<WithoutDebug>(&self.engine_state, &mut self.stack, &block, input)
                .map(|p| p.body)
        })
        .map_err(|payload| panic_to_shell_error(&payload, self.span))
        .and_then(|result| result);

        let suggestions = match res.and_then(|data| data.into_value(self.span)) {
            Ok(value) => convert_to_suggestions(value, replacing, &buffer[..cursor]),
            Err(err) => {
                report(format!("failed to eval menu source: {err}"));
                Vec::new()
            }
        };

        // Menu sources run synchronously, so results are always final.
        CompletionResult::fresh(suggestions)
    }
}

/// Replacement span when the source names none: what reedline feeds the completer.
fn default_span(line: &str, pos: usize, mode: SourceMode) -> reedline::Span {
    match mode {
        SourceMode::Diff => reedline::Span {
            start: pos.saturating_sub(line.len()),
            end: pos,
        },
        SourceMode::Live => reedline::Span {
            start: 0,
            end: line.len(),
        },
    }
}

/// Read a menu source's output the way a completer's is read: same accepted shapes and span
/// handling, clamped against `seen`. The `options` of the record form are dropped — reedline
/// filters and sorts its own list.
fn convert_to_suggestions(value: Value, default: reedline::Span, seen: &str) -> Vec<Suggestion> {
    // `null` declines. A menu has no next source to fall through to, so it shows nothing.
    let Some(returned) = Returned::read(value) else {
        return Vec::new();
    };

    map_value_completions(
        returned.completions.into_iter(),
        default,
        SpanClamp::within(seen),
    )
    .into_iter()
    .map(|semantic| semantic.suggestion)
    .collect()
}

