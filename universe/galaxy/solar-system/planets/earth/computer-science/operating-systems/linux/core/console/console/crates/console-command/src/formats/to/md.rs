use console_cmd_base::formats::to::delimited::merge_descriptors;
use console_engine::command_prelude::*;
use console_protocol::{Config, ast::PathMember};
use std::collections::HashSet;

#[derive(Clone)]
pub struct ToMd;

/// Defines how lists should be formatted in Markdown output
#[derive(Clone, Copy, Default, PartialEq)]
enum ListStyle {
    /// No list markers, just plain text separated by newlines
    None,
    /// Ordered list using "1. ", "2. ", etc.
    Ordered,
    /// Unordered list using "* " (default)
    #[default]
    Unordered,
}

impl ListStyle {
    const OPTIONS: &[&'static str] = &["ordered", "unordered", "none"];

    fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "ordered" => Some(Self::Ordered),
            "unordered" => Some(Self::Unordered),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

struct ToMdOptions {
    pretty: bool,
    per_element: bool,
    center: Option<Vec<CellPath>>,
    escape_md: bool,
    escape_html: bool,
    list_style: ListStyle,
}

/// Special markdown element types that can be represented as single-field records
const SPECIAL_MARKDOWN_HEADERS: &[&str] = &["h1", "h2", "h3", "blockquote"];

/// Check if a record represents a special markdown element (h1, h2, h3, blockquote)
fn is_special_markdown_record(record: &console_protocol::Record) -> bool {
    record.len() == 1
        && record.get_index(0).is_some_and(|(header, _)| {
            SPECIAL_MARKDOWN_HEADERS.contains(&header.to_ascii_lowercase().as_str())
        })
}

impl Command for ToMd {
    fn name(&self) -> &str {
        "to md"
    }

    fn signature(&self) -> Signature {
        Signature::build("to md")
            .input_output_types(vec![(Type::Any, Type::String)])
            .switch(
                "pretty",
                "Formats the Markdown table to vertically align items.",
                Some('p'),
            )
            .switch(
                "per-element",
                "Treat each row as markdown syntax element.",
                Some('e'),
            )
            .named(
                "center",
                SyntaxShape::List(Box::new(SyntaxShape::CellPath)),
                "Formats the Markdown table to center given columns.",
                Some('c'),
            )
            .switch(
                "escape-md",
                "Escapes Markdown special characters.",
                Some('m'),
            )
            .switch("escape-html", "Escapes HTML special characters.", Some('t'))
            .switch(
                "escape-all",
                "Escapes both Markdown and HTML special characters.",
                Some('a'),
            )
            .named(
                "list",
                SyntaxShape::String,
                "Format lists as 'ordered' (1. 2. 3.), 'unordered' (* * *), or 'none'. Default: unordered.",
                Some('l'),
            )
            .category(Category::Formats)
    }

    fn description(&self) -> &str {
        "Convert table into simple Markdown."
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Outputs an MD string representing the contents of this table.",
                example: "[[foo bar]; [1 2]] | to md",
                result: Some(Value::test_string(
                    "| foo | bar |\n| --- | --- |\n| 1 | 2 |",
                )),
            },
            Example {
                description: "Optionally, output a formatted markdown string.",
                example: "[[foo bar]; [1 2]] | to md --pretty",
                result: Some(Value::test_string(
                    "| foo | bar |\n| --- | --- |\n| 1   | 2   |",
                )),
            },
            Example {
                description: "Treat each row as a markdown element.",
                example: r#"[{"H1": "Welcome to Nushell" } [[foo bar]; [1 2]]] | to md --per-element --pretty"#,
                result: Some(Value::test_string(
                    "# Welcome to Nushell\n| foo | bar |\n| --- | --- |\n| 1   | 2   |",
                )),
            },
            Example {
                description: "Render a list (unordered by default).",
                example: "[0 1 2] | to md",
                result: Some(Value::test_string("* 0\n* 1\n* 2")),
            },
            Example {
                description: "Separate list into markdown tables.",
                example: "[ {foo: 1, bar: 2} {foo: 3, bar: 4} {foo: 5}] | to md --per-element",
                result: Some(Value::test_string(
                    "| foo | bar |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |\n\n| foo |\n| --- |\n| 5 |",
                )),
            },
            Example {
                description: "Center a column of a markdown table.",
                example: "[ {foo: 1, bar: 2} {foo: 3, bar: 4}] | to md --pretty --center [bar]",
                result: Some(Value::test_string(
                    "| foo | bar |\n| --- |:---:|\n| 1   |  2  |\n| 3   |  4  |",
                )),
            },
            Example {
                description: "Escape markdown special characters.",
                example: r#"[ {foo: "_1_", bar: "\# 2"} {foo: "[3]", bar: "4|5"}] | to md --escape-md"#,
                result: Some(Value::test_string(
                    "| foo | bar |\n| --- | --- |\n| \\_1\\_ | \\# 2 |\n| \\[3\\] | 4\\|5 |",
                )),
            },
            Example {
                description: "Escape html special characters.",
                example: r#"[ {a: p, b: "<p>Welcome to shell</p>"}] | to md --escape-html"#,
                result: Some(Value::test_string(
                    "| a | b |\n| --- | --- |\n| p | &lt;p&gt;Welcome to shell&lt;&#x2f;p&gt; |",
                )),
            },
            Example {
                description: "Render a list as an ordered markdown list.",
                example: "[one two three] | to md --list ordered",
                result: Some(Value::test_string("1. one\n2. two\n3. three")),
            },
            Example {
                description: "Render a list without markers.",
                example: "[one two three] | to md --list none",
                result: Some(Value::test_string("one\ntwo\nthree")),
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
        let head = call.head;

        let pretty = call.has_flag(engine_state, stack, "pretty")?;
        let per_element = call.has_flag(engine_state, stack, "per-element")?;
        let escape_md = call.has_flag(engine_state, stack, "escape-md")?;
        let escape_html = call.has_flag(engine_state, stack, "escape-html")?;
        let escape_both = call.has_flag(engine_state, stack, "escape-all")?;
        let center: Option<Vec<CellPath>> = call.get_flag(engine_state, stack, "center")?;
        let list_style_str: Option<Spanned<String>> = call.get_flag(engine_state, stack, "list")?;

        let list_style = match &list_style_str {
            Some(spanned) => {
                ListStyle::from_str(&spanned.item).ok_or_else(|| ShellError::InvalidValue {
                    valid: format!("one of {}", ListStyle::OPTIONS.join(", ")),
                    actual: spanned.item.clone(),
                    span: spanned.span,
                })?
            }
            None => ListStyle::default(),
        };

        let config = stack.get_config(engine_state);

        to_md(
            input,
            ToMdOptions {
                pretty,
                per_element,
                center,
                escape_md: escape_md || escape_both,
                escape_html: escape_html || escape_both,
                list_style,
            },
            &config,
            head,
        )
    }
}

fn to_md(
    mut input: PipelineData,
    options: ToMdOptions,
    config: &Config,
    head: Span,
) -> Result<PipelineData, ShellError> {
    // text/markdown became a valid mimetype with rfc7763
    let metadata = Some(
        input
            .take_metadata()
            .unwrap_or_default()
            .with_content_type(Some("text/markdown".into())),
    );

    // Collect input to check if it's a simple list (no records/tables)
    let values: Vec<Value> = input.into_iter().collect();

    // Check if input is a simple list (no records, lists, or tables)
    // Tables in shell can be represented as List of Records or List of Lists
    let is_simple_list = !values
        .iter()
        .any(|v| matches!(v, Value::Record { .. } | Value::List { .. }));

    // For simple lists, use list_style formatting
    if is_simple_list {
        let result = values
            .into_iter()
            .enumerate()
            .map(|(idx, val)| {
                format_list_item(
                    val,
                    idx,
                    options.list_style,
                    options.escape_md,
                    options.escape_html,
                    config,
                )
            })
            .collect::<Vec<String>>()
            .join("")
            .trim()
            .to_string();
        return Ok(Value::string(result, head).into_pipeline_data_with_metadata(metadata));
    }

    // With --per-element each run of rows with the same shape gets its own
    // table. Otherwise everything goes in one table, so the rows keep their
    // input order even when some of them are missing columns.
    if options.per_element || values.len() == 1 {
        let groups = if options.per_element {
            group_by(values, config)
        } else {
            vec![values]
        };
        return Ok(Value::string(
            groups
                .into_iter()
                .scan(0usize, |list_idx, mut group| {
                    if group.len() > 1 {
                        return Some(format!(
                            "{}\n\n",
                            table(
                                Value::list(group, head).into_pipeline_data(),
                                options.pretty,
                                &options.center,
                                options.escape_md,
                                options.escape_html,
                                config
                            )
                        ));
                    }
                    let val = group.pop().unwrap_or_else(|| Value::nothing(head));
                    Some(match &val {
                        Value::List { .. } => {
                            format!(
                                "{}\n\n",
                                table(
                                    val.into_pipeline_data(),
                                    options.pretty,
                                    &options.center,
                                    options.escape_md,
                                    options.escape_html,
                                    config
                                )
                            )
                        }
                        // For records, check if it's a special markdown element (h1, h2, etc.)
                        Value::Record { val: record, .. } => {
                            if is_special_markdown_record(record) {
                                // Special markdown elements use fragment() directly
                                fragment(
                                    val,
                                    options.pretty,
                                    &options.center,
                                    options.escape_md,
                                    options.escape_html,
                                    config,
                                )
                            } else {
                                // Regular records are rendered as tables
                                format!(
                                    "{}\n\n",
                                    fragment(
                                        val,
                                        options.pretty,
                                        &options.center,
                                        options.escape_md,
                                        options.escape_html,
                                        config
                                    )
                                )
                            }
                        }
                        _ => {
                            let result = format_list_item(
                                val,
                                *list_idx,
                                options.list_style,
                                options.escape_md,
                                options.escape_html,
                                config,
                            );
                            *list_idx += 1;
                            result
                        }
                    })
                })
                .collect::<Vec<String>>()
                .join("")
                .trim(),
            head,
        )
        .into_pipeline_data_with_metadata(metadata));
    }
    Ok(Value::string(
        table(
            Value::list(values, head).into_pipeline_data(),
            options.pretty,
            &options.center,
            options.escape_md,
            options.escape_html,
            config,
        ),
        head,
    )
    .into_pipeline_data_with_metadata(metadata))
}

/// Formats a single list item with the appropriate list marker based on list_style
fn format_list_item(
    input: Value,
    index: usize,
    list_style: ListStyle,
    escape_md: bool,
    escape_html: bool,
    config: &Config,
) -> String {
    let value_string = input.to_expanded_string("|", config);
    let escaped = escape_value(value_string, escape_md, escape_html, false);

    match list_style {
        ListStyle::Ordered => format!("{}. {}\n", index + 1, escaped),
        ListStyle::Unordered => format!("* {}\n", escaped),
        ListStyle::None => format!("{}\n", escaped),
    }
}

fn escape_markdown_characters(input: String, escape_md: bool, for_table: bool) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        // A line break would end the table row, so write it as an HTML break instead
        if for_table && matches!(ch, '\r' | '\n') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            output.push_str("<br>");
            continue;
        }

        let must_escape = match ch {
            '\\' => true,
            '|' if for_table => true,
            '`' | '*' | '_' | '{' | '}' | '[' | ']' | '(' | ')' | '<' | '>' | '#' | '+' | '-'
            | '.' | '!'
                if escape_md =>
            {
                true
            }
            _ => false,
        };

        if must_escape {
            output.push('\\');
        }
        output.push(ch);
    }
    output
}

/// Escapes a value string with optional HTML and Markdown escaping
fn escape_value(value: String, escape_md: bool, escape_html: bool, for_table: bool) -> String {
    escape_markdown_characters(
        if escape_html {
            v_htmlescape::escape_fmt(&value).to_string()
        } else {
            value
        },
        escape_md,
        for_table,
    )
}

fn fragment(
    input: Value,
    pretty: bool,
    center: &Option<Vec<CellPath>>,
    escape_md: bool,
    escape_html: bool,
    config: &Config,
) -> String {
    let mut out = String::new();

    if let Value::Record { val, .. } = &input {
        match val.get_index(0) {
            Some((header, data)) if is_special_markdown_record(val) => {
                // SAFETY: is_special_markdown_record already validated the header matches one of these
                let markup = match header.to_ascii_lowercase().as_ref() {
                    "h1" => "# ",
                    "h2" => "## ",
                    "h3" => "### ",
                    "blockquote" => "> ",
                    _ => "> ", // Fallback for any future validated headers not yet listed explicitly
                };

                let value_string = data.to_expanded_string("|", config);
                out.push_str(markup);
                out.push_str(&escape_value(value_string, escape_md, escape_html, false));
            }
            _ => {
                out = table(
                    input.into_pipeline_data(),
                    pretty,
                    center,
                    escape_md,
                    escape_html,
                    config,
                )
            }
        }
    } else {
        let value_string = input.to_expanded_string("|", config);
        out = escape_value(value_string, escape_md, escape_html, false);
    }

    out.push('\n');
    out
}

fn collect_headers(headers: &[String], escape_md: bool) -> (Vec<String>, Vec<usize>) {
    let mut escaped_headers: Vec<String> = Vec::new();
    let mut column_widths: Vec<usize> = Vec::new();

    if !headers.is_empty() && (headers.len() > 1 || !headers[0].is_empty()) {
        for header in headers {
            let escaped_header_string = escape_markdown_characters(
                v_htmlescape::escape_fmt(header).to_string(),
                escape_md,
                true,
            );
            column_widths.push(escaped_header_string.len());
            escaped_headers.push(escaped_header_string);
        }
    } else {
        column_widths = vec![0; headers.len()];
    }

    (escaped_headers, column_widths)
}

fn table(
    input: PipelineData,
    pretty: bool,
    center: &Option<Vec<CellPath>>,
    escape_md: bool,
    escape_html: bool,
    config: &Config,
) -> String {
    let vec_of_values = input
        .into_iter()
        .flat_map(|val| match val {
            Value::List { vals, .. } => vals.into_owned(),
            other => vec![other],
        })
        .collect::<Vec<Value>>();
    let mut headers = merge_descriptors(&vec_of_values);

    // Put an empty column name where it sits in its own record. Every column
    // before it in that record is already in `headers`, so the index fits.
    for value in &vec_of_values {
        if let Value::Record { val, .. } = value
            && !headers.contains(&String::new())
            && let Some(index) = val.columns().position(|column| column.is_empty())
        {
            headers.insert(index, String::new());
        }
    }

    let (escaped_headers, mut column_widths) = collect_headers(&headers, escape_md);

    let mut escaped_rows: Vec<Vec<String>> = Vec::new();

    for row in vec_of_values {
        let mut escaped_row: Vec<String> = Vec::new();
        let span = row.span();

        match row.to_owned() {
            Value::Record { val: row, .. } => {
                for i in 0..headers.len() {
                    let value_string = row
                        .get(&headers[i])
                        .cloned()
                        .unwrap_or_else(|| Value::nothing(span))
                        .to_expanded_string(", ", config);
                    let escaped_string = escape_markdown_characters(
                        if escape_html {
                            v_htmlescape::escape_fmt(&value_string).to_string()
                        } else {
                            value_string
                        },
                        escape_md,
                        true,
                    );

                    let new_column_width = escaped_string.len();
                    escaped_row.push(escaped_string);

                    if column_widths[i] < new_column_width {
                        column_widths[i] = new_column_width;
                    }
                    if column_widths[i] < 3 {
                        column_widths[i] = 3;
                    }
                }
            }
            p => {
                let value_string =
                    escape_value(p.to_abbreviated_string(config), escape_md, true, true);
                escaped_row.push(value_string);
            }
        }

        escaped_rows.push(escaped_row);
    }

    if (column_widths.is_empty() || column_widths.iter().all(|x| *x == 0))
        && escaped_rows.is_empty()
    {
        String::from("")
    } else {
        get_output_string(
            &headers,
            &escaped_headers,
            &escaped_rows,
            &column_widths,
            pretty,
            center,
        )
        .trim()
        .to_string()
    }
}

#[derive(PartialEq)]
enum GroupKey {
    Columns(Vec<String>),
    Value(String),
}

/// Splits the values into runs of consecutive values with the same shape,
/// so the output keeps the input order.
fn group_by(values: Vec<Value>, config: &Config) -> Vec<Vec<Value>> {
    let mut groups: Vec<Vec<Value>> = Vec::new();
    let mut last_key = None;
    for val in values {
        let key = match &val {
            Value::Record { val: record, .. } => {
                GroupKey::Columns(record.columns().cloned().collect())
            }
            other => GroupKey::Value(other.to_expanded_string(",", config)),
        };
        match groups.last_mut() {
            Some(group) if last_key.as_ref() == Some(&key) => group.push(val),
            _ => groups.push(vec![val]),
        }
        last_key = Some(key);
    }
    groups
}

fn get_output_string(
    column_names: &[String],
    headers: &[String],
    rows: &[Vec<String>],
    column_widths: &[usize],
    pretty: bool,
    center: &Option<Vec<CellPath>>,
) -> String {
    let mut output_string = String::new();

    let mut to_center: HashSet<String> = HashSet::new();
    if let Some(center_vec) = center.as_ref() {
        for cell_path in center_vec {
            if let Some(PathMember::String { val, .. }) = cell_path
                .members
                .iter()
                .find(|member| matches!(member, PathMember::String { .. }))
            {
                to_center.insert(val.clone());
            }
        }
    }

    if !headers.is_empty() {
        output_string.push('|');

        for i in 0..headers.len() {
            output_string.push(' ');
            if pretty {
                if center.is_some() && to_center.contains(&column_names[i]) {
                    output_string.push_str(&get_centered_string(
                        headers[i].clone(),
                        column_widths[i],
                        ' ',
                    ));
                } else {
                    output_string.push_str(&get_padded_string(
                        headers[i].clone(),
                        column_widths[i],
                        ' ',
                    ));
                }
            } else {
                output_string.push_str(&headers[i]);
            }

            output_string.push_str(" |");
        }

        output_string.push_str("\n|");

        for i in 0..headers.len() {
            let centered_column = center.is_some() && to_center.contains(&column_names[i]);
            let border_char = if centered_column { ':' } else { ' ' };
            if pretty {
                output_string.push(border_char);
                output_string.push_str(&get_padded_string(
                    String::from("-"),
                    column_widths[i],
                    '-',
                ));
                output_string.push(border_char);
            } else if centered_column {
                output_string.push_str(":---:");
            } else {
                output_string.push_str(" --- ");
            }

            output_string.push('|');
        }

        output_string.push('\n');
    }

    for row in rows {
        if !headers.is_empty() {
            output_string.push('|');
        }

        for i in 0..row.len() {
            if !headers.is_empty() {
                output_string.push(' ');
            }

            if pretty && column_widths.get(i).is_some() {
                if center.is_some() && to_center.contains(&column_names[i]) {
                    output_string.push_str(&get_centered_string(
                        row[i].clone(),
                        column_widths[i],
                        ' ',
                    ));
                } else {
                    output_string.push_str(&get_padded_string(
                        row[i].clone(),
                        column_widths[i],
                        ' ',
                    ));
                }
            } else {
                output_string.push_str(&row[i]);
            }

            if !headers.is_empty() {
                output_string.push_str(" |");
            }
        }

        output_string.push('\n');
    }

    output_string
}

fn get_centered_string(text: String, desired_length: usize, padding_character: char) -> String {
    let total_padding = if text.len() > desired_length {
        0
    } else {
        desired_length - text.len()
    };

    let repeat_left = total_padding / 2;
    let repeat_right = total_padding - repeat_left;

    format!(
        "{}{}{}",
        padding_character.to_string().repeat(repeat_left),
        text,
        padding_character.to_string().repeat(repeat_right)
    )
}

fn get_padded_string(text: String, desired_length: usize, padding_character: char) -> String {
    let repeat_length = if text.len() > desired_length {
        0
    } else {
        desired_length - text.len()
    };

    format!(
        "{}{}",
        text,
        padding_character.to_string().repeat(repeat_length)
    )
}

