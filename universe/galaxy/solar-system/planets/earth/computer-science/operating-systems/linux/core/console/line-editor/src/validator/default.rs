use crate::{ValidationResult, Validator};

/// A default validator which checks for mismatched quotes and brackets
pub struct DefaultValidator;

impl Validator for DefaultValidator {
    fn validate(&self, line: &str) -> ValidationResult {
        if line.split('"').count().is_multiple_of(2) || incomplete_brackets(line) {
            ValidationResult::Incomplete
        } else {
            ValidationResult::Complete
        }
    }
}

fn incomplete_brackets(line: &str) -> bool {
    let mut balance: Vec<char> = Vec::new();

    for c in line.chars() {
        if c == '{' {
            balance.push('}');
        } else if c == '[' {
            balance.push(']');
        } else if c == '(' {
            balance.push(')');
        } else if ['}', ']', ')'].contains(&c) {
            if let Some(last) = balance.last() {
                if last == &c {
                    balance.pop();
                }
            }
        }
    }

    !balance.is_empty()
}

