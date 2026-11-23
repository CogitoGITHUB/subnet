use console_protocol::{
    DEFAULT_ERROR_CONTEXT, Filesize, IntoValue, ParseError, Range, Record, ShellError, Span, Type,
    Unit, Value,
    ast::{Expr, Expression, ListItem, RecordItem},
    engine::{EngineState, StateWorkingSet},
    shell_error::generic::GenericError,
    truncated_source_window,
};
use std::{borrow::Cow, sync::Arc};

fn truncated_nuon_error(src: &str, err_span: Span, msg: impl Into<String>) -> ShellError {
    let (src, span) = truncated_source_window(src, err_span, DEFAULT_ERROR_CONTEXT);
    ShellError::OutsideSpannedLabeledError {
        src,
        error: "Error when loading".into(),
        msg: msg.into(),
        span,
    }
}

/// The error for a filesize or duration (`ty`) at `err_span` that doesn't fit in 64 bits.
fn too_large_error(src: &str, err_span: Span, ty: &str) -> ShellError {
    let msg = format!("{ty} too large");
    if ty == "filesize" {
        let (src, span) = truncated_source_window(src, err_span, DEFAULT_ERROR_CONTEXT);
        ShellError::OutsideSpannedLabeledError {
            src,
            error: msg.clone(),
            msg,
            span,
        }
    } else {
        truncated_nuon_error(src, err_span, msg)
    }
}

/// convert a raw string representation of NUON data to an actual Nushell [`Value`]
///
// WARNING: please leave the following two trailing spaces, they matter for the documentation
// formatting
/// > **Note**
/// > [`Span`] can be passed to [`from_nuon`] if there is context available to the caller, e.g. when
/// > using this function in a command implementation such as
/// > [`from nuon`](https://www.nushell.sh/commands/docs/from_nuon.html).
///
/// also see [`super::to_nuon`] for the inverse operation
pub fn from_nuon(input: &str, span: Option<Span>) -> Result<Value, ShellError> {
    let mut engine_state = EngineState::default();
    // NOTE: the parser needs `$env.PWD` to be set, that's a know _API issue_ with the
    // [`EngineState`]
    engine_state.add_env_var("PWD".to_string(), Value::string("", Span::unknown()));
    let mut working_set = StateWorkingSet::new(&engine_state);

    // The parser skips exactly one leading byte order mark, as it does for a script.
    // Windows editors and PowerShell's `Out-File` write one, so a reader that refuses
    // them cannot open files people actually have. `U+FEFF` anywhere else is ordinary
    // content.
    let mut block = console_parser::parse(&mut working_set, None, input.as_bytes(), false);

    if let Some(pipeline) = block.pipelines.get(1) {
        if let Some(element) = pipeline.elements.first() {
            let (src, label_span) =
                truncated_source_window(input, element.expr.span, DEFAULT_ERROR_CONTEXT);
            return Err(ShellError::Generic(
                make_generic_error(
                    span,
                    "error when loading nuon text",
                    "could not load nuon text",
                )
                .with_inner([ShellError::OutsideSpannedLabeledError {
                    src,
                    error: "error when loading".into(),
                    msg: "excess values when loading".into(),
                    span: label_span,
                }]),
            ));
        } else {
            return Err(ShellError::Generic(
                make_generic_error(
                    span,
                    "error when loading nuon text",
                    "could not load nuon text",
                )
                .with_inner([ShellError::Generic(make_generic_error(
                    span,
                    "error when loading",
                    "excess values when loading",
                ))]),
            ));
        }
    }

    let expr = if block.pipelines.is_empty() {
        Expression::new(
            &mut working_set,
            Expr::Nothing,
            span.unwrap_or(Span::unknown()),
            Type::Nothing,
        )
    } else {
        let mut pipeline = Arc::make_mut(&mut block).pipelines.remove(0);

        if let Some(expr) = pipeline.elements.get(1) {
            let (src, label_span) =
                truncated_source_window(input, expr.expr.span, DEFAULT_ERROR_CONTEXT);
            return Err(ShellError::Generic(
                make_generic_error(
                    span,
                    "error when loading nuon text",
                    "could not load nuon text",
                )
                .with_inner([ShellError::OutsideSpannedLabeledError {
                    src,
                    error: "error when loading".into(),
                    msg: "detected a pipeline in nuon file".into(),
                    span: label_span,
                }]),
            ));
        }

        if pipeline.elements.is_empty() {
            Expression::new(
                &mut working_set,
                Expr::Nothing,
                span.unwrap_or(Span::unknown()),
                Type::Nothing,
            )
        } else {
            pipeline.elements.remove(0).expr
        }
    };

    // The parser rejects a repeated key and a filesize or duration too large for 64 bits; report
    // them as the errors they are rather than as a general parse failure.
    match working_set.parse_errors.first() {
        Some(ParseError::ColumnDefinedTwice(col_name, second_use, first_use)) => {
            return Err(ShellError::ColumnDefinedTwice {
                col_name: col_name.clone(),
                second_use: *second_use,
                first_use: *first_use,
                help: None,
            });
        }
        Some(ParseError::LiteralTooLarge(ty, span)) => {
            return Err(too_large_error(input, *span, ty));
        }
        _ => {}
    }

    if let Some(err) = working_set.parse_errors.first() {
        let (src, label_span) = truncated_source_window(input, err.span(), DEFAULT_ERROR_CONTEXT);
        return Err(ShellError::Generic(
            make_generic_error(
                span,
                "error when parsing nuon text",
                "could not parse nuon text",
            )
            .with_inner([ShellError::OutsideSpannedLabeledError {
                src,
                error: "error when parsing".into(),
                msg: err.to_string(),
                span: label_span,
            }]),
        ));
    }

    let value = convert_to_value(expr, span.unwrap_or(Span::unknown()), input)?;

    Ok(value)
}

fn make_generic_error(
    span: Option<Span>,
    error: impl Into<Cow<'static, str>>,
    msg: impl Into<Cow<'static, str>>,
) -> GenericError {
    match span {
        Some(span) => GenericError::new(error, msg, span),
        None => GenericError::new_internal(error, msg),
    }
}

fn convert_to_value(
    expr: Expression,
    span: Span,
    original_text: &str,
) -> Result<Value, ShellError> {
    match expr.expr {
        Expr::AttributeBlock(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "attributes not supported in nuon",
        )),
        Expr::BinaryOp(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "binary operators not supported in nuon",
        )),
        Expr::UnaryNot(..) | Expr::UnaryMinus(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "unary operators not supported in nuon",
        )),
        Expr::Block(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "blocks not supported in nuon",
        )),
        Expr::Closure(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "closures not supported in nuon",
        )),
        Expr::Binary(val) => Ok(Value::binary(val, span)),
        Expr::Bool(val) => Ok(Value::bool(val, span)),
        Expr::Call(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "calls not supported in nuon",
        )),
        Expr::CellPath(val) => Ok(Value::cell_path(val, span)),
        Expr::DateTime(dt) => Ok(Value::date(dt, span)),
        Expr::ExternalCall(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "calls not supported in nuon",
        )),
        Expr::Filepath(val, _) => Ok(Value::string(val, span)),
        Expr::Directory(val, _) => Ok(Value::string(val, span)),
        Expr::Float(val) => Ok(Value::float(val, span)),
        Expr::FullCellPath(full_cell_path) => {
            if !full_cell_path.tail.is_empty() {
                Err(truncated_nuon_error(
                    original_text,
                    expr.span,
                    "subexpressions and cellpaths not supported in nuon",
                ))
            } else {
                convert_to_value(full_cell_path.head, span, original_text)
            }
        }

        Expr::Garbage => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "extra tokens in input file",
        )),
        Expr::GlobPattern(val, _) => Ok(Value::string(val, span)),
        Expr::ImportPattern(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "imports not supported in nuon",
        )),
        Expr::Overlay(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "overlays not supported in nuon",
        )),
        Expr::Int(val) => Ok(Value::int(val, span)),
        Expr::Keyword(kw) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            format!(
                "{} not supported in nuon",
                String::from_utf8_lossy(&kw.keyword)
            ),
        )),
        Expr::List(vals) => {
            let mut output = vec![];

            for item in vals {
                match item {
                    ListItem::Item(expr) => {
                        output.push(convert_to_value(expr, span, original_text)?);
                    }
                    ListItem::Spread(_, inner) => {
                        return Err(truncated_nuon_error(
                            original_text,
                            inner.span,
                            "spread operator not supported in nuon",
                        ));
                    }
                }
            }

            Ok(Value::list(output, span))
        }
        Expr::MatchBlock(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "match blocks not supported in nuon",
        )),
        Expr::Nothing => Ok(Value::nothing(span)),
        Expr::Operator(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "operators not supported in nuon",
        )),
        Expr::Range(range) => {
            let from = if let Some(f) = range.from {
                convert_to_value(f, span, original_text)?
            } else {
                Value::nothing(expr.span)
            };

            let next = if let Some(s) = range.next {
                convert_to_value(s, span, original_text)?
            } else {
                Value::nothing(expr.span)
            };

            let to = if let Some(t) = range.to {
                convert_to_value(t, span, original_text)?
            } else {
                Value::nothing(expr.span)
            };

            Ok(Value::range(
                Range::new(from, next, to, range.operator.inclusion, expr.span)?,
                expr.span,
            ))
        }
        Expr::Record(key_vals) => {
            let mut record = Record::with_capacity(key_vals.len());

            // The parser has already rejected a key used twice (see `from_nuon`).
            for key_val in key_vals {
                match key_val {
                    RecordItem::Pair(key, val) => {
                        let key_str = match key.expr {
                            Expr::String(key_str) => key_str,
                            _ => {
                                return Err(truncated_nuon_error(
                                    original_text,
                                    key.span,
                                    "only strings can be keys",
                                ));
                            }
                        };

                        record.push(key_str, convert_to_value(val, span, original_text)?);
                    }
                    RecordItem::Spread(_, inner) => {
                        return Err(truncated_nuon_error(
                            original_text,
                            inner.span,
                            "spread operator not supported in nuon",
                        ));
                    }
                }
            }

            Ok(Value::record(record, span))
        }
        Expr::RowCondition(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "row conditions not supported in nuon",
        )),
        Expr::Signature(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "signatures not supported in nuon",
        )),
        Expr::String(s) | Expr::RawString(s) => Ok(Value::string(s.clone(), span)),
        Expr::StringInterpolation(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "string interpolation not supported in nuon",
        )),
        Expr::GlobInterpolation(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "glob interpolation not supported in nuon",
        )),
        Expr::Collect(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "`$in` not supported in nuon",
        )),
        Expr::Subexpression(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "subexpressions not supported in nuon",
        )),
        Expr::Table(mut table) => {
            let mut cols = vec![];

            let mut output = vec![];

            // The parser has already rejected a column named twice (see `from_nuon`).
            for key in table.columns.as_mut() {
                let key_str = match &mut key.expr {
                    Expr::String(key_str) => key_str,
                    _ => {
                        return Err(truncated_nuon_error(
                            original_text,
                            expr.span,
                            "only strings can be keys",
                        ));
                    }
                };

                cols.push(std::mem::take(key_str));
            }

            for row in table.rows.into_vec() {
                if cols.len() != row.len() {
                    return Err(truncated_nuon_error(
                        original_text,
                        expr.span,
                        "table has mismatched columns",
                    ));
                }

                let record = cols
                    .iter()
                    .zip(row.into_vec())
                    .map(|(col, cell)| {
                        convert_to_value(cell, span, original_text).map(|val| (col.clone(), val))
                    })
                    .collect::<Result<_, _>>()?;

                output.push(Value::record(record, span));
            }

            Ok(Value::list(output, span))
        }
        Expr::ValueWithUnit(value) => {
            let size = match value.expr.expr {
                Expr::Int(val) => val,
                _ => {
                    return Err(truncated_nuon_error(
                        original_text,
                        expr.span,
                        "non-integer unit value",
                    ));
                }
            };

            // The parser stores every filesize in bytes and every duration in nanoseconds, and
            // rejects one too large for 64 bits, so parsed text always fits here. A value with
            // a larger unit still has to be checked against the 64-bit range.
            match value.unit.item {
                Unit::Filesize(unit) => match Filesize::from_unit(size, unit) {
                    Some(val) => Ok(val.into_value(span)),
                    None => Err(too_large_error(original_text, expr.span, "filesize")),
                },

                // Every duration unit goes through `Unit::build_value`, which checks the
                // multiplication against the 64-bit range. A plain `size * factor` would panic
                // in debug builds and wrap to a negative duration in release ones, and the nuon
                // spec requires an error.
                unit => unit
                    .build_value(size, span)
                    .map_err(|_| too_large_error(original_text, expr.span, "duration")),
            }
        }
        Expr::Var(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "variables not supported in nuon",
        )),
        Expr::VarDecl(..) => Err(truncated_nuon_error(
            original_text,
            expr.span,
            "variable declarations not supported in nuon",
        )),
    }
}

