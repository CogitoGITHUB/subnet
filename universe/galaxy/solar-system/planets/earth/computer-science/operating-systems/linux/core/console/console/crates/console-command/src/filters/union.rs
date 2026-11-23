use super::utils;
use console_engine::command_prelude::*;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Union;

impl Command for Union {
    fn name(&self) -> &str {
        "union"
    }

    fn signature(&self) -> Signature {
        Signature::build("union")
            .input_output_types(vec![
                (
                    Type::List(Box::new(Type::Any)),
                    Type::List(Box::new(Type::Any)),
                ),
                (Type::table(), Type::table()),
            ])
            .required(
                "other",
                SyntaxShape::List(Box::new(SyntaxShape::Any)),
                "The other list to union with.",
            )
            .category(Category::Filters)
    }

    fn description(&self) -> &str {
        "Returns a list of unique elements from both the input and the provided list."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["merge", "deduplicate", "combine"]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        mut input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let head = call.head;
        let metadata = input.take_metadata();

        let input_vals: Vec<Value> = input.into_iter().collect();
        let other_vals = utils::extract_other_list(engine_state, stack, call, head)?;

        let signals = engine_state.signals().clone();
        let total_capacity = input_vals.len() + other_vals.len();
        let mut seen: HashSet<String> = HashSet::with_capacity(total_capacity);
        let mut result = Vec::with_capacity(total_capacity);

        for val in input_vals {
            signals.check(&head)?;
            if seen.insert(utils::value_to_key(engine_state, &val, head)?) {
                result.push(val);
            }
        }

        for val in other_vals {
            signals.check(&head)?;
            if seen.insert(utils::value_to_key(engine_state, &val, head)?) {
                result.push(val);
            }
        }

        Ok(PipelineData::Value(Value::list(result, head), metadata))
    }

    fn examples(&self) -> Vec<Example<'static>> {
        vec![
            Example {
                example: "[1 2 3 4] | union [3 4 5 6]",
                description: "Return the union of two lists",
                result: Some(Value::test_list(vec![
                    Value::test_int(1),
                    Value::test_int(2),
                    Value::test_int(3),
                    Value::test_int(4),
                    Value::test_int(5),
                    Value::test_int(6),
                ])),
            },
            Example {
                example: "[1 1 2 3] | union [2 3 4]",
                description: "Union with duplicates in input",
                result: Some(Value::test_list(vec![
                    Value::test_int(1),
                    Value::test_int(2),
                    Value::test_int(3),
                    Value::test_int(4),
                ])),
            },
            Example {
                example: "[{a:1} {a:2}] | union [{a:2} {a:3}]",
                description: "Union of two tables (dedup rows)",
                result: Some(Value::test_list(vec![
                    Value::test_record(record!("a" => Value::test_int(1))),
                    Value::test_record(record!("a" => Value::test_int(2))),
                    Value::test_record(record!("a" => Value::test_int(3))),
                ])),
            },
        ]
    }
}

