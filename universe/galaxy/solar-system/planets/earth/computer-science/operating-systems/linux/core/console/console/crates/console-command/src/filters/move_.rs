use std::ops::Not;

use console_engine::command_prelude::*;
use console_protocol::shell_error::generic::GenericError;

#[derive(Clone, Debug)]
enum Location {
    Before(Spanned<String>),
    After(Spanned<String>),
    Last,
    First,
}

#[derive(Clone)]
pub struct Move;

impl Command for Move {
    fn name(&self) -> &str {
        "move"
    }

    fn description(&self) -> &str {
        "Moves columns relative to other columns or make them the first/last columns. Flags are mutually exclusive."
    }

    fn signature(&self) -> console_protocol::Signature {
        Signature::build("move")
            .input_output_types(vec![
                (Type::record(), Type::record()),
                (Type::table(), Type::table()),
            ])
            .rest("columns", SyntaxShape::String, "The columns to move.")
            .named(
                "after",
                SyntaxShape::String,
                "The column that will precede the columns moved.",
                None,
            )
            .named(
                "before",
                SyntaxShape::String,
                "The column that will be the next after the columns moved.",
                None,
            )
            .switch("first", "Makes the columns be the first ones.", None)
            .switch("last", "Makes the columns be the last ones.", None)
            .category(Category::Filters)
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "[[name value index]; [foo a 1] [bar b 2] [baz c 3]] | move index --before name",
                description: "Move a column before the first column.",
                result: Some(Value::test_list(vec![
                    Value::test_record(record! {
                        "index" => Value::test_int(1),
                        "name" =>  Value::test_string("foo"),
                        "value" => Value::test_string("a"),
                    }),
                    Value::test_record(record! {
                        "index" => Value::test_int(2),
                        "name" =>  Value::test_string("bar"),
                        "value" => Value::test_string("b"),
                    }),
                    Value::test_record(record! {
                        "index" => Value::test_int(3),
                        "name" =>  Value::test_string("baz"),
                        "value" => Value::test_string("c"),
                    }),
                ])),
            },
            Example {
                example: "[[name value index]; [foo a 1] [bar b 2] [baz c 3]] | move value name --after index",
                description: "Move multiple columns after the last column and reorder them.",
                result: Some(Value::test_list(vec![
                    Value::test_record(record! {
                        "index" => Value::test_int(1),
                        "value" => Value::test_string("a"),
                        "name" =>  Value::test_string("foo"),
                    }),
                    Value::test_record(record! {
                        "index" => Value::test_int(2),
                        "value" => Value::test_string("b"),
                        "name" =>  Value::test_string("bar"),
                    }),
                    Value::test_record(record! {
                        "index" => Value::test_int(3),
                        "value" => Value::test_string("c"),
                        "name" =>  Value::test_string("baz"),
                    }),
                ])),
            },
            Example {
                example: "{ name: foo, value: a, index: 1 } | move name --before index",
                description: "Move columns of a record.",
                result: Some(Value::test_record(record! {
                    "value" => Value::test_string("a"),
                    "name" => Value::test_string("foo"),
                    "index" => Value::test_int(1),
                })),
            },
        ]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let mut input = input.into_stream_or_original(engine_state);
        let head = call.head;
        let columns: Vec<Value> = call.rest(engine_state, stack, 0)?;
        let after: Option<Value> = call.get_flag(engine_state, stack, "after")?;
        let before: Option<Value> = call.get_flag(engine_state, stack, "before")?;
        let first = call.has_flag(engine_state, stack, "first")?;
        let last = call.has_flag(engine_state, stack, "last")?;

        let location = match (after, before, first, last) {
            (Some(v), None, false, false) => Location::After(Spanned {
                span: v.span(),
                item: v.coerce_into_string()?,
            }),
            (None, Some(v), false, false) => Location::Before(Spanned {
                span: v.span(),
                item: v.coerce_into_string()?,
            }),
            (None, None, true, false) => Location::First,
            (None, None, false, true) => Location::Last,
            (None, None, false, false) => {
                return Err(ShellError::Generic(GenericError::new(
                    "Cannot move columns",
                    "Missing required location flag",
                    head,
                )));
            }
            _ => {
                return Err(ShellError::Generic(GenericError::new(
                    "Cannot move columns",
                    "Use only a single flag",
                    head,
                )));
            }
        };

        let metadata = input.take_metadata();

        match input {
            PipelineData::Value(Value::List { .. }, ..) | PipelineData::ListStream { .. } => {
                let res = input.into_iter().map(move |x| match x.as_record() {
                    Ok(record) => match move_record_columns(record, &columns, &location, head) {
                        Ok(val) => val,
                        Err(error) => Value::error(error, head),
                    },
                    Err(error) => Value::error(error, head),
                });

                Ok(res.into_pipeline_data_with_metadata(
                    head,
                    engine_state.signals().clone(),
                    metadata,
                ))
            }
            PipelineData::Value(Value::Record { val, .. }, ..) => {
                Ok(move_record_columns(&val, &columns, &location, head)?
                    .into_pipeline_data_with_metadata(metadata))
            }
            other => Err(ShellError::OnlySupportsThisInputType {
                exp_input_type: "record or table".to_string(),
                wrong_type: other.get_type().to_string(),
                dst_span: head,
                src_span: Span::new(head.start, head.start),
            }),
        }
    }
}

// Move columns within a record
fn move_record_columns(
    record: &Record,
    columns: &[Value],
    location: &Location,
    span: Span,
) -> Result<Value, ShellError> {
    let mut column_idx: Vec<usize> = Vec::with_capacity(columns.len());

    // Find indices of columns to be moved
    for column in columns.iter() {
        if let Some(idx) = record.index_of(column.coerce_string()?) {
            column_idx.push(idx);
        } else {
            return Err(ShellError::Generic(GenericError::new(
                "Cannot move columns",
                "column does not exist",
                column.span(),
            )));
        }
    }

    let mut out = Record::with_capacity(record.len());

    match &location {
        Location::Before(pivot) | Location::After(pivot) => {
            // Check if pivot exists
            if !record.contains(&pivot.item) {
                return Err(ShellError::Generic(GenericError::new(
                    "Cannot move columns",
                    "column does not exist",
                    pivot.span,
                )));
            }

            for (i, (inp_col, inp_val)) in record.iter().enumerate() {
                if inp_col == &pivot.item {
                    // Check if this pivot is also a column we are supposed to move
                    if column_idx.contains(&i) {
                        return Err(ShellError::IncompatibleParameters {
                            left_message: "Column cannot be moved".to_string(),
                            left_span: inp_val.span(),
                            right_message: "relative to itself".to_string(),
                            right_span: pivot.span,
                        });
                    }

                    if let Location::After(..) = location {
                        out.push(inp_col.clone(), inp_val.clone());
                    }

                    insert_moved(record, span, &column_idx, &mut out)?;

                    if let Location::Before(..) = location {
                        out.push(inp_col.clone(), inp_val.clone());
                    }
                } else if !column_idx.contains(&i) {
                    out.push(inp_col.clone(), inp_val.clone());
                }
            }
        }
        Location::First => {
            insert_moved(record, span, &column_idx, &mut out)?;

            out.extend(where_unmoved(record, &column_idx));
        }
        Location::Last => {
            out.extend(where_unmoved(record, &column_idx));

            insert_moved(record, span, &column_idx, &mut out)?;
        }
    };

    Ok(Value::record(out, span))
}

fn where_unmoved<'a>(
    record: &'a Record,
    column_idx: &'a [usize],
) -> impl Iterator<Item = (String, Value)> + use<'a> {
    record
        .iter()
        .enumerate()
        .filter(|(i, _)| column_idx.contains(i).not())
        .map(|(_, (c, v))| (c.clone(), v.clone()))
}

fn insert_moved(
    record: &Record,
    span: Span,
    column_idx: &[usize],
    out: &mut Record,
) -> Result<(), ShellError> {
    for idx in column_idx.iter() {
        if let Some((col, val)) = record.get_index(*idx) {
            out.push(col.clone(), val.clone());
        } else {
            return Err(ShellError::NushellFailedSpanned {
                msg: "Error indexing input columns".to_string(),
                label: "originates from here".to_string(),
                span,
            });
        }
    }
    Ok(())
}

