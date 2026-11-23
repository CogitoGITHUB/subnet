use indexmap::IndexMap;
use console_engine::command_prelude::*;

#[derive(Clone)]
pub struct FromSsv;

const DEFAULT_MINIMUM_SPACES: usize = 2;

impl Command for FromSsv {
    fn name(&self) -> &str {
        "from ssv"
    }

    fn signature(&self) -> Signature {
        Signature::build("from ssv")
            .input_output_types(vec![(Type::String, Type::table())])
            .switch(
                "noheaders",
                "Don't treat the first row as column names.",
                Some('n'),
            )
            .switch("aligned-columns", "Assume columns are aligned.", Some('a'))
            .named(
                "minimum-spaces",
                SyntaxShape::Int,
                "The minimum spaces to separate columns.",
                Some('m'),
            )
            .category(Category::Formats)
    }

    fn description(&self) -> &str {
        "Parse text as space-separated values and create a table. The default minimum number of spaces counted as a separator is 2."
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "'FOO   BAR
1   2' | from ssv",
                description: "Converts ssv formatted string to table.",
                result: Some(Value::test_list(vec![Value::test_record(record! {
                    "FOO" => Value::test_string("1"),
                    "BAR" => Value::test_string("2"),
                })])),
            },
            Example {
                example: "'FOO   BAR
1   2' | from ssv --noheaders",
                description: "Converts ssv formatted string to table but not treating the first row as column names.",
                result: Some(Value::test_list(vec![
                    Value::test_record(record! {
                        "column0" => Value::test_string("FOO"),
                        "column1" => Value::test_string("BAR"),
                    }),
                    Value::test_record(record! {
                        "column0" => Value::test_string("1"),
                        "column1" => Value::test_string("2"),
                    }),
                ])),
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
        from_ssv(engine_state, stack, call, input)
    }
}

enum HeaderOptions<'a> {
    WithHeaders(&'a str),
    WithoutHeaders,
}

fn parse_aligned_columns<'a>(
    lines: impl Iterator<Item = &'a str>,
    headers: HeaderOptions,
    separator: &str,
) -> Vec<Vec<(String, String)>> {
    fn construct<'a>(
        lines: impl Iterator<Item = &'a str>,
        headers: Vec<(String, usize)>,
    ) -> Vec<Vec<(String, String)>> {
        lines
            .map(|l| {
                headers
                    .iter()
                    .enumerate()
                    .map(|(i, (header_name, start_position))| {
                        let char_index_start = match l.char_indices().nth(*start_position) {
                            Some(idx) => idx.0,
                            None => *start_position,
                        };
                        let val = match headers.get(i + 1) {
                            Some((_, end)) => {
                                if *end < l.len() {
                                    let char_index_end = match l.char_indices().nth(*end) {
                                        Some(idx) => idx.0,
                                        None => *end,
                                    };
                                    l.get(char_index_start..char_index_end)
                                } else {
                                    l.get(char_index_start..)
                                }
                            }
                            None => l.get(char_index_start..),
                        }
                        .unwrap_or("")
                        .trim()
                        .into();
                        (header_name.clone(), val)
                    })
                    .collect()
            })
            .collect()
    }

    let find_indices = |line: &str| {
        let values = line
            .split(&separator)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        values
            .fold(
                (0, vec![]),
                |(current_pos, mut indices), value| match line[current_pos..].find(value) {
                    None => (current_pos, indices),
                    Some(index) => {
                        let absolute_index = current_pos + index;
                        indices.push(absolute_index);
                        (absolute_index + value.len(), indices)
                    }
                },
            )
            .1
    };

    let parse_with_headers = |lines, headers_raw: &str| {
        let indices = find_indices(headers_raw);
        let headers = headers_raw
            .split(&separator)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .zip(indices);

        let columns = headers.collect::<Vec<(String, usize)>>();

        construct(lines, columns)
    };

    let parse_without_headers = |ls: Vec<&str>| {
        let mut indices = ls
            .iter()
            .flat_map(|s| find_indices(s))
            .collect::<Vec<usize>>();

        indices.sort_unstable();
        indices.dedup();

        let headers: Vec<(String, usize)> = indices
            .iter()
            .enumerate()
            .map(|(i, position)| (format!("column{i}"), *position))
            .collect();

        construct(ls.iter().map(|s| s.to_owned()), headers)
    };

    match headers {
        HeaderOptions::WithHeaders(headers_raw) => parse_with_headers(lines, headers_raw),
        HeaderOptions::WithoutHeaders => parse_without_headers(lines.collect()),
    }
}

fn parse_separated_columns<'a>(
    lines: impl Iterator<Item = &'a str>,
    headers: HeaderOptions,
    separator: &str,
) -> Vec<Vec<(String, String)>> {
    fn collect<'a>(
        headers: Vec<String>,
        rows: impl Iterator<Item = &'a str>,
        separator: &str,
    ) -> Vec<Vec<(String, String)>> {
        rows.map(|r| {
            headers
                .iter()
                .zip(r.split(separator).map(str::trim).filter(|s| !s.is_empty()))
                .map(|(a, b)| (a.to_owned(), b.to_owned()))
                .collect()
        })
        .collect()
    }

    let parse_with_headers = |lines, headers_raw: &str| {
        let headers = headers_raw
            .split(&separator)
            .map(str::trim)
            .map(str::to_owned)
            .filter(|s| !s.is_empty())
            .collect();
        collect(headers, lines, separator)
    };

    let parse_without_headers = |ls: Vec<&str>| {
        let num_columns = ls.iter().map(|r| r.len()).max().unwrap_or(0);

        let headers = (0..=num_columns)
            .map(|i| format!("column{i}"))
            .collect::<Vec<String>>();
        collect(headers, ls.into_iter(), separator)
    };

    match headers {
        HeaderOptions::WithHeaders(headers_raw) => parse_with_headers(lines, headers_raw),
        HeaderOptions::WithoutHeaders => parse_without_headers(lines.collect()),
    }
}

fn string_to_table(
    s: &str,
    noheaders: bool,
    aligned_columns: bool,
    split_at: usize,
) -> Vec<Vec<(String, String)>> {
    let mut lines = s
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'));
    let separator = " ".repeat(std::cmp::max(split_at, 1));

    let (ls, header_options) = if noheaders {
        (lines, HeaderOptions::WithoutHeaders)
    } else {
        match lines.next() {
            Some(header) => (lines, HeaderOptions::WithHeaders(header)),
            None => return vec![],
        }
    };

    let f = if aligned_columns {
        parse_aligned_columns
    } else {
        parse_separated_columns
    };

    f(ls, header_options, &separator)
}

fn from_ssv_string_to_value(
    s: &str,
    noheaders: bool,
    aligned_columns: bool,
    split_at: usize,
    span: Span,
) -> Value {
    let rows = string_to_table(s, noheaders, aligned_columns, split_at)
        .into_iter()
        .map(|row| {
            let mut dict = IndexMap::new();
            for (col, entry) in row {
                dict.insert(col, Value::string(entry, span));
            }
            Value::record(dict.into_iter().collect(), span)
        })
        .collect();

    Value::list(rows, span)
}

fn from_ssv(
    engine_state: &EngineState,
    stack: &mut Stack,
    call: &Call,
    input: PipelineData,
) -> Result<PipelineData, ShellError> {
    let name = call.head;

    let noheaders = call.has_flag(engine_state, stack, "noheaders")?;
    let aligned_columns = call.has_flag(engine_state, stack, "aligned-columns")?;
    let minimum_spaces: Option<Spanned<usize>> =
        call.get_flag(engine_state, stack, "minimum-spaces")?;

    let (concat_string, _span, metadata) = input.collect_string_strict(name)?;
    let split_at = match minimum_spaces {
        Some(number) => number.item,
        None => DEFAULT_MINIMUM_SPACES,
    };

    Ok(
        from_ssv_string_to_value(&concat_string, noheaders, aligned_columns, split_at, name)
            .into_pipeline_data_with_metadata(metadata),
    )
}

