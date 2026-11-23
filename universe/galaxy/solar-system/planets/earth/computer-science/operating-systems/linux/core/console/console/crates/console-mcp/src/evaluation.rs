use crate::history::History;
use miette::{Diagnostic, SourceCode, SourceSpan};
use console_protocol::{
    FromValue, PipelineData, PipelineExecutionData, Signals, Span, Value,
    debugger::WithoutDebug,
    engine::{EngineState, Job, Jobs, Mail, Stack, StateWorkingSet, ThreadJob},
};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{Value as JsonValue, json};
use std::{
    fmt::Write,
    sync::{Arc, Mutex as SyncMutex, atomic::AtomicBool, mpsc, mpsc::Sender},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT_ENV_VAR: &str = "NU_MCP_OUTPUT_LIMIT";
const DEFAULT_OUTPUT_LIMIT: usize = 10 * 1024; // 10kib

/// Formats a miette Diagnostic error as NUON text plus a JSON mirror for MCP.
///
/// Extracts structured error information (code, message, help, labels with spans)
/// and formats it as machine-readable NUON text and JSON.
///
/// The output includes:
/// - `code`: Error code (e.g., "nu::parser::parse_mismatch")
/// - `msg`: Short error message
/// - `severity`: "error", "warning", or "advice" (if available)
/// - `help`: Hint/suggestion for fixing the error (if available)
/// - `url`: Documentation URL (if available)
/// - `labels`: List of source locations with context:
///   - `text`: What the label is pointing at (e.g., "expected duration")
///   - `span`: The exact text that caused the error
///   - `line`: 1-indexed line number
///   - `column`: 1-indexed column number
fn format_mcp_error(
    working_set: &StateWorkingSet,
    error: &dyn Diagnostic,
    default_code: Option<&'static str>,
    span: Span,
) -> (String, Option<JsonValue>) {
    let mut record = console_protocol::record! {};

    // Error code (e.g., "nu::parser::parse_mismatch")
    let code = error
        .code()
        .map(|c| c.to_string())
        .or_else(|| default_code.map(String::from));
    if let Some(code) = code {
        record.push("code", Value::string(code, span));
    }

    // Error message from Display trait
    record.push("msg", Value::string(error.to_string(), span));

    // Severity level (error, warning, advice)
    if let Some(severity) = error.severity() {
        let severity_str = match severity {
            miette::Severity::Error => "error",
            miette::Severity::Warning => "warning",
            miette::Severity::Advice => "advice",
        };
        record.push("severity", Value::string(severity_str, span));
    }

    // Help/hint text if available
    if let Some(help) = error.help() {
        record.push("help", Value::string(help.to_string(), span));
    }

    // Documentation URL if available
    if let Some(url) = error.url() {
        record.push("url", Value::string(url.to_string(), span));
    }

    // Labels with span information, line/column, and source context
    if let Some(labels) = error.labels() {
        let labels_list: Vec<Value> = labels
            .map(|label| {
                let mut label_record = console_protocol::record! {};

                // Label text/message (what it's pointing at, e.g., "expected duration")
                if let Some(text) = label.label() {
                    label_record.push("text", Value::string(text, span));
                }

                // Extract source context with line/column info
                let source_span: SourceSpan = *label.inner();
                if let Some((span_text, line, column)) =
                    extract_source_context(working_set, &source_span)
                {
                    // The exact source text at the error span
                    label_record.push("span", Value::string(span_text, span));
                    // 1-indexed line and column for human readability
                    label_record.push("line", Value::int(line as i64, span));
                    label_record.push("column", Value::int(column as i64, span));
                }

                Value::record(label_record, span)
            })
            .collect();

        if !labels_list.is_empty() {
            record.push("labels", Value::list(labels_list, span));
        }
    }

    let value = Value::record(record, span);
    let structured_content = console_json::Value::from_value(value.clone())
        .ok()
        .and_then(|value| serde_json::to_value(value).ok());
    let message = console_nuon::to_nuon(
        working_set.permanent(),
        &value,
        console_nuon::ToNuonConfig::default()
            .style(console_nuon::ToStyle::Raw)
            .span(Some(span)),
    )
    .unwrap_or_else(|_| error.to_string());

    (message, structured_content)
}

/// Extract the source code context around a span for error display.
/// Returns (span_text, line_number, column_number) where line/column are 1-indexed.
fn extract_source_context(
    working_set: &StateWorkingSet,
    span: &SourceSpan,
) -> Option<(String, usize, usize)> {
    // Use the working_set as the source code provider (it implements miette::SourceCode)
    let contents = working_set.read_span(span, 0, 0).ok()?;

    // Get the source text from the span data (it's &[u8])
    let source = contents.data();
    let span_text = if source.is_empty() {
        String::new()
    } else {
        String::from_utf8_lossy(source).into_owned()
    };

    // SpanContents provides 0-indexed line/column, convert to 1-indexed for humans
    let line = contents.line() + 1;
    let column = contents.column() + 1;

    Some((span_text, line, column))
}

/// Creates an invalid_params MCP error for user input errors (parse/compile errors).
///
/// Uses error code -32602 (Invalid params) since these are user input errors, not server errors.
/// Error text is formatted as NUON and mirrored as JSON error data.
fn user_input_error(
    working_set: &StateWorkingSet,
    error: &dyn Diagnostic,
    default_code: Option<&'static str>,
    span: Span,
) -> rmcp::ErrorData {
    let (message, data) = format_mcp_error(working_set, error, default_code, span);
    rmcp::ErrorData::invalid_params(message, data)
}

/// Creates an internal MCP error for runtime errors.
///
/// Uses error code -32603 (Internal error) since these are server-side execution errors.
/// Error text is formatted as NUON and mirrored as JSON error data.
pub(crate) fn shell_error_to_mcp_error(
    error: console_protocol::ShellError,
    engine_state: &EngineState,
    span: Span,
) -> rmcp::ErrorData {
    let working_set = StateWorkingSet::new(engine_state);
    let (message, data) = format_mcp_error(&working_set, &error, Some("nu::shell::error"), span);
    rmcp::ErrorData::internal_error(message, data)
}

/// Maximum length for job descriptions shown in `job list`.
const JOB_DESCRIPTION_MAX_LEN: usize = 40;

/// How long an evaluation can run before being auto-promoted to a background
/// job. Overridden via the `NU_MCP_PROMOTE_AFTER` env var on the persistent
/// stack.
///
/// 2 minutes is deliberate: a shorter default (we had 10s) made Claude Opus 4.7
/// give up on shell because everyday commands kept getting shoved into the
/// background. Keep the happy path synchronous; callers who know a command is
/// long-running can widen the window by setting `$env.NU_MCP_PROMOTE_AFTER`.
const DEFAULT_PROMOTE_AFTER: Duration = Duration::from_secs(120);

/// Evaluates Nushell code in a persistent REPL-style context for MCP.
///
/// # Architecture
///
/// The evaluator maintains a persistent `EngineState` and `Stack` that carry
/// state across evaluations—just like an interactive REPL session. Each evaluation:
/// 1. Parses code into a `Block` and gets a `StateDelta` via `working_set.render()`
/// 2. **Merges the delta** into the persistent engine state
/// 3. Evaluates the block with the persistent state and stack
///
/// Step 2 ensures parsed blocks (including closures) are registered and available.
///
/// # Cancellation & Job Promotion
///
/// The evaluator supports cancellation via `CancellationToken`. When cancelled:
/// 1. The evaluation is promoted to a background job (not interrupted)
/// 2. Results are delivered to the main thread's mailbox when complete
/// 3. The original state remains unchanged (forked state is not committed)
/// 4. The caller can retrieve results via `job recv`
///
/// # State Persistence
///
/// Variables, definitions, and environment changes persist across calls,
/// enabling workflows like:
/// ```nu
/// $env.MY_VAR = "hello"  # First call
/// $env.MY_VAR            # Second call returns "hello"
/// ```
///
/// # History
///
/// The evaluator maintains a `$history` list that stores all command outputs.
/// Each evaluation can access previous outputs via `$history.0`, `$history.1`, etc.
/// History is stored as a ring buffer with a configurable limit (default: 100 entries) via
/// `NU_MCP_HISTORY_LIMIT` env var. When the limit is reached, oldest entries are evicted;
/// indices don't shift, and an evicted index reads as `null`.
/// Large outputs are truncated in the response but stored in full in history.
///
/// # Concurrency
///
/// Evaluations run one at a time, like lines typed into a REPL: a call made while another
/// is running waits until that one commits its state or is promoted to a background job.
/// Otherwise two calls would fork the same state, and the one that finished last would
/// overwrite the other's history entry and variables.
pub struct Evaluator {
    state: Mutex<EvalState>,
    /// Held from fork to commit, so that evaluations don't overlap. `state` is only
    /// locked briefly, so `list_commands` and `command_help` don't wait for an evaluation.
    eval_lock: Mutex<()>,
}

/// The mutable evaluation state that persists across evaluations.
struct EvalState {
    engine_state: EngineState,
    stack: Stack,
    history: History,
}

impl EvalState {
    /// Creates a forked copy of the state for isolated evaluation.
    ///
    /// The forked state has its own `Signals` instance that can be triggered
    /// to interrupt the evaluation without affecting the original state.
    ///
    /// Returns `(forked_state, interrupt_trigger)` where `interrupt_trigger`
    /// is an `Arc<AtomicBool>` that can be set to `true` to interrupt the evaluation.
    fn fork(&self) -> (Self, Arc<AtomicBool>) {
        let interrupt = Arc::new(AtomicBool::new(false));
        let signals = Signals::new(interrupt.clone());

        let mut engine_state = self.engine_state.clone();
        engine_state.set_signals(signals);

        // Create a child stack that inherits from current state
        // We clone instead of using parent linking since we may discard entirely
        let stack = self.stack.clone();

        // Clone history so changes can be discarded
        let history = self.history.clone();

        (
            Self {
                engine_state,
                stack,
                history,
            },
            interrupt,
        )
    }
}

impl Evaluator {
    pub fn new(mut engine_state: EngineState) -> Self {
        // Disable ANSI coloring for MCP - it's a computer-to-computer protocol
        let mut config = console_protocol::Config::clone(engine_state.get_config());
        config.use_ansi_coloring = console_protocol::UseAnsiColoring::False;
        engine_state.set_config(config);

        let history = History::new(&mut engine_state);

        Self {
            state: Mutex::new(EvalState {
                engine_state,
                // Use capture_all() to capture external command stdout AND stderr
                // instead of letting them go to the process's terminal (OutDest::Inherit)
                stack: Stack::new().capture_all(),
                history,
            }),
            eval_lock: Mutex::new(()),
        }
    }

    /// Evaluates Nushell source for the MCP tool.
    ///
    /// Runs on a forked state and promotes to a background job if the request is
    /// cancelled or exceeds the promote-after timeout.
    ///
    /// The returned tool result keeps the human-readable NUON text in `content`
    /// and mirrors the same response as JSON `structuredContent` for MCP clients.
    pub async fn eval_async(&self, shell_source: &str, ct: CancellationToken) -> CallToolResult {
        match self.eval_async_output(shell_source, ct).await {
            Ok(output) => output.into_call_tool_result(),
            Err(err) => error_call_tool_result(err),
        }
    }

    async fn eval_async_output(
        &self,
        shell_source: &str,
        ct: CancellationToken,
    ) -> Result<EvalOutput, rmcp::ErrorData> {
        // Released when this call returns: after the commit below, or once the
        // evaluation is promoted to a background job (whose state is never committed).
        let _eval_guard = self.eval_lock.lock().await;

        let (forked_state, interrupt, promote_after) = {
            let state = self.state.lock().await;
            let timeout = promote_timeout(&state.engine_state, &state.stack);
            let (forked, interrupt) = state.fork();
            (forked, interrupt, timeout)
        };

        let jobs = forked_state.engine_state.jobs.clone();
        let root_job_sender = forked_state.engine_state.root_job_sender.clone();

        let source = shell_source.to_string();
        let description = job_description(&source);

        let (result_tx, mut result_rx) = oneshot::channel();

        tokio::task::spawn_blocking(move || {
            if result_tx.send(eval_inner(forked_state, &source)).is_err() {
                tracing::debug!("evaluation result receiver dropped before completion");
            }
        });

        tokio::select! {
            biased;
            _ = ct.cancelled() => {
                promote_to_background_job(result_rx, interrupt, jobs, root_job_sender, description)
            }
            result = &mut result_rx => match result {
                Ok((new_state, eval_result)) => {
                    let mut state = self.state.lock().await;
                    *state = new_state;
                    eval_result
                }
                Err(_) => Err(rmcp::ErrorData::internal_error(
                    "Evaluation task panicked".to_string(),
                    None,
                )),
            },
            _ = tokio::time::sleep(promote_after) => {
                promote_to_background_job(result_rx, interrupt, jobs, root_job_sender, description)
            }
        }
    }

    /// Synchronous evaluation without cancellation support.
    ///
    /// Provided for backwards compatibility and testing.

    pub async fn list_available_commands(&self, find: Option<String>) -> Result<String, String> {
        let state = self.state.lock().await;
        let commands: Vec<String> = state
            .engine_state
            .get_signatures_and_declids(false)
            .into_iter()
            .filter(|(sig, _)| {
                if let Some(find) = &find {
                    let find = find.to_lowercase();
                    let name = sig.name.to_lowercase();
                    let description = sig.description.to_lowercase();
                    let search_terms = sig
                        .search_terms
                        .iter()
                        .any(|term| term.to_lowercase().contains(&find));

                    name.contains(&find) || description.contains(&find) || search_terms
                } else {
                    true
                }
            })
            .map(|(sig, _)| format!("{} - {}", sig.call_signature(), sig.description))
            .collect();

        if commands.is_empty() {
            return Err("No matching commands found".to_string());
        }

        Ok(commands.join("\n"))
    }

    pub async fn command_help(&self, command_name: &str) -> Result<String, String> {
        let state = self.state.lock().await;
        let engine_state = &state.engine_state;

        // Resolve the name the way the parser does, so the help describes the command that
        // calling `command_name` runs.
        let decl_id = engine_state
            .find_decl(command_name.as_bytes(), &[])
            .ok_or_else(|| format!("command `{command_name}` not found"))?;
        let signature = engine_state.get_signature_in_scope(command_name.to_owned(), decl_id);

        Ok(format_command_help(&signature))
    }
}

fn format_command_help(signature: &console_protocol::Signature) -> String {
    let mut output = String::new();
    let usage = signature.call_signature();

    writeln!(output, "Command: {}", signature.name).ok();
    writeln!(output, "Usage: {}", usage).ok();

    if !signature.description.is_empty() {
        writeln!(output, "\nDescription: {}", signature.description).ok();
    }

    if !signature.extra_description.is_empty() {
        writeln!(output, "\nDetails: {}", signature.extra_description).ok();
    }

    if !signature.search_terms.is_empty() {
        writeln!(
            output,
            "\nSearch terms: {}",
            signature.search_terms.join(", ")
        )
        .ok();
    }

    if !signature.named.is_empty() {
        writeln!(output, "\nFlags:").ok();
        for flag in &signature.named {
            let short = flag.short.map(|s| format!("-{}, ", s)).unwrap_or_default();
            let arg = flag
                .arg
                .as_ref()
                .map(|shape| format!(" <{shape}>", shape = shape))
                .unwrap_or_default();
            writeln!(output, "  {}--{}{}: {}", short, flag.long, arg, flag.desc).ok();
        }
    }

    if !signature.required_positional.is_empty()
        || !signature.optional_positional.is_empty()
        || signature.rest_positional.is_some()
    {
        writeln!(output, "\nParameters:").ok();

        for positional in &signature.required_positional {
            writeln!(output, "  {}: {}", positional.name, positional.desc).ok();
        }

        for positional in &signature.optional_positional {
            writeln!(output, "  [{}]: {}", positional.name, positional.desc).ok();
        }

        if let Some(rest) = &signature.rest_positional {
            writeln!(output, "  ...{}: {}", rest.name, rest.desc).ok();
        }
    }

    output.trim_end().to_string()
}

/// Result of a successful evaluation, containing both the MCP response
/// (which may truncate large output with a `$history` reference) and the
/// full untruncated output for use by promoted background jobs.
#[derive(Debug)]
struct EvalOutput {
    /// The formatted MCP response (NUON record with cwd, history_index, output/note).
    response: String,
    /// JSON mirror of `response` for MCP clients that support structured output.
    structured_content: JsonValue,
    /// The full output NUON string, never truncated.
    full_output: String,
}

impl EvalOutput {
    fn into_call_tool_result(self) -> CallToolResult {
        call_tool_result(self.response, self.structured_content, false)
    }
}

fn call_tool_result(text: String, structured_content: JsonValue, is_error: bool) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
    result.is_error = Some(is_error);
    result.structured_content = Some(structured_content);
    result
}

fn error_call_tool_result(err: rmcp::ErrorData) -> CallToolResult {
    let message = err.message.to_string();
    let structured_content = err
        .data
        .unwrap_or_else(|| json!({ "msg": message.clone() }));

    call_tool_result(message, structured_content, true)
}

/// Registers a [`ThreadJob`] for a still-running evaluation and spawns a task
/// that delivers results to the main thread's mailbox (job 0) on completion.
/// Shares the forked evaluation's interrupt signal so `job kill` works.
///
/// The promoted path sends the **full** (non-truncated) output through the
/// mailbox so `job recv` returns usable data instead of a dangling `$history`
/// reference.
fn promote_to_background_job(
    result_rx: oneshot::Receiver<(EvalState, Result<EvalOutput, rmcp::ErrorData>)>,
    interrupt: Arc<AtomicBool>,
    jobs: Arc<SyncMutex<Jobs>>,
    root_job_sender: Sender<Mail>,
    description: String,
) -> Result<EvalOutput, rmcp::ErrorData> {
    let signals = Signals::new(interrupt);
    let (sender, _receiver) = mpsc::channel();
    let auto_kill = false;
    let thread_job = ThreadJob::new(signals, Some(description), auto_kill, sender);

    let job_id = match jobs.lock() {
        Ok(mut jobs) => jobs.add_job(Job::Thread(thread_job)),
        Err(err) => {
            return Err(rmcp::ErrorData::internal_error(
                format!("failed to register background job: jobs lock poisoned ({err})"),
                None,
            ));
        }
    };

    tokio::spawn(async move {
        let output = match result_rx.await {
            Ok((_state, Ok(eval_output))) => eval_output.full_output,
            Ok((_state, Err(err))) => format!("Error: {}", err.message),
            Err(_) => "Evaluation task panicked".to_string(),
        };

        let value = Value::string(output, Span::unknown());
        if root_job_sender
            .send((None, PipelineData::value(value, None)))
            .is_err()
        {
            tracing::warn!("failed to send background job output to root mailbox");
        }

        match jobs.lock() {
            Ok(mut jobs) => {
                jobs.remove_job(job_id);
            }
            Err(err) => {
                tracing::warn!("failed to remove finished background job: {err}");
            }
        }
    });

    Err(rmcp::ErrorData::internal_error(
        format!(
            "Operation promoted to background job (id: {}). \
             Use `job list` to see it and `job recv` to get the result.",
            job_id.get()
        ),
        None,
    ))
}

/// Creates a short description for display in `job list`.
fn job_description(source: &str) -> String {
    let first_line = source.lines().next().unwrap_or(source);
    if first_line.len() <= JOB_DESCRIPTION_MAX_LEN {
        format!("mcp: {first_line}")
    } else {
        let truncated: String = first_line.chars().take(JOB_DESCRIPTION_MAX_LEN).collect();
        format!("mcp: {truncated}...")
    }
}

/// Inner evaluation logic that operates on an owned `EvalState`.
///
/// Returns the (possibly modified) state along with the result.
/// This allows the caller to decide whether to commit or discard the state.
fn eval_inner(
    mut state: EvalState,
    shell_source: &str,
) -> (EvalState, Result<EvalOutput, rmcp::ErrorData>) {
    let EvalState {
        engine_state,
        stack,
        history,
    } = &mut state;

    let result = eval_on_state(engine_state, stack, history, shell_source);
    (state, result)
}

/// Core evaluation logic shared by both sync and async paths.
fn eval_on_state(
    engine_state: &mut EngineState,
    stack: &mut Stack,
    history: &mut History,
    shell_source: &str,
) -> Result<EvalOutput, rmcp::ErrorData> {
    let (block, delta) = {
        let mut working_set = StateWorkingSet::new(engine_state);
        let block = console_parser::parse(&mut working_set, None, shell_source.as_bytes(), false);

        if let Some(err) = working_set.parse_errors.first() {
            let span = block.span.unwrap_or(Span::unknown());
            return Err(user_input_error(&working_set, err, None, span));
        }

        if let Some(err) = working_set.compile_errors.first() {
            let span = block.span.unwrap_or(Span::unknown());
            return Err(user_input_error(&working_set, err, None, span));
        }

        (block, working_set.render())
    };

    let block_span = block.span.unwrap_or(Span::unknown());

    engine_state
        .merge_delta(delta)
        .map_err(|e| shell_error_to_mcp_error(e, engine_state, block_span))?;

    // Set up $history variable on the stack before evaluation
    if let Some(history_var_id) = history.var_id() {
        stack.add_var(history_var_id, history.as_value());
    }

    let output =
        console_engine::eval_block::<WithoutDebug>(engine_state, stack, &block, PipelineData::empty())
            .map_err(|e| shell_error_to_mcp_error(e, engine_state, block_span))?;

    let cwd = engine_state
        .cwd(Some(stack))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| String::from("unknown"));

    let (output_value, output_nuon) = process_pipeline(output, engine_state, block_span)?;

    // Create timestamp for response
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let timestamp_value = chrono::DateTime::from_timestamp_nanos(timestamp).fixed_offset();

    // Store in history
    let history_index = history.push(output_value.clone(), engine_state, stack);

    let truncated =
        output_limit(engine_state, stack).is_some_and(|limit| output_nuon.len() > limit);

    let mut response_record = console_protocol::record! {
        "cwd" => Value::string(cwd, block_span),
        "history_index" => Value::int(history_index as i64, block_span),
        "timestamp" => Value::date(timestamp_value, block_span),
    };

    if truncated {
        let note = format!("output truncated, full result in $history.{history_index}");
        response_record.push("note", Value::string(note, block_span));
    } else {
        response_record.push("output", output_value);
    }

    let response_value = Value::record(response_record, block_span);
    let nuon_config = console_nuon::ToNuonConfig::default()
        .style(console_nuon::ToStyle::Raw)
        .raw_strings(true)
        .span(Some(block_span));

    let response = console_nuon::to_nuon(engine_state, &response_value, nuon_config)
        .map_err(|e| shell_error_to_mcp_error(e, engine_state, block_span))?;
    let structured_content = structured_json_from_value(response_value, engine_state, block_span)?;

    Ok(EvalOutput {
        response,
        structured_content,
        full_output: output_nuon,
    })
}

fn structured_json_from_value(
    value: Value,
    engine_state: &EngineState,
    span: Span,
) -> Result<JsonValue, rmcp::ErrorData> {
    let value = console_json::Value::from_value_serialized(value, engine_state)
        .map_err(|e| shell_error_to_mcp_error(e, engine_state, span))?;

    serde_json::to_value(value).map_err(|err| {
        rmcp::ErrorData::internal_error(
            format!("failed to serialize structured MCP content: {err}"),
            None,
        )
    })
}

/// Returns the duration after which a running evaluation is auto-promoted
/// to a background job.
///
/// Defaults to [`DEFAULT_PROMOTE_AFTER`] (2 minutes). Can be overridden via
/// `NU_MCP_PROMOTE_AFTER` env var on the persistent stack.
fn promote_timeout(engine_state: &EngineState, stack: &Stack) -> Duration {
    stack
        .get_env_var(engine_state, "NU_MCP_PROMOTE_AFTER")
        .and_then(|v| v.as_duration().ok())
        .and_then(|nanos| u64::try_from(nanos).ok())
        .map(Duration::from_nanos)
        .unwrap_or(DEFAULT_PROMOTE_AFTER)
}

/// Returns the output limit in bytes.
///
/// Defaults to 10kib. Can be overridden via `NU_MCP_OUTPUT_LIMIT` env var.
/// Set to `0` to disable truncation entirely.
fn output_limit(engine_state: &EngineState, stack: &Stack) -> Option<usize> {
    let limit = stack
        .get_env_var(engine_state, OUTPUT_LIMIT_ENV_VAR)
        .and_then(|v| v.as_filesize().ok())
        .and_then(|fs| usize::try_from(fs.get()).ok())
        .unwrap_or(DEFAULT_OUTPUT_LIMIT);

    if limit == 0 { None } else { Some(limit) }
}

fn process_pipeline(
    pipeline_execution_data: PipelineExecutionData,
    engine_state: &EngineState,
    call_span: Span,
) -> Result<(Value, String), rmcp::ErrorData> {
    let span = pipeline_execution_data.span().unwrap_or(call_span);

    if let PipelineData::ByteStream(stream, ..) = pipeline_execution_data.body {
        // Try to handle as a child process first (external commands)
        // This properly handles both stdout and stderr when capture_all() is used
        match stream.into_child() {
            Ok(mut child) => {
                // `process_pipeline` reports the captured output itself. Mark the child
                // exit status as handled so `pipefail` does not re-raise a
                // `non_zero_exit_code` after we already collected stdout/stderr.
                child.ignore_error(true);

                let output = child
                    .wait_with_output()
                    .map_err(|e| shell_error_to_mcp_error(e, engine_state, call_span))?;

                // Combine stdout and stderr into a single output
                let mut combined = Vec::new();
                if let Some(stdout) = output.stdout {
                    combined.extend(stdout);
                }
                if let Some(stderr) = output.stderr {
                    if !combined.is_empty() && !stderr.is_empty() {
                        combined.push(b'\n');
                    }
                    combined.extend(stderr);
                }

                let string_output = String::from_utf8_lossy(&combined).into_owned();
                let value = Value::string(&string_output, span);
                return Ok((value, string_output));
            }
            Err(stream) => {
                // Not a child process (e.g., Read or File source), use write_to
                let mut buffer = Vec::new();
                stream
                    .write_to(&mut buffer)
                    .map_err(|e| shell_error_to_mcp_error(e, engine_state, call_span))?;
                let string_output = String::from_utf8_lossy(&buffer).into_owned();
                let value = Value::string(&string_output, span);
                return Ok((value, string_output));
            }
        }
    }

    let mut values = Vec::new();
    for item in pipeline_execution_data.body {
        if let Value::Error { error, .. } = &item {
            return Err(shell_error_to_mcp_error(
                *error.clone(),
                engine_state,
                call_span,
            ));
        }
        values.push(item);
    }

    let value_to_store = match values.len() {
        1 => values.pop().unwrap_or_else(|| Value::nothing(span)),
        _ => Value::list(values, span),
    };

    let nuon_string = console_nuon::to_nuon(
        engine_state,
        &value_to_store,
        console_nuon::ToNuonConfig::default()
            .style(console_nuon::ToStyle::Raw)
            .span(Some(call_span)),
    )
    .map_err(|e| shell_error_to_mcp_error(e, engine_state, call_span))?;

    Ok((value_to_store, nuon_string))
}

