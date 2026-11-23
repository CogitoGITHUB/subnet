use crate::database::{MEMORY_DB, SQLiteDatabase, get_shared_mem_conn, values_to_sql};
use console_engine::command_prelude::*;
use console_protocol::shell_error::generic::GenericError;
use rusqlite::{Connection, params_from_iter};

#[derive(Clone)]
pub struct StorInsert;

impl Command for StorInsert {
    fn name(&self) -> &str {
        "stor insert"
    }

    fn signature(&self) -> Signature {
        Signature::build("stor insert")
            .input_output_types(vec![
                (Type::Nothing, Type::table()),
                (Type::record(), Type::table()),
                (Type::table(), Type::table()),
                // FIXME Type::Any input added to disable pipeline input type checking, as run-time checks can raise undesirable type errors
                // which aren't caught by the parser. see https://github.com/nushell/shell/pull/14922 for more details
                (Type::Any, Type::table()),
            ])
            .required_named(
                "table-name",
                SyntaxShape::String,
                "Name of the table you want to insert into.",
                Some('t'),
            )
            .named(
                "data-record",
                SyntaxShape::record(),
                "A record of column names and column values to insert into the specified table. Takes precedence over pipeline input.",
                Some('d'),
            )
            .allow_variants_without_examples(true)
            .category(Category::Database)
    }

    fn description(&self) -> &str {
        "Insert information into a specified table in the in-memory sqlite database."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["sqlite", "storing", "table", "saving"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Insert data in the in-memory sqlite database using a data-record of column-name and column-value pairs",
                example: "stor insert --table-name nudb --data-record {bool1: true, int1: 5, float1: 1.1, str1: fdncred, datetime1: 2023-04-17}",
                result: None,
            },
            Example {
                description: "Insert data through pipeline input as a record of column-name and column-value pairs",
                example: "{bool1: true, int1: 5, float1: 1.1, str1: fdncred, datetime1: 2023-04-17} | stor insert --table-name nudb",
                result: None,
            },
            Example {
                description: "Insert data through pipeline input as a table literal",
                example: "[[bool1 int1 float1]; [true 5 1.1], [false 8 3.14]] | stor insert --table-name nudb",
                result: None,
            },
            Example {
                description: "Insert ls entries",
                example: "ls | stor insert --table-name files",
                result: None,
            },
            Example {
                description: "Insert nu records as json data",
                example: "ls -l | each {{file: $in.name, metadata: ($in | reject name)}} | stor insert --table-name files_with_md",
                result: None,
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
        let span = call.head;
        let table_name: Option<String> = call.get_flag(engine_state, stack, "table-name")?;
        let data_record: Option<Record> = call.get_flag(engine_state, stack, "data-record")?;

        let records = handle(span, data_record, input)?;

        let conn = get_shared_mem_conn()?;
        for record in records {
            process(engine_state, table_name.clone(), span, &conn, record)?;
        }

        let db = Box::new(SQLiteDatabase::new(
            std::path::Path::new(MEMORY_DB),
            engine_state.signals().clone(),
        ));
        Ok(Value::custom(db, span).into_pipeline_data())
    }
}

fn handle(
    span: Span,
    data_record: Option<Record>,
    input: PipelineData,
) -> Result<Vec<Record>, ShellError> {
    // The flag wins over the input: the first statement of a block receives the block's
    // input implicitly, so `each {|r| stor insert -t t -d $r }` gets both.
    if let Some(record) = data_record {
        return Ok(vec![record]);
    }

    // Handle the input types
    let values = match input {
        PipelineData::Empty => {
            return Err(ShellError::MissingParameter {
                param_name: "requires a table or a record".into(),
                span,
            });
        }
        PipelineData::ListStream(stream, ..) => stream.into_iter().collect::<Vec<_>>(),
        PipelineData::Value(Value::List { vals, .. }, ..) => vals.into_owned(),
        PipelineData::Value(val, ..) => vec![val],
        _ => {
            return Err(ShellError::OnlySupportsThisInputType {
                exp_input_type: "list or record".into(),
                wrong_type: "".into(),
                dst_span: span,
                src_span: span,
            });
        }
    };

    values
        .into_iter()
        .map(|val| match val {
            Value::Record { val, .. } => Ok(val.into_owned()),
            other => Err(ShellError::OnlySupportsThisInputType {
                exp_input_type: "record".into(),
                wrong_type: other.get_type().to_string(),
                dst_span: span,
                src_span: other.span(),
            }),
        })
        .collect()
}

fn process(
    engine_state: &EngineState,
    table_name: Option<String>,
    span: Span,
    conn: &Connection,
    record: Record,
) -> Result<(), ShellError> {
    if table_name.is_none() {
        return Err(ShellError::MissingParameter {
            param_name: "requires at table name".into(),
            span,
        });
    }
    let new_table_name = table_name.unwrap_or("table".into());

    let mut create_stmt = format!("INSERT INTO {new_table_name} (");
    let mut column_placeholders: Vec<String> = Vec::new();

    let cols = record.columns();
    cols.for_each(|col| {
        column_placeholders.push(col.to_string());
    });

    create_stmt.push_str(&column_placeholders.join(", "));

    // Values are set as placeholders.
    create_stmt.push_str(") VALUES (");
    let mut value_placeholders: Vec<String> = Vec::new();
    for (index, _) in record.columns().enumerate() {
        value_placeholders.push(format!("?{}", index + 1));
    }
    create_stmt.push_str(&value_placeholders.join(", "));
    create_stmt.push(')');

    // dbg!(&create_stmt);

    // Get the params from the passed values
    let params = values_to_sql(engine_state, record.values().cloned(), span)?;

    conn.execute(&create_stmt, params_from_iter(params))
        .map_err(|err| {
            ShellError::Generic(GenericError::new_internal(
                "Failed to insert using the SQLite connection to the in-memory database from insert.rs.",
                err.to_string(),
            ))
        })?;
    Ok(())
}

