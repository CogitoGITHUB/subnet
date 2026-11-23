use console_utils::escape_quote_string;

fn string_should_be_quoted(input: &str) -> bool {
    input.is_empty()
        || input.starts_with('$')
        || input.chars().any(|c| {
            c.is_whitespace()
                || c == '('
                || c == '['
                || c == '{'
                || c == '}'
                || c == '\''
                || c == '`'
                || c == '"'
                || c == '\\'
                || c == ';'
                || c == '|'
                // `#` starts a comment when it begins a token (or after whitespace).
                // Without quoting, `nu script.nu #000000` becomes `main #000000` and
                // the argument is stripped.
                || c == '#'
        })
}

// Escape rules:
// input argument is not a flag, does not start with $ and doesn't contain special characters, it is passed as it is (foo -> foo)
// input argument is not a flag and either starts with $ or contains special characters, quotes are added, " and \ are escaped (two \words -> "two \\words")
// input argument is a flag without =, it's passed as it is (--foo -> --foo)
// input argument is a flag with =, the first two points apply to the value (--foo=bar -> --foo=bar; --foo=bar' -> --foo="bar'")
//
// special characters are white space, (, [, {, }, ', `, ", \, ;, |, and #
pub fn escape_for_script_arg(input: &str) -> String {
    // handle for flag, maybe we need to escape the value.
    if input.starts_with("--") {
        if let Some((arg_name, arg_val)) = input.split_once('=') {
            // only want to escape arg_val.
            let arg_val = if string_should_be_quoted(arg_val) {
                escape_quote_string(arg_val)
            } else {
                arg_val.into()
            };

            return format!("{arg_name}={arg_val}");
        } else {
            return input.into();
        }
    }
    if string_should_be_quoted(input) {
        escape_quote_string(input)
    } else {
        input.into()
    }
}

