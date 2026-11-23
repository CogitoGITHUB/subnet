//! Reads a fish completion script: the top-level `complete` statements for the tool, and the
//! `set` statements whose variables they use.
//!
//! Which subcommand a statement belongs to comes from its `-n` conditions. fish's own helpers
//! and the per-tool helpers its scripts define follow a naming pattern, such as
//! `__fish_seen_subcommand_from add`, `__fish_git_using_command add` and
//! `__fish_git_needs_command`. A statement with a condition that doesn't fit one of those
//! patterns is skipped, since it may belong to any subcommand.
//!
//! Values that fish computes, such as `-a '(__fish_git_remotes)'`, become shell completers
//! when they translate (see [`completers::translate_fish`]). A statement that completes values
//! without naming a flag or listing subcommands completes the command's arguments.

use super::completers::{self, Dynamic, FishScript, FishWord, Shared};
use super::spec::{CommandSpec, Completions, FlagSpec, is_command_word, spec_at};
use console_protocol::SyntaxShape;
use std::collections::HashMap;

/// Reads the specs of `tool` from its fish completion script, starting at the subcommand
/// `path` (empty for the tool itself).
pub(super) fn parse(script: &str, tool: &str, path: &[String]) -> Vec<CommandSpec> {
    let mut variables: HashMap<String, Vec<String>> = HashMap::new();
    let mut specs = vec![CommandSpec::new(vec![])];
    let mut depth = 0usize;
    let statements = statements(script);
    let functions = functions(&statements);
    let fish = FishScript {
        tool,
        functions: &functions,
        words: tokens,
    };

    for statement in statements {
        let words = words(&statement);
        let Some(first) = words.first() else {
            continue;
        };
        match first.as_str() {
            _ if BLOCKS.contains(&first.as_str()) => depth += 1,
            "end" => depth = depth.saturating_sub(1),
            "set" if depth == 0 => {
                let mut args = words[1..].iter().skip_while(|word| word.starts_with('-'));
                if let Some(name) = args.next() {
                    variables.insert(name.clone(), args.cloned().collect());
                }
            }
            "complete" if depth == 0 => {
                let complete = Complete::parse(&words[1..], &variables);
                if complete.command.as_deref() == Some(tool) {
                    complete.add_to(&mut specs, &variables, &fish);
                }
            }
            _ => {}
        }
    }

    specs.retain(|spec| spec.path.starts_with(path));
    specs
}

/// The keywords that start a block, which `end` closes.
const BLOCKS: &[&str] = &["function", "if", "for", "while", "begin", "switch"];

/// The options of one `complete` statement that matter for a signature.
#[derive(Default)]
struct Complete {
    command: Option<String>,
    shorts: Vec<String>,
    longs: Vec<String>,
    /// Single-dash long options (`-o name`), which an extern can't declare.
    olds: Vec<String>,
    arguments: Vec<String>,
    /// The insides of the command substitutions among the arguments, such as
    /// `__fish_git_remotes` for `-a '(__fish_git_remotes)'`.
    substitutions: Vec<String>,
    description: String,
    requires_value: bool,
    conditions: Vec<String>,
    /// `-e` (erase) and `-C` (complete a command line) don't add completions.
    ignored: bool,
}

impl Complete {
    /// Reads the options like fish's `complete` builtin does: short options may be grouped
    /// (`-xa args`) and long options may use `=` (`--long-option=name`).
    fn parse(args: &[String], variables: &HashMap<String, Vec<String>>) -> Self {
        let mut complete = Complete::default();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            let (option, mut value) = if let Some(long) = arg.strip_prefix("--") {
                match long.split_once('=') {
                    Some((name, value)) => (long_to_short(name), Some(value.to_string())),
                    None => (long_to_short(long), None),
                }
            } else if let Some(group) = arg.strip_prefix('-').filter(|group| !group.is_empty()) {
                // Options without a value, then at most one that takes the rest or the next word.
                let mut chars = group.char_indices();
                let mut option = None;
                for (index, c) in chars.by_ref() {
                    if takes_value(c) {
                        let rest = &group[index + c.len_utf8()..];
                        option = Some((c, (!rest.is_empty()).then(|| rest.to_string())));
                        break;
                    }
                    complete.set_switch(c);
                }
                match option {
                    Some((c, value)) => (Some(c), value),
                    None => continue,
                }
            } else {
                // `complete git ...` names the command without `-c`.
                if complete.command.is_none() {
                    complete.command = Some(arg.clone());
                }
                continue;
            };

            let Some(option) = option else {
                continue;
            };
            if !takes_value(option) {
                complete.set_switch(option);
                continue;
            }
            if value.is_none() {
                value = args.next().cloned();
            }
            let Some(value) = value else {
                continue;
            };
            match option {
                'c' => complete.command = Some(value),
                's' => complete.shorts.push(value),
                'l' => complete.longs.push(value),
                'o' => complete.olds.push(value),
                'a' => {
                    let (values, substitutions) = arguments(&value, variables);
                    complete.arguments.extend(values);
                    complete.substitutions.extend(substitutions);
                }
                'd' => complete.description = value,
                'n' => complete.conditions.push(value),
                _ => {}
            }
        }
        complete
    }

    fn set_switch(&mut self, option: char) {
        match option {
            'r' | 'x' => self.requires_value = true,
            'e' | 'C' => self.ignored = true,
            _ => {}
        }
    }

    /// Adds the flags this statement declares, the subcommands it lists or the values of the
    /// command's arguments, to `specs`.
    fn add_to(
        self,
        specs: &mut Vec<CommandSpec>,
        variables: &HashMap<String, Vec<String>>,
        fish: &FishScript,
    ) {
        if self.ignored {
            return;
        }
        let Some(Position {
            paths,
            lists_subcommands,
        }) = Position::of(&self.conditions, variables)
        else {
            return;
        };

        let declares_flags =
            !self.shorts.is_empty() || !self.longs.is_empty() || !self.olds.is_empty();
        let translated = self
            .substitutions
            .iter()
            .filter_map(|substitution| completers::translate_fish(substitution, fish))
            .collect::<Vec<_>>();
        let dynamic = translated
            .iter()
            .filter_map(|shared| match shared {
                Shared::Dynamic(dynamic) => Some(dynamic.clone()),
                Shared::Directory | Shared::Files => None,
            })
            .collect::<Vec<Dynamic>>();
        for path in paths {
            if declares_flags {
                let spec = spec_at(specs, &path);
                for old in &self.olds {
                    spec.skip(format!("-{old}"));
                }
                // The `-a` words of a flag that takes a value are the values it accepts.
                let (arg, completions) = match self.requires_value {
                    true => (
                        Some(match translated.contains(&Shared::Directory) {
                            true => SyntaxShape::Directory,
                            false => SyntaxShape::String,
                        }),
                        Completions {
                            values: self.arguments.clone(),
                            dynamic: dynamic.clone(),
                        },
                    ),
                    false => (None, Completions::default()),
                };
                spec.add_names(
                    self.longs.clone(),
                    self.shorts.clone(),
                    FlagSpec {
                        arg,
                        completions,
                        description: self.description.clone(),
                        ..FlagSpec::default()
                    },
                );
            } else if !lists_subcommands {
                spec_at(specs, &path).args.extend(Completions {
                    values: self.arguments.clone(),
                    dynamic: dynamic.clone(),
                });
            } else {
                for word in self.arguments.iter().filter(|word| is_command_word(word)) {
                    let mut sub_path = path.clone();
                    sub_path.push(word.clone());
                    let spec = spec_at(specs, &sub_path);
                    if spec.description.is_empty() {
                        spec.description.clone_from(&self.description);
                    }
                }
            }
        }
    }
}

/// The long names of the `complete` options that matter here.
fn long_to_short(name: &str) -> Option<char> {
    Some(match name {
        "command" => 'c',
        "short-option" => 's',
        "long-option" => 'l',
        "old-option" => 'o',
        "arguments" => 'a',
        "description" => 'd',
        "condition" => 'n',
        "require-parameter" => 'r',
        "exclusive" => 'x',
        "erase" => 'e',
        "do-complete" => 'C',
        "path" => 'p',
        "wraps" => 'w',
        _ => return None,
    })
}

/// Whether the `complete` option takes a value.
fn takes_value(option: char) -> bool {
    matches!(option, 'c' | 'p' | 's' | 'l' | 'o' | 'a' | 'd' | 'w' | 'n')
}

/// Where a statement applies, from its conditions.
struct Position {
    /// The subcommand paths, empty for the tool itself.
    paths: Vec<Vec<String>>,
    /// Whether the statement completes the subcommand name itself, so its `-a` words are
    /// subcommands rather than values.
    lists_subcommands: bool,
}

impl Position {
    /// Returns `None` when a condition doesn't fit a known pattern.
    fn of(conditions: &[String], variables: &HashMap<String, Vec<String>>) -> Option<Self> {
        let mut paths = vec![vec![]];
        let mut lists_subcommands = false;
        // `a && b`, `a; and b` and several `-n` options all mean that every part holds.
        let parts = conditions
            .iter()
            .flat_map(|condition| condition.split(['&', ';']))
            .map(|part| part.trim().trim_start_matches("and ").trim())
            .filter(|part| !part.is_empty());

        for part in parts {
            let mut words = expand(part, variables);
            let mut negated = words.first().is_some_and(|word| word == "not");
            if negated {
                words.remove(0);
            }
            let Some(function) = words.first() else {
                continue;
            };
            let args = &words[1..];
            let sub_pattern = SUBCOMMAND_SUFFIXES
                .iter()
                .find_map(|suffix| function.strip_suffix(suffix));
            let is_sub_pattern = sub_pattern.is_some();
            // `__fish_git_not_using_command add` holds outside `add`, like `not ...`.
            negated |= sub_pattern.is_some_and(|prefix| prefix.ends_with("not_"));

            if negated {
                // `not __fish_seen_subcommand_from add rm` means no subcommand is typed yet.
                lists_subcommands |= is_sub_pattern;
            } else if args.is_empty()
                && TOP_LEVEL_SUFFIXES
                    .iter()
                    .any(|suffix| function.ends_with(suffix))
            {
                lists_subcommands = true;
            } else if is_sub_pattern
                && !args.is_empty()
                && args.iter().all(|arg| is_command_word(arg))
            {
                paths = paths
                    .iter()
                    .flat_map(|path| {
                        args.iter().map(move |arg| {
                            let mut path = path.clone();
                            path.push(arg.clone());
                            path
                        })
                    })
                    .collect();
            } else {
                return None;
            }
        }
        Some(Self {
            paths,
            lists_subcommands,
        })
    }
}

/// Names of conditions that hold when one of their arguments is the subcommand.
const SUBCOMMAND_SUFFIXES: &[&str] = &[
    "subcommand_from",
    "using_command",
    "using_subcommand",
    "seen_subcommand",
    "is_subcommand",
];

/// Names of conditions that hold while no subcommand is typed yet.
const TOP_LEVEL_SUFFIXES: &[&str] = &[
    "use_subcommand",
    "needs_command",
    "needs_subcommand",
    "need_command",
    "no_subcommand",
    "no_command",
    "is_first_arg",
    "is_first_token",
];

/// Splits the text of `-a` into the values it lists and the insides of its command
/// substitutions. `$name` words are replaced with the values of that variable, and other words
/// that expand are dropped.
fn arguments(text: &str, variables: &HashMap<String, Vec<String>>) -> (Vec<String>, Vec<String>) {
    let mut values = vec![];
    let mut substitutions = vec![];
    for word in tokens(text) {
        if let Some(name) = word.text.strip_prefix('$') {
            values.extend(variable(name, variables));
        } else if let Some(inside) = word
            .text
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .filter(|_| word.expands)
        {
            substitutions.push(inside.to_string());
        } else if !word.expands && !candidate(&word.text).is_empty() {
            values.push(candidate(&word.text).to_string());
        }
    }
    (values, substitutions)
}

/// A candidate without the description fish allows after a tab: `add\tAdd files`. The tab is
/// a tab character when the word was unquoted, and `\t` when it was quoted.
fn candidate(word: &str) -> &str {
    let word = word.split("\\t").next().unwrap_or_default();
    word.split('\t').next().unwrap_or_default()
}

/// The values of the variable `$name`. Command substitutions that `set` recorded are dropped,
/// as are variables the script doesn't set.
fn variable(name: &str, variables: &HashMap<String, Vec<String>>) -> Vec<String> {
    variables
        .get(name)
        .into_iter()
        .flatten()
        .filter(|value| !value.contains('('))
        .map(|value| candidate(value).to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

/// The helper functions a script defines at its top level, as their statements.
fn functions(statements: &[String]) -> HashMap<String, Vec<String>> {
    let mut functions = HashMap::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut depth = 0usize;
    for statement in statements {
        let words = words(statement);
        let Some(first) = words.first() else {
            continue;
        };
        match first.as_str() {
            "function" if depth == 0 => {
                current = words.get(1).map(|name| (name.clone(), vec![]));
                depth += 1;
                continue;
            }
            _ if BLOCKS.contains(&first.as_str()) => depth += 1,
            "end" => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some((name, body)) = current.take() {
                        functions.insert(name, body);
                    }
                    continue;
                }
            }
            _ => {}
        }
        if let Some((_, body)) = &mut current {
            body.push(statement.clone());
        }
    }
    functions
}

/// Splits `text` into words like fish does and replaces each `$name` word with the values of
/// that variable. Command substitutions are dropped, as are variables the script doesn't set.
fn expand(text: &str, variables: &HashMap<String, Vec<String>>) -> Vec<String> {
    words(text)
        .into_iter()
        .flat_map(|word| match word.strip_prefix('$') {
            Some(name) => variable(name, variables),
            None if word.contains('(') => vec![],
            None => vec![candidate(&word).to_string()],
        })
        .collect()
}

/// The statements of the script, with `\` line continuations joined. A line holds several
/// statements when they are separated by `;`, as in `function f; ...; end`.
fn statements(script: &str) -> Vec<String> {
    let mut lines = vec![];
    let mut current = String::new();
    for line in script.lines() {
        let line = line.trim();
        let trailing_backslashes = line.len() - line.trim_end_matches('\\').len();
        if trailing_backslashes % 2 == 1 {
            current.push_str(&line[..line.len() - 1]);
            current.push(' ');
        } else {
            current.push_str(line);
            lines.push(std::mem::take(&mut current));
        }
    }
    lines.push(current);
    lines
        .iter()
        .flat_map(|line| split_statements(line))
        .collect()
}

/// Splits a line at each `;` that ends a statement: outside quotes and command substitutions,
/// and before a comment.
fn split_statements(line: &str) -> Vec<String> {
    let mut statements = vec![];
    let mut start = 0;
    let mut quote = None;
    let mut parens = 0usize;
    let mut word_start = true;
    let mut chars = line.char_indices();
    while let Some((index, c)) = chars.next() {
        match (quote, c) {
            (_, '\\') => {
                chars.next();
            }
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, '(') => parens += 1,
            (None, ')') => parens = parens.saturating_sub(1),
            (None, '#') if word_start && parens == 0 => break,
            (None, ';') if parens == 0 => {
                statements.push(line[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
        word_start = c.is_whitespace() || c == ';';
    }
    statements.push(line[start..].trim().to_string());
    statements
}

/// Splits a statement into words the way fish does, removing quotes and escapes. `$name` and
/// `(command)` are kept as written, and a `#` or `;` outside quotes ends the statement.
fn words(statement: &str) -> Vec<String> {
    tokens(statement)
        .into_iter()
        .map(|word| word.text)
        .collect()
}

/// [`words`], each marked with whether fish expands it. A `|` between commands is a word of
/// its own.
fn tokens(statement: &str) -> Vec<FishWord> {
    let mut words = vec![];
    let mut word = String::new();
    let mut in_word = false;
    let mut expands = false;
    let mut parens = 0usize;
    let mut chars = statement.chars();
    let finish = |words: &mut Vec<FishWord>, word: &mut String, expands: &mut bool| {
        words.push(FishWord {
            text: std::mem::take(word),
            expands: std::mem::take(expands),
        });
    };

    while let Some(c) = chars.next() {
        match c {
            // A command substitution is kept as written, quotes and all, to be read again.
            '\'' | '"' if parens > 0 => {
                word.push(c);
                while let Some(q) = chars.next() {
                    word.push(q);
                    match q {
                        _ if q == c => break,
                        '\\' => word.extend(chars.next()),
                        _ => {}
                    }
                }
            }
            '\'' | '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        _ if q == c => break,
                        '\\' => match chars.next() {
                            Some(e) if e == c || e == '\\' || (c == '"' && e == '$') => {
                                word.push(e)
                            }
                            Some(e) => {
                                word.push('\\');
                                word.push(e);
                            }
                            None => word.push('\\'),
                        },
                        // fish expands variables inside double quotes.
                        '$' if c == '"' => {
                            expands = true;
                            word.push(q);
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(e @ (' ' | '\'' | '"' | '\\' | '$' | '(' | ')' | ';' | '#' | '|')) => {
                        word.push(e)
                    }
                    Some('t') => word.push('\t'),
                    Some('n') => word.push('\n'),
                    Some(e) => {
                        word.push('\\');
                        word.push(e);
                    }
                    None => {}
                }
            }
            '(' => {
                in_word = true;
                expands = true;
                parens += 1;
                word.push(c);
            }
            ')' => {
                parens = parens.saturating_sub(1);
                word.push(c);
            }
            _ if parens > 0 => word.push(c),
            '#' if !in_word => break,
            ';' => break,
            '|' => {
                if in_word {
                    finish(&mut words, &mut word, &mut expands);
                    in_word = false;
                }
                word.push(c);
                expands = true;
                finish(&mut words, &mut word, &mut expands);
            }
            _ if c.is_whitespace() => {
                if in_word {
                    finish(&mut words, &mut word, &mut expands);
                    in_word = false;
                }
            }
            _ => {
                expands |= matches!(c, '$' | '*' | '?' | '{') || (c == '~' && !in_word);
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        finish(&mut words, &mut word, &mut expands);
    }
    words
}

