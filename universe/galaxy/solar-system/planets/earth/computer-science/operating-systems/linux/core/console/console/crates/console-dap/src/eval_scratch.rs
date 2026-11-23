//! Expression evaluation while paused — without re-entering the paused engine.
//!
//! The run engine can't be used while the eval thread sits inside a Debugger
//! callback: the callback holds that engine's debugger lock, and parsing needs
//! `&mut EngineState`, which no callback has. So watch/hover/console
//! expressions, breakpoint conditions and logpoint interpolation run against a
//! *clone* of the run engine, undebugged, with the snapshotted shadow
//! variables declared and bound on a fresh stack.
//!
//! The clone is taken right after the parse, so the script's own `def`s,
//! modules and blocks are in scope, as are the host's config, plugins and
//! `$nu`.
//!
//! Limitations of a snapshot: commands defined at runtime are invisible, and
//! `$env`/`cd` changes live on the program's `Stack` rather than its engine,
//! so they aren't reflected. Mutations here don't affect the real program, and
//! stream-valued variables show their placeholder, and the prompt commands are
//! stubbed out (see [`stub_prompts`]).

use console_protocol::ast::Block;
use console_protocol::debugger::WithoutDebug;
use console_protocol::engine::{EngineState, Stack, StateWorkingSet};
use console_protocol::{PipelineData, Signals, Span, Type, Value, VarId};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(crate) struct Scratch {
    engine: EngineState,
    /// Shadow variables already declared in `engine`. Names are only added,
    /// never removed, so a `VarId` stays valid for the session — which is what
    /// lets `blocks` below be reused.
    var_ids: HashMap<String, VarId>,
    /// Successfully parsed expressions by source text. Declaring a variable or
    /// parsing an expression merges a delta into `engine` and nothing is ever
    /// reclaimed, so without these two caches a logpoint on a hot line would
    /// grow the engine on every hit.
    ///
    /// Failures are not cached: an expression that fails to parse now may
    /// reference a variable declared later.
    blocks: HashMap<String, Arc<Block>>,
}

// --- Construction ---------------------------------------------------------

impl Scratch {
    /// Clone the run engine for scratch evaluation. Called from `engine::run`
    /// after the parse, so the script's declarations are in scope, and before
    /// `activate_debugger`, so the clone is undebugged from the start.
    pub(crate) fn from_run_engine(engine: &EngineState) -> Result<Self, String> {
        let mut engine = engine.clone();

        // A clone shares the session's `Arc`s, the debugger slot above all:
        // evaluating through a shared slot would re-enter the mutex the
        // callback is holding. This also detaches the run's jobs, repl state
        // and regex cache, so a watch expression can't disturb them.
        engine.make_session_state_unique();

        // Not the run's interrupt flag: being paused is exactly when that one
        // is likely to be raised, which would abort every scratch eval as
        // `Interrupted`. Callers install their own via `set_interrupt`.
        engine.set_signals(Signals::new(Arc::new(AtomicBool::new(false))));

        Ok(Self {
            engine: stub_prompts(engine)?,
            var_ids: HashMap::new(),
            blocks: HashMap::new(),
        })
    }

    /// The flag that aborts the next evaluation. Per caller, so a watch that is
    /// given up on cannot unwind a logpoint that happens to hold the engine.
    pub(crate) fn set_interrupt(&mut self, interrupt: &Arc<AtomicBool>) {
        self.engine.set_signals(Signals::new(interrupt.clone()));
    }
}

/// Replace the DAP prompt shims inherited from the run engine with errors.
/// They block until the client answers a `nuDapUi` event, which a watch, a
/// hover, a breakpoint condition or a logpoint has no business waiting for.
fn stub_prompts(mut engine: EngineState) -> Result<EngineState, String> {
    const REASON: &str = "scratch expressions are evaluated without a UI to prompt with";
    const HELP: &str = "prompt from the script itself, not from a watch or the debug console";

    let mut working_set = StateWorkingSet::new(&engine);
    for name in ["input", "input list", "input listen"] {
        working_set.add_decl(Box::new(crate::print_cmd::DapInputUnsupported {
            name,
            reason: REASON,
            help: HELP,
        }));
    }
    let delta = working_set.render();

    engine
        .merge_delta(delta)
        .map_err(|e| format!("stub prompts in the scratch engine: {e:?}"))?;
    Ok(engine)
}

// --- Evaluation -----------------------------------------------------------

impl Scratch {
    /// Evaluate `expr` with the given variables in scope.
    pub(crate) fn eval(&mut self, expr: &str, vars: &[(String, Value)]) -> Result<Value, String> {
        self.declare_missing(vars)?;
        let block = self.block_for(expr)?;
        let mut stack = self.bind(vars);

        let exec = console_engine::eval_block::<WithoutDebug>(
            &self.engine,
            &mut stack,
            &block,
            PipelineData::empty(),
        )
        .map_err(|e| e.to_string())?;

        exec.body
            .into_value(Span::unknown())
            .map_err(|e| e.to_string())
    }

    /// Declare any shadow variable the parser hasn't seen, so it can resolve
    /// `$name`. Free once a frame's locals are declared, which is the steady
    /// state for watch expressions and logpoints.
    fn declare_missing(&mut self, vars: &[(String, Value)]) -> Result<(), String> {
        let missing: Vec<&str> = vars
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| !self.var_ids.contains_key(*name))
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        let mut working_set = StateWorkingSet::new(&self.engine);
        // nu registers variable names with the `$` prefix.
        let declared: Vec<(String, VarId)> = missing
            .into_iter()
            .map(|name| {
                let id = working_set.add_variable(
                    format!("${name}").into_bytes(),
                    Span::unknown(),
                    Type::Any,
                    false,
                );
                (name.to_string(), id)
            })
            .collect();

        let delta = working_set.render();
        self.engine.merge_delta(delta).map_err(|e| e.to_string())?;
        self.var_ids.extend(declared);
        Ok(())
    }

    /// The parsed block for `expr`, parsing and caching it on first use.
    fn block_for(&mut self, expr: &str) -> Result<Arc<Block>, String> {
        if let Some(block) = self.blocks.get(expr) {
            return Ok(Arc::clone(block));
        }

        let mut working_set = StateWorkingSet::new(&self.engine);
        let block = console_parser::parse(&mut working_set, None, expr.as_bytes(), false);

        // Bail before merging, so a broken condition re-evaluated on every
        // hit leaves nothing behind.
        if let Some(err) = working_set.parse_errors.first() {
            return Err(err.to_string());
        }

        if let Some(err) = working_set.compile_errors.first() {
            return Err(err.to_string());
        }

        let delta = working_set.render();
        self.engine.merge_delta(delta).map_err(|e| e.to_string())?;
        self.blocks.insert(expr.to_string(), Arc::clone(&block));

        Ok(block)
    }

    /// Permanent engine state as (variables, blocks), so the tests can assert
    /// that re-evaluating an expression allocates nothing new.

    /// A fresh stack with each shadow value bound to its variable.
    fn bind(&self, vars: &[(String, Value)]) -> Stack {
        let mut stack = Stack::new();
        for (name, value) in vars {
            if let Some(id) = self.var_ids.get(name) {
                stack.add_var(*id, value.clone());
            }
        }
        stack
    }
}

// --- Interpolation --------------------------------------------------------

impl Scratch {
    /// Interpolate a logpoint message. Two syntaxes are accepted:
    /// - **Nushell** — a message that is entirely a `$"..."` / `$'...'`
    ///   literal is evaluated as-is, so `$"iteration ($i)"` works as written.
    /// - **DAP `{expression}`** — otherwise each `{expr}` segment is
    ///   substituted; unmatched braces pass through verbatim.
    pub(crate) fn interpolate(&mut self, template: &str, vars: &[(String, Value)]) -> String {
        let trimmed = template.trim();
        if is_nu_interpolation(trimmed) {
            return self.eval_to_string(trimmed, vars);
        }

        self.interpolate_braces(template, vars)
    }

    /// Substitute every `{expr}` segment; an unmatched `{` is literal text.
    fn interpolate_braces(&mut self, template: &str, vars: &[(String, Value)]) -> String {
        let mut out = String::new();
        let mut rest = template;

        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('}') {
                Some(end) => {
                    out.push_str(&self.eval_to_string(&after[..end], vars));
                    rest = &after[end + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }

        out.push_str(rest);
        out
    }

    /// Evaluate for display: strings pass through unquoted, anything else is
    /// rendered, and a failure becomes a visible placeholder — a logpoint never
    /// pauses, so a silent error would leave no sign of itself.
    fn eval_to_string(&mut self, expr: &str, vars: &[(String, Value)]) -> String {
        match self.eval(expr, vars) {
            Ok(Value::String { val, .. }) => val,
            // The session's config, not a default one: a logpoint has to
            // format a value the same way the Variables row beside it does.
            Ok(v) => v.to_expanded_string(", ", self.engine.get_config()),
            Err(e) => format!("{{error: {e}}}"),
        }
    }
}

/// Whether the whole message is a nu string-interpolation literal, to be
/// evaluated as one expression.
///
/// A heuristic on the delimiters, not the structure, so `$"a" $"b"` is misread
/// as a single literal; the fallback is a visible `{error: …}` and doing
/// better means real nu lexing.
fn is_nu_interpolation(trimmed: &str) -> bool {
    trimmed.len() > 2
        && ((trimmed.starts_with("$\"") && trimmed.ends_with('"'))
            || (trimmed.starts_with("$'") && trimmed.ends_with('\'')))
}

