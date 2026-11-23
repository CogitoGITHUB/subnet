use super::utils;
use console_engine::command_prelude::*;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Difference;

impl Command for Difference {
    fn name(&self) -> &str {
        "difference"
    }

    fn signature(&self) -> Signature {
        Signature::build("difference")
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
                "The other list to subtract from the input.",
            )
            .category(Category::Filters)
    }

    fn description(&self) -> &str {
        "Returns a list of unique elements in the input that are not present in the other list."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["subtract", "minus", "exclude", "remove"]
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

        let other_vals = utils::extract_other_list(engine_state, stack, call, head)?;

        let other_set: HashSet<String> = other_vals
            .iter()
            .map(|v| utils::value_to_key(engine_state, v, head))
            .collect::<Result<HashSet<_>, _>>()?;

        let signals = engine_state.signals().clone();
        let mut seen: HashSet<String> = HashSet::new();
        let mut result = Vec::new();

        for val in input {
            signals.check(&head)?;
            let key = utils::value_to_key(engine_state, &val, head)?;
            if !other_set.contains(&key) && seen.insert(key) {
                result.push(val);
            }
        }

        Ok(PipelineData::Value(Value::list(result, head), metadata))
    }

    fn examples(&self) -> Vec<Example<'static>> {
        vec![
            Example {
                example: "[1 2 3 4] | difference [3 4 5 6]",
                description: "Return the difference of two lists",
                result: Some(Value::test_list(vec![
                    Value::test_int(1),
                    Value::test_int(2),
                ])),
            },
            Example {
                example: "[1 1 2 3] | difference [2 3]",
                description: "Difference with duplicates in input",
                result: Some(Value::test_list(vec![Value::test_int(1)])),
            },
            Example {
                example: "[{a:1} {a:2} {a:3}] | difference [{a:2} {a:4}]",
                description: "Difference of two tables",
                result: Some(Value::test_list(vec![
                    Value::test_record(record!("a" => Value::test_int(1))),
                    Value::test_record(record!("a" => Value::test_int(3))),
                ])),
            },
        ]
    }
}

