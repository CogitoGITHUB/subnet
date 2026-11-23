//! Interactive last-result (`$ans` by default) helpers: truncation and AST detection.

use crate::{
    LAST_VARIABLE_ID, Span, Value,
    ast::{Block, Expr, Expression, Pipeline},
};

/// Truncate `value` so that [`Value::memory_size`] is at most `budget` bytes.
///
/// Returns the (possibly truncated) value and whether truncation occurred.
///
/// Strategy:
/// - **Lists of records (tables):** keep a leading prefix of *whole rows* only. Partial rows
///   (or `nothing` fillers) would break homogeneous-table rendering and collapse to
///   `{record N fields}` list view.
/// - **Other lists:** keep a leading prefix of whole items; for trailing scalars/strings/binary
///   only, a partial last item is allowed when it still fits cleanly.
/// - Records (standalone): fill fields left-to-right until the budget is exhausted
/// - Strings / binary / globs: keep a byte prefix
/// - Other scalar-ish values: keep whole if they fit, otherwise replace with `nothing`
pub fn truncate_value_to_budget(value: Value, budget: usize) -> (Value, bool) {
    if budget == 0 {
        return (Value::nothing(value.span()), true);
    }

    if value.memory_size() <= budget {
        return (value, false);
    }

    let span = value.span();
    match value {
        Value::List { vals, .. } => truncate_list(vals.into_owned(), budget, span),
        Value::Record { val, .. } => truncate_record(val.into_owned(), budget, span),
        Value::String { val, .. } => truncate_string(val, budget, span),
        Value::Binary { val, .. } => truncate_binary(val.into_owned(), budget, span),
        Value::Glob { val, .. } => {
            // Truncated globs become plain strings (prefix of the pattern).
            truncate_string(val, budget, span)
        }
        other => {
            // Cannot partially shrink; store nothing and mark truncated.
            drop(other);
            (Value::nothing(span), true)
        }
    }
}

fn truncate_list(vals: Vec<Value>, budget: usize, span: Span) -> (Value, bool) {
    // Base cost of the list shell; keep adding items while they fit.
    let base = Value::list(vec![], span).memory_size();
    if base > budget {
        return (Value::nothing(span), true);
    }

    let original_len = vals.len();
    // Table-like lists must keep whole rows so `table` still expands columns.
    let table_like = !vals.is_empty() && vals.iter().all(|v| matches!(v, Value::Record { .. }));

    let mut kept = Vec::new();
    let mut used = base;
    let mut truncated = false;

    for item in vals {
        let item_size = item.memory_size();
        if used.saturating_add(item_size) <= budget {
            used += item_size;
            kept.push(item);
            continue;
        }

        // Item does not fit whole.
        truncated = true;

        if table_like {
            // Drop this row and stop — do not emit a partial record or `nothing`.
            break;
        }

        // For non-table lists, allow a partial last scalar/string/binary only.
        // Never push `nothing` or empty placeholders that change list shape.
        let remaining = budget.saturating_sub(used);
        if remaining > 0 && can_partially_truncate_list_item(&item) {
            let (partial, _) = truncate_value_to_budget(item, remaining);
            if !matches!(partial, Value::Nothing { .. })
                && used.saturating_add(partial.memory_size()) <= budget
            {
                kept.push(partial);
            }
        }
        break;
    }

    if kept.len() < original_len {
        truncated = true;
    }

    let out = Value::list(kept, span);
    let still_over = out.memory_size() > budget;
    (out, truncated || still_over)
}

/// Whether a list item is a type we are willing to partially shrink as the last element.
fn can_partially_truncate_list_item(item: &Value) -> bool {
    matches!(
        item,
        Value::String { .. } | Value::Binary { .. } | Value::Glob { .. } | Value::List { .. }
    )
}

fn truncate_record(record: crate::Record, budget: usize, span: Span) -> (Value, bool) {
    let base = Value::record(crate::Record::new(), span).memory_size();
    if base > budget {
        return (Value::nothing(span), true);
    }

    let mut out = crate::Record::new();
    let mut used = base;
    let mut truncated = false;
    let original_len = record.len();

    for (key, val) in record {
        let key_cost = key.capacity();
        let remaining = budget.saturating_sub(used.saturating_add(key_cost));
        if remaining == 0 {
            truncated = true;
            break;
        }

        let (stored_val, val_trunc) = if val.memory_size() <= remaining {
            (val, false)
        } else {
            let (v, t) = truncate_value_to_budget(val, remaining);
            (v, t)
        };

        let entry_size = key_cost + stored_val.memory_size();
        if used.saturating_add(entry_size) > budget {
            truncated = true;
            break;
        }
        used += entry_size;
        truncated |= val_trunc;
        out.push(key, stored_val);
    }

    if out.len() < original_len {
        truncated = true;
    }

    (Value::record(out, span), truncated)
}

fn truncate_string(val: String, budget: usize, span: Span) -> (Value, bool) {
    // memory_size for string is size_of::<Value>() + capacity.
    let whole = Value::string(val, span);
    if whole.memory_size() <= budget {
        return (whole, false);
    }

    let val = match whole {
        Value::String { val, .. } => val,
        other => {
            // Only strings are passed here; fall back without panicking.
            drop(other);
            return (Value::nothing(span), true);
        }
    };

    if std::mem::size_of::<Value>() > budget {
        return (Value::nothing(span), true);
    }

    // Shrink content until Value::memory_size fits. capacity of a freshly built String equals len.
    let mut end = budget
        .saturating_sub(std::mem::size_of::<Value>())
        .min(val.len());
    while end > 0 && !val.is_char_boundary(end) {
        end -= 1;
    }

    loop {
        let prefix = val[..end].to_string();
        let out = Value::string(prefix, span);
        if out.memory_size() <= budget {
            return (out, true);
        }
        if end == 0 {
            return (Value::nothing(span), true);
        }
        end -= 1;
        while end > 0 && !val.is_char_boundary(end) {
            end -= 1;
        }
    }
}

fn truncate_binary(val: Vec<u8>, budget: usize, span: Span) -> (Value, bool) {
    let whole = Value::binary(val, span);
    if whole.memory_size() <= budget {
        return (whole, false);
    }

    let val = match whole {
        Value::Binary { val, .. } => val,
        other => {
            drop(other);
            return (Value::nothing(span), true);
        }
    };

    if std::mem::size_of::<Value>() > budget {
        return (Value::nothing(span), true);
    }

    let mut end = budget
        .saturating_sub(std::mem::size_of::<Value>())
        .min(val.len());
    loop {
        let out = Value::binary(val[..end].to_vec(), span);
        if out.memory_size() <= budget {
            return (out, true);
        }
        if end == 0 {
            return (Value::nothing(span), true);
        }
        end -= 1;
    }
}

/// True when `value` is a [`Value::Error`] or a non-empty list of only errors.
///
/// Commands like `str length` embed type mismatches as values via `operate()`
/// instead of failing the pipeline. Interactive last-result must not replace
/// `$ans.last` with those payloads (same policy as a thrown runtime error).
/// Mixed lists (some errors, some values) are real results and return false.
pub fn value_is_error_only(value: &Value) -> bool {
    match value {
        Value::Error { .. } => true,
        Value::List { vals, .. } => !vals.is_empty() && vals.iter().all(Value::is_error),
        _ => false,
    }
}

/// Returns true when `block` is only a reference to the last-result variable or a
/// cell-path rooted at it (e.g. `$ans`, `$ans.last`, `$ans.exit_code`, `$ans.command`),
/// optionally wrapped in parentheses / a single-element pipeline.
///
/// Such expressions must not overwrite `$ans.last` when re-evaluated.
pub fn block_is_bare_last_result(block: &Block) -> bool {
    if block.pipelines.len() != 1 {
        return false;
    }
    pipeline_is_bare_last_result(&block.pipelines[0])
}

fn pipeline_is_bare_last_result(pipeline: &Pipeline) -> bool {
    if pipeline.elements.len() != 1 {
        return false;
    }
    let element = &pipeline.elements[0];
    if element.redirection.is_some() {
        return false;
    }
    expr_is_bare_last_result(&element.expr)
}

fn expr_is_bare_last_result(expr: &Expression) -> bool {
    match &expr.expr {
        Expr::Var(var_id) => *var_id == LAST_VARIABLE_ID,
        Expr::FullCellPath(path) => expr_is_bare_last_result(&path.head),
        // Parenthesized subexpression: `($ans)` / `($ans.last)`
        Expr::Block(block_id) | Expr::RowCondition(block_id) | Expr::Closure(block_id) => {
            // These shouldn't appear for simple paren groups; paren groups are usually Subexpression
            let _ = block_id;
            false
        }
        Expr::Subexpression(block_id) => {
            // Handled at call site if we have working set; treat conservatively as false here
            // unless we only have the Expression. Callers with EngineState should use the
            // overload below. Without block body we can't know — return false.
            let _ = block_id;
            false
        }
        _ => false,
    }
}

/// Like [`block_is_bare_last_result`] but expands subexpressions via `get_block`.
///
/// Uses a trait object so recursive subexpression walks do not monomorphize infinitely.
pub fn block_is_bare_last_result_with<'a>(
    block: &Block,
    get_block: &mut dyn FnMut(crate::BlockId) -> &'a Block,
) -> bool {
    if block.pipelines.len() != 1 {
        return false;
    }
    let pipeline = &block.pipelines[0];
    if pipeline.elements.len() != 1 {
        return false;
    }
    let element = &pipeline.elements[0];
    if element.redirection.is_some() {
        return false;
    }
    expr_is_bare_last_result_with(&element.expr, get_block)
}

fn expr_is_bare_last_result_with<'a>(
    expr: &Expression,
    get_block: &mut dyn FnMut(crate::BlockId) -> &'a Block,
) -> bool {
    match &expr.expr {
        Expr::Var(var_id) => *var_id == LAST_VARIABLE_ID,
        Expr::FullCellPath(path) => expr_is_bare_last_result_with(&path.head, get_block),
        Expr::Subexpression(block_id) => {
            let inner = get_block(*block_id);
            block_is_bare_last_result_with(inner, get_block)
        }
        _ => false,
    }
}

