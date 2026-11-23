//! Converts nu `Value`s into the DAP variable tree stored in a
//! `PauseSnapshot` arena. Records/lists/tables become expandable nodes;
//! everything else renders as a leaf string.

use crate::dap::types::Variable;
use crate::state::{PauseSnapshot, RenderCache, VarNode};
use console_protocol::engine::EngineState;
use console_protocol::{ByteStreamSource, Config, PipelineData, Type, Value};
use serde_json::{Value as J, json};
use std::fmt::Write;

/// Max children materialized per list/record level to keep responses bounded.
const MAX_CHILDREN: usize = 200;
/// Levels materialized eagerly at pause time; deeper levels hydrate on demand
/// (`materialize_children` when the client expands a node), so no depth limit.
const EAGER_DEPTH: usize = 1;
/// Cap on any single rendered row, so one hostile value can't flood the pane.
const MAX_ROW_CHARS: usize = 120;
/// Cap on a closure label specifically, leaving room for the capture suffix.
const MAX_CLOSURE_CHARS: usize = 60;

/// Everything a render needs that does not live in the `Value` itself.
///
/// Both fields are resolved on the eval thread and carried in the snapshot:
/// the server thread answers `variables` without an `EngineState` (see the
/// concurrency rule in `state.rs`), so engine-derived facts arrive
/// pre-computed.
#[derive(Clone, Copy)]
pub(crate) struct RenderCtx<'a> {
    pub config: &'a Config,
    pub cache: &'a RenderCache,
}

/// Resolve what rendering needs from the engine: each block's source text
/// (for closure rows) and the names of the variables blocks capture (for the
/// rows under an expanded closure).
///
/// Called once after the parse, after which blocks are fixed (`source` is a
/// parse-time keyword). Anything missing degrades: an unknown block to
/// `<closure>`, an unknown var to `var{id}`.
pub(crate) fn collect_render_cache(engine_state: &EngineState) -> RenderCache {
    let mut cache = RenderCache::default();
    for id in 0..engine_state.num_blocks() {
        let block = engine_state.get_block(console_protocol::BlockId::new(id));

        // From the block's captures, not every variable in the program, so
        // the map stays small.
        for (var_id, _) in &block.captures {
            cache
                .var_names
                .entry(var_id.get())
                .or_insert_with(|| crate::debugger::stepping::var_name(engine_state, *var_id));
        }

        let Some(span) = block.span else { continue };
        let text = String::from_utf8_lossy(engine_state.get_span_contents(span));
        // A row is one line, so a multi-line closure body is flattened.
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.is_empty() {
            continue;
        }
        cache.closure_src.insert(id, cap(&flat, MAX_CLOSURE_CHARS));
    }
    cache
}

/// Render `value` as the single line shown in the Variables pane.
///
/// Container shapes and every type not special-cased below come from
/// [`Value::to_abbreviated_string`], the same rendering `table` applies per
/// cell, so values read as they do elsewhere in shell. What stays local is
/// what a debugger needs and a pipeline does not: bounded rows, previews
/// rather than bare shapes, literal dates, a visible `null`, and binary as a
/// nu literal. `docs/value-rendering.md` compares every candidate renderer
/// variant by variant and records why none of them is used wholesale.
pub(crate) fn short_render(value: &Value, ctx: RenderCtx<'_>) -> String {
    let config = ctx.config;
    match value {
        Value::String { val, .. } => {
            // chars(), not byte slicing: a byte index can land mid-codepoint
            // and panic, and a panic in a callback hangs the session.
            let n_chars = val.chars().count();
            if n_chars > MAX_ROW_CHARS {
                let head: String = val.chars().take(MAX_ROW_CHARS - 3).collect();
                format!("\"{head}…\" ({n_chars} chars)")
            } else {
                format!("\"{val}\"")
            }
        }
        Value::Nothing { .. } => "null".into(),
        Value::Date { val, .. } => val.to_rfc3339(),
        // A table's rows have no useful one-line preview: keep the shape.
        Value::List { .. } if matches!(value.get_type(), Type::Table(_)) => {
            value.to_abbreviated_string(config)
        }
        Value::List { vals, .. } => {
            // Preview the first few scalar elements: [1, 2, 3, …]
            let mut preview = String::from("[");
            for (i, v) in vals.iter().take(3).enumerate() {
                if i > 0 {
                    preview.push_str(", ");
                }
                preview.push_str(&scalar_preview(v));
            }
            if vals.len() > 3 {
                preview.push_str(", …");
            }
            preview.push(']');
            if preview.len() > 60 {
                // Too wide to preview: `[list N items]`, from the shell.
                value.to_abbreviated_string(config)
            } else {
                preview
            }
        }
        Value::Record { val, .. } => {
            // Preview the first few fields: {name: a.txt, size: 120, …}
            let mut preview = String::from("{");
            for (i, (k, v)) in val.iter().take(3).enumerate() {
                if i > 0 {
                    preview.push_str(", ");
                }
                preview.push_str(k);
                preview.push_str(": ");
                preview.push_str(&scalar_preview(v));
            }
            if val.len() > 3 {
                preview.push_str(", …");
            }
            preview.push('}');
            if preview.len() > 60 {
                // Ditto: `{record N fields}`.
                value.to_abbreviated_string(config)
            } else {
                preview
            }
        }
        Value::Closure { val, .. } => closure_label(val, ctx.cache),
        Value::Binary { val, .. } => {
            // First bytes as hex, shell-literal style: 0x[de ad be ef …] (N
            // bytes). `to nuon` writes the same form unspaced and unbounded;
            // the spacing and byte count are what a row needs.
            let mut s = String::from("0x[");
            for (i, b) in val.iter().take(8).enumerate() {
                if i > 0 {
                    s.push(' ');
                }
                let _ = write!(s, "{b:02x}");
            }
            if val.len() > 8 {
                s.push_str(" …");
            }
            let _ = write!(s, "] ({} bytes)", val.len());
            s
        }
        // Upstream renders errors as the whole multi-line `ShellError` Debug;
        // one line of the message is what fits a row.
        Value::Error { error, .. } => cap_row(&format!("<error: {error}>")),
        // Everything else (int, float, bool, filesize, duration, range, glob,
        // cell-path, custom) renders the way the shell renders it. Capped
        // because a custom value's base form can expand without bound.
        other => cap_row(&other.to_abbreviated_string(config)),
    }
}

/// A closure as `{|x| $x * 2}` plus the number of variables it captured, which
/// is usually what you want to know about a closure you are stopped inside.
///
/// The literal comes from the render cache; a block id missing from it (a
/// stale snapshot) degrades to a bare `<closure>`.
fn closure_label(val: &console_protocol::engine::Closure, cache: &RenderCache) -> String {
    let body = cache
        .closure_src
        .get(&val.block_id.get())
        .cloned()
        .unwrap_or_else(|| "<closure>".to_string());
    match val.captures.len() {
        0 => body,
        1 => format!("{body} +1 capture"),
        n => format!("{body} +{n} captures"),
    }
}

/// Truncate a rendered row to [`MAX_ROW_CHARS`].
fn cap_row(s: &str) -> String {
    cap(s, MAX_ROW_CHARS)
}

/// Truncate to `max` chars on a char boundary: a byte index can land
/// mid-codepoint and panic, and a panic in a callback hangs the session.
fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{head}…")
}

/// Describe a stream without consuming it. Draining would eat the program's
/// own data, and pulling even one element would re-enter the evaluator while
/// it holds the `EngineState.debugger` mutex, so everything here is *static*:
/// kind, origin, size, the producing command, and the pipeline metadata. The
/// element type is therefore absent — nothing short of pulling can know it.
///
/// Wording follows `describe --no-collect`, so both name a stream alike.
/// `describe`'s own code takes `PipelineData` by value, while a paused
/// debugger only borrows the register it is looking at.
pub(crate) fn describe_stream(data: &PipelineData, engine_state: &EngineState) -> String {
    let (kind, origin, size, span, meta) = match data {
        PipelineData::ByteStream(bs, meta) => {
            // Same call `describe` makes: "binary (stream)" / "string (stream)"
            // / "byte stream".
            let kind = bs.type_().describe().to_string();
            // Origin names match the `origin` field of `describe --detailed`.
            let origin = match bs.source() {
                ByteStreamSource::Read(_) => "unknown",
                ByteStreamSource::File(_) => "file",
                ByteStreamSource::Child(_) => "external",
            };
            // Size is ours: `describe` never reports it, but it costs nothing.
            (kind, Some(origin), bs.known_size(), Some(bs.span()), meta)
        }
        // `describe --detailed` calls this a "list stream".
        PipelineData::ListStream(ls, meta) => {
            ("list stream".to_string(), None, None, Some(ls.span()), meta)
        }
        _ => ("stream".to_string(), None, None, None, &None),
    };

    // The stream's span is the producing command — which list stream, not
    // just "a list stream". It subsumes the origin (`open` means a file,
    // `^cmd` an external), so the origin is only a fallback.
    let producer = span.and_then(|s| command_word(engine_state, s));
    let mut parts = match (&producer, origin) {
        (Some(p), _) => vec![format!("{kind} from `{p}`")],
        (None, Some(o)) => vec![format!("{kind} from {o}")],
        (None, None) => vec![kind],
    };

    if let Some(meta) = meta {
        // Set by `open`: the file the bytes are coming from.
        if let console_protocol::DataSource::FilePath(path) = &meta.data_source {
            parts.push(
                path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy().to_string()),
            );
        }
        // Sniffed by `open` from the extension: `text/x-toml`, `application/json`.
        if let Some(ct) = &meta.content_type {
            parts.push(ct.clone());
        }
    }

    if let Some(n) = size {
        parts.push(format!("{n} bytes"));
    }

    format!("<{}>", parts.join(", "))
}

/// The producing command's name, when the span actually points at one.
///
/// A stream's span is not always a command: `"a-b" | split row "-" | get 0`
/// leaves it pointing at the *literal* `"a-b"`, and `from `"a-b"`` is worse
/// than silence. So only an identifier, optionally `^`-prefixed for an
/// external, is accepted.
fn command_word(engine_state: &EngineState, span: console_protocol::Span) -> Option<String> {
    let raw = String::from_utf8_lossy(engine_state.get_span_contents(span));
    let word = raw.split_whitespace().next()?;
    let name = word.strip_prefix('^').unwrap_or(word);
    let looks_like_a_command = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '\\'));
    looks_like_a_command.then(|| word.to_string())
}

/// One-token rendering used inside list/record previews. Lossier than a full
/// row, since several share one 60-char line, but never *blank*: a field that
/// rendered to nothing would read as missing rather than empty.
fn scalar_preview(value: &Value) -> String {
    match value {
        // Bare, unlike a row: quotes on every element would eat the line.
        // Except the empty string, which would read `{name: , size: 120}`.
        Value::String { val, .. } if val.is_empty() => "\"\"".into(),
        Value::String { val, .. } => {
            let mut s: String = val.chars().take(12).collect();
            if val.chars().count() > 12 {
                s.push('…');
            }
            s
        }
        Value::Int { val, .. } => val.to_string(),
        // `ObviousFloat`, as the shell does it: `1.0`, not `f64`'s bare `1`.
        Value::Float { val, .. } => console_utils::ObviousFloat(*val).to_string(),
        Value::Bool { val, .. } => val.to_string(),
        Value::Nothing { .. } => "null".into(),
        Value::List { vals, .. } => format!("[{}]", vals.len()),
        Value::Record { val, .. } => format!("{{{}}}", val.len()),
        _ => "…".into(),
    }
}

fn type_name(value: &Value) -> String {
    value.get_type().to_string()
}

/// Adds `value` to the snapshot arena under `name`, returning the arena index.
/// Expandable values get a fresh variablesReference registered in var_refs.
pub(crate) fn add_value(
    snapshot: &mut PauseSnapshot,
    name: String,
    value: &Value,
    depth: usize,
) -> usize {
    // Containers are always expandable, children materializing lazily. So is
    // a closure that captured something: its captures are its children, the
    // only way to see what it closed over.
    let expandable = match value {
        Value::List { .. } | Value::Record { .. } => true,
        Value::Closure { val, .. } => !val.captures.is_empty(),
        _ => false,
    };

    let var_ref = if expandable { snapshot.alloc_ref() } else { 0 };

    // Cheap `Arc` bumps: rendering needs these while `snapshot` is borrowed
    // mutably below.
    let config = snapshot.config.clone();
    let cache = snapshot.cache.clone();
    let ctx = RenderCtx {
        config: &config,
        cache: &cache,
    };

    let node_idx = snapshot.var_arena.len();
    snapshot.var_arena.push(VarNode {
        var: Variable {
            name,
            value: short_render(value, ctx),
            type_: Some(type_name(value)),
            variables_reference: var_ref,
        },
        children: Vec::new(),
        // Full value for `nuDapVisualize`. Clones the subtree once per
        // ancestor, bounded by the eager depth and `MAX_CHILDREN`.
        value: value.clone(),
    });

    if expandable && depth < EAGER_DEPTH {
        materialize_at(snapshot, node_idx, depth);
    }

    node_idx
}

/// Materialize the direct children of the node at `node_idx` (one level).
fn materialize_at(snapshot: &mut PauseSnapshot, node_idx: usize, depth: usize) {
    let var_ref = snapshot.var_arena[node_idx].var.variables_reference;
    if var_ref == 0 || snapshot.var_refs.contains_key(&var_ref) {
        return;
    }
    let value = snapshot.var_arena[node_idx].value.clone();
    let cache = snapshot.cache.clone();
    let mut children = Vec::new();
    match &value {
        Value::Record { val, .. } => {
            for (k, v) in val.iter().take(MAX_CHILDREN) {
                children.push(add_value(snapshot, k.clone(), v, depth + 1));
            }
        }
        Value::List { vals, .. } => {
            for (i, v) in vals.iter().enumerate().take(MAX_CHILDREN) {
                children.push(add_value(snapshot, format!("[{i}]"), v, depth + 1));
            }
        }
        // A closure's children are the variables it closed over, under their
        // source names so they read like the enclosing scope.
        Value::Closure { val, .. } => {
            for (var_id, v) in val.captures.iter().take(MAX_CHILDREN) {
                let name = cache
                    .var_names
                    .get(&var_id.get())
                    .cloned()
                    .unwrap_or_else(|| format!("var{}", var_id.get()));
                children.push(add_value(snapshot, name, v, depth + 1));
            }
        }
        _ => {}
    }
    snapshot.var_refs.insert(var_ref, children.clone());
    snapshot.var_arena[node_idx].children = children;
}

/// On-demand hydration, called when the client expands a node whose children
/// were never materialized. A no-op for refs already populated or unknown.
pub(crate) fn materialize_children(snapshot: &mut PauseSnapshot, var_ref: i64) {
    if var_ref == 0 || snapshot.var_refs.contains_key(&var_ref) {
        return;
    }
    if let Some(idx) = snapshot
        .var_arena
        .iter()
        .position(|n| n.var.variables_reference == var_ref)
    {
        // Children of an on-demand node stay lazy themselves (depth beyond
        // the eager horizon).
        materialize_at(snapshot, idx, EAGER_DEPTH);
    }
}

/// Rebuild a Locals+Globals snapshot for a past timeline entry, without
/// `engine_state` (the server thread must never touch it). Pipeline,
/// Registers and Process are live-only.
pub(crate) fn build_history_snapshot(
    entry: &crate::state::TimelineEntry,
    baseline_env: Option<&std::collections::HashMap<String, Value>>,
    shell_constant: Option<&Value>,
    config: std::sync::Arc<Config>,
    cache: std::sync::Arc<RenderCache>,
) -> PauseSnapshot {
    let mut snap = PauseSnapshot::new();
    snap.frames = entry.frames.clone();
    // Cached by the eval thread, since `engine_state` is off-limits here.
    snap.config = config;
    snap.cache = cache;

    // Locals: `return` first, then shadow vars sorted by name (same order as
    // the live build).
    let mut locals = Vec::new();
    if let Some(v) = &entry.last_result {
        locals.push(add_value(&mut snap, "return".to_string(), v, 0));
    }
    let mut vars: Vec<&crate::state::ShadowVar> = entry.shadow_vars.values().collect();
    vars.sort_by(|a, b| a.name.cmp(&b.name));
    for sv in vars {
        locals.push(add_value(&mut snap, sv.name.clone(), &sv.value, 0));
    }
    snap.var_refs.insert(PauseSnapshot::LOCALS_REF, locals);

    // Globals: $nu (cached constant) and $env (baseline overlaid with the
    // entry's env mutations at that moment).
    let mut globals = Vec::new();
    if let Some(nu) = shell_constant {
        let v = nu.clone();
        globals.push(add_value(&mut snap, "$nu".to_string(), &v, 0));
    }
    {
        let mut env_map: std::collections::BTreeMap<String, Value> = baseline_env
            .map(|b| b.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (k, v) in entry.env_shadow.iter() {
            env_map.insert(k.clone(), v.clone());
        }
        let mut rec = console_protocol::Record::new();
        for (k, v) in env_map {
            rec.push(k, v);
        }
        let env_val = Value::record(rec, console_protocol::Span::unknown());
        globals.push(add_value(&mut snap, "$env".to_string(), &env_val, 0));
    }
    snap.var_refs.insert(PauseSnapshot::GLOBALS_REF, globals);

    // Pipeline: recorded `in → cmd` if this was a pipe-stage boundary.
    let mut pipeline = Vec::new();
    if let Some((cmd, v)) = &entry.pipe_input {
        pipeline.push(add_value(&mut snap, format!("in → {cmd}"), v, 0));
    }
    snap.var_refs.insert(PauseSnapshot::PIPELINE_REF, pipeline);

    // Registers/Process are live-only; empty in the past.
    snap.var_refs
        .insert(PauseSnapshot::REGISTERS_REF, Vec::new());
    snap.var_refs.insert(PauseSnapshot::PROCESS_REF, Vec::new());

    snap
}

/// Bounds for `to_json` (the visualizer payload): generous compared to the
/// Variables tree, but still finite so a huge table can't stall the UI.
const JSON_MAX_ITEMS: usize = 1000;
const JSON_MAX_DEPTH: usize = 8;

/// Convert a nu Value to JSON for the visualizer webview, setting `truncated`
/// when any bound was hit.
///
/// A *display projection*, not a serialization, hence not console-json's `FromValue`
/// (the path behind `to json`): a debugger inspects hostile values — huge,
/// deeply nested, broken — and must always render something. So this one is
/// bounded, renders `Value::Error` inline instead of failing the whole payload,
/// emits binary as a `$nuBinary` hex marker rather than a number array, and
/// needs no engine for closures. See `docs/value-rendering.md`.
///
/// Types with no arm of their own fall through to `to_abbreviated_string`, as
/// in [`short_render`], so a value reads the same in its row and here.
pub(crate) fn to_preview_json(
    value: &Value,
    depth: usize,
    truncated: &mut bool,
    ctx: RenderCtx<'_>,
) -> serde_json::Value {
    let config = ctx.config;
    if depth >= JSON_MAX_DEPTH {
        *truncated = true;
        return J::String("…".into());
    }
    match value {
        Value::String { val, .. } => json!(val),
        Value::Int { val, .. } => json!(val),
        Value::Float { val, .. } => json!(val),
        Value::Bool { val, .. } => json!(val),
        Value::Nothing { .. } => J::Null,
        // Config-driven, like the row: `Filesize`'s own `Display` ignores the
        // user's `filesize` settings and would read `1 kB`, not `1.0 kB`.
        Value::Filesize { val, .. } => json!(config.filesize.format(*val).to_string()),
        Value::Duration { val, .. } => json!(format!("{val}ns")),
        Value::Date { val, .. } => json!(val.to_rfc3339()),
        Value::List { vals, .. } => {
            if vals.len() > JSON_MAX_ITEMS {
                *truncated = true;
            }
            J::Array(
                vals.iter()
                    .take(JSON_MAX_ITEMS)
                    .map(|v| to_preview_json(v, depth + 1, truncated, ctx))
                    .collect(),
            )
        }
        Value::Record { val, .. } => {
            if val.len() > JSON_MAX_ITEMS {
                *truncated = true;
            }
            let mut map = serde_json::Map::new();
            for (k, v) in val.iter().take(JSON_MAX_ITEMS) {
                map.insert(k.clone(), to_preview_json(v, depth + 1, truncated, ctx));
            }
            J::Object(map)
        }
        Value::Closure { val, .. } => json!(closure_label(val, ctx.cache)),
        Value::Binary { val, .. } => {
            // Marker object the webview turns into a hex view. Hex keeps the
            // adapter dependency-free.
            const MAX_BYTES: usize = 65536;
            if val.len() > MAX_BYTES {
                *truncated = true;
            }
            let hex: String = val
                .iter()
                .take(MAX_BYTES)
                .map(|b| format!("{b:02x}"))
                .collect();
            json!({ "$nuBinary": hex, "length": val.len() })
        }
        Value::Error { error, .. } => json!(format!("<error: {error}>")),
        // Ranges, globs, cell-paths, custom values: the shell's own
        // rendering, matching the Variables row.
        other => json!(cap_row(&other.to_abbreviated_string(config))),
    }
}

