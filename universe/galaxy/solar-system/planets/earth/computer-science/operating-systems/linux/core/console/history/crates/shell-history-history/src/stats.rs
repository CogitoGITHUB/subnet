use std::collections::{HashMap, HashSet};

use crossterm::style::{Color, ResetColor, SetAttribute, SetForegroundColor};
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use shell_history_client::{history::History, settings::Settings};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stats {
    pub total_commands: usize,
    pub unique_commands: usize,
    pub top: Vec<(Vec<String>, usize)>,
}

fn first_non_whitespace(s: &str) -> Option<usize> {
    s.char_indices()
        // find the first non whitespace char
        .find(|(_, c)| !c.is_ascii_whitespace())
        // return the index of that char
        .map(|(i, _)| i)
}

fn first_whitespace(s: &str) -> usize {
    s.char_indices()
        // find the first whitespace char
        .find(|(_, c)| c.is_ascii_whitespace())
        // return the index of that char, (or the max length of the string)
        .map_or(s.len(), |(i, _)| i)
}

fn interesting_command<'a>(settings: &Settings, mut command: &'a str) -> &'a str {
    // Sort by length so that we match the longest prefix first
    let mut common_prefix = settings.stats.common_prefix.clone();
    common_prefix.sort_by_key(|b| std::cmp::Reverse(b.len()));

    // Trim off the common prefix, if it exists
    for p in &common_prefix {
        if command.starts_with(p) {
            let i = p.len();
            let prefix = &command[..i];
            command = command[i..].trim_start();
            if command.is_empty() {
                // no commands following, just use the prefix
                return prefix;
            }
            break;
        }
    }

    // Sort the common_subcommands by length so that we match the longest subcommand first
    let mut common_subcommands = settings.stats.common_subcommands.clone();
    common_subcommands.sort_by_key(|b| std::cmp::Reverse(b.len()));

    // Check for a common subcommand
    for p in &common_subcommands {
        if command.starts_with(p) {
            // if the subcommand is the same length as the command, then we just use the subcommand
            if p.len() == command.len() {
                return command;
            }
            // otherwise we need to use the subcommand + the next word
            let non_whitespace = first_non_whitespace(&command[p.len()..]).unwrap_or(0);
            let j =
                p.len() + non_whitespace + first_whitespace(&command[p.len() + non_whitespace..]);
            return &command[..j];
        }
    }
    // Return the first word if there is no subcommand
    &command[..first_whitespace(command)]
}

fn split_at_pipe(command: &str) -> Vec<&str> {
    let mut result = vec![];
    let mut quoted = false;
    let mut start = 0;
    let mut graphemes = UnicodeSegmentation::grapheme_indices(command, true);

    while let Some((i, c)) = graphemes.next() {
        let current = i;
        match c {
            "\"" if command[start..current] != *"\"" => {
                quoted = !quoted;
            }
            "'" if command[start..current] != *"'" => {
                quoted = !quoted;
            }
            "\\" if graphemes.next().is_some() => {}
            "|" if !quoted => {
                if current > start && command[start..].starts_with('|') {
                    start += 1;
                }
                result.push(&command[start..current]);
                start = current;
            }
            _ => {}
        }
    }
    if command[start..].starts_with('|') {
        start += 1;
    }
    result.push(&command[start..]);
    result
}

fn strip_leading_env_vars(command: &str) -> &str {
    // fast path: no equals sign, no environment variable
    if !command.contains('=') {
        return command;
    }

    let mut in_token = false;
    let mut token_start_pos = 0;
    let mut in_single_quotes = false;
    let mut in_double_quotes = false;
    let mut escape_next = false;
    let mut has_equals_outside_quotes = false;

    for (i, g) in UnicodeSegmentation::grapheme_indices(command, true) {
        if escape_next {
            escape_next = false;
            continue;
        }

        if !in_token {
            token_start_pos = i;
        }

        match g {
            "\\" => {
                escape_next = true;
                in_token = true;
            }
            "'" if !in_double_quotes => {
                in_single_quotes = !in_single_quotes;
                in_token = true;
            }
            "\"" if !in_single_quotes => {
                in_double_quotes = !in_double_quotes;
                in_token = true;
            }
            "=" if !in_single_quotes && !in_double_quotes => {
                has_equals_outside_quotes = true;
                in_token = true;
            }
            " " | "\t" if !in_single_quotes && !in_double_quotes => {
                if in_token {
                    if !has_equals_outside_quotes {
                        // if we're not in an env var, we can break early
                        break;
                    }
                    in_token = false;
                    has_equals_outside_quotes = false;
                }
            }
            _ => {
                in_token = true;
            }
        }
    }

    command[token_start_pos..].trim()
}

pub fn pretty_print(stats: Stats, ngram_size: usize) {
    let max = stats.top.iter().map(|x| x.1).max().unwrap();
    let num_pad = max.ilog10() as usize + 1;

    // Find the length of the longest command name for each column
    let column_widths = stats
        .top
        .iter()
        .map(|(commands, _)| commands.iter().map(|c| c.len()).collect::<Vec<usize>>())
        .fold(vec![0; ngram_size], |acc, item| {
            acc.iter()
                .zip(item.iter())
                .map(|(a, i)| *std::cmp::max(a, i))
                .collect()
        });

    for (command, count) in stats.top {
        let gray = SetAttribute(crossterm::style::Attribute::Dim);
        let bold = SetAttribute(crossterm::style::Attribute::Bold);

        let in_ten = 10 * count / max;

        print!("[");
        

        for i in 0..in_ten {
            

            

            print!("▮");
        }

        for _ in in_ten..10 {
            print!(" ");
        }

        let formatted_command = command
            .iter()
            .zip(column_widths.iter())
            .map(|(cmd, width)| format!("{cmd:width$}"))
            .collect::<Vec<_>>()
            .join(" | ");

        println!(
            "{ResetColor}] {gray}{count:num_pad$}{ResetColor} {bold}{formatted_command}{ResetColor}"
        );
    }
    println!("Total commands:   {}", stats.total_commands);
    println!("Unique commands:  {}", stats.unique_commands);
}

pub fn compute(
    settings: &Settings,
    history: &[History],
    count: usize,
    ngram_size: usize,
) -> Option<Stats> {
    let mut commands = HashSet::<&str>::with_capacity(history.len());
    let mut total_unignored = 0;
    let mut prefixes = HashMap::<Vec<&str>, usize>::with_capacity(history.len());

    for i in history {
        // just in case it somehow has a leading tab or space or something (legacy console history didn't ignore space prefixes)
        let command = strip_leading_env_vars(i.command.trim());
        let prefix = interesting_command(settings, command);

        if settings.stats.ignored_commands.iter().any(|c| c == prefix) {
            continue;
        }

        total_unignored += 1;
        commands.insert(command);

        split_at_pipe(command)
            .iter()
            .map(|l| {
                let command = l.trim();
                commands.insert(command);
                command
            })
            .collect::<Vec<_>>()
            .windows(ngram_size)
            .for_each(|w| {
                *prefixes
                    .entry(w.iter().map(|c| interesting_command(settings, c)).collect())
                    .or_default() += 1;
            });
    }

    let unique = commands.len();
    let mut top = prefixes.into_iter().collect::<Vec<_>>();

    top.sort_unstable_by_key(|x| std::cmp::Reverse(x.1));
    top.truncate(count);

    if top.is_empty() {
        return None;
    }

    Some(Stats {
        unique_commands: unique,
        total_commands: total_unignored,
        top: top
            .into_iter()
            .map(|t| (t.0.into_iter().map(|s| s.to_string()).collect(), t.1))
            .collect(),
    })
}

