// One job: the diff — the hunks of a comparison, with the lines that
// changed and where they were and where they are.
//
// The comparison is named by the caller and is never mixed: the working
// tree against the index, the index against HEAD, and the working tree
// against HEAD are three different answers.

use crate::command::Comparison;
use crate::model::{Hunk, HunkLine, HunkLineKind};

/// FileDiff is the diff of one file: the hunks that changed it, and the
/// name it has. A diff of a whole tree is a list of these.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<Hunk>,
}

/// parse reads a unified diff into its hunks.
pub fn parse(output: &str) -> Vec<Hunk> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_line: u64 = 0;
    let mut new_line: u64 = 0;
    for line in output.lines() {
        if line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let (old_start, old_lines, new_start, new_lines) = read_header(line);
            old_line = old_start;
            new_line = new_start;
            current = Some(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines: Vec::new(),
            });
            continue;
        }
        let hunk = match current.as_mut() {
            Some(hunk) => hunk,
            None => continue,
        };
        if line.starts_with('+') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Added,
                text: line[1..].to_string(),
                old_line: None,
                new_line: Some(new_line),
            });
            new_line += 1;
        } else if line.starts_with('-') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Removed,
                text: line[1..].to_string(),
                old_line: Some(old_line),
                new_line: None,
            });
            old_line += 1;
        } else if line.starts_with(' ') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Context,
                text: line[1..].to_string(),
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        }
    }
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
    hunks
}

/// read_header reads "@@ -12,3 +14,5 @@" into the two sides.
fn read_header(line: &str) -> (u64, u64, u64, u64) {
    let mut old = (0, 0);
    let mut new = (0, 0);
    for piece in line.split_whitespace() {
        if let Some(rest) = piece.strip_prefix('-') {
            old = read_range(rest);
        } else if let Some(rest) = piece.strip_prefix('+') {
            new = read_range(rest);
        }
    }
    (old.0, old.1, new.0, new.1)
}

/// read_range reads "12,3" or "12" into a start and a count.
fn read_range(text: &str) -> (u64, u64) {
    let text = text.trim_end_matches("@@");
    match text.split_once(',') {
        Some((start, count)) => (
            start.trim().parse().unwrap_or(0),
            count.trim().parse().unwrap_or(0),
        ),
        None => (text.trim().parse().unwrap_or(0), 1),
    }
}

/// parse_files reads a diff of a whole tree: the files it touched, and the
/// hunks of each.
///
/// The name of the file is read from the header of its own diff, so a path
/// with a space in it stays one path.
pub fn parse_files(output: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut path = String::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;
    let mut old_line: u64 = 0;
    let mut new_line: u64 = 0;
    for line in output.lines() {
        if line.starts_with("diff --git ") {
            // The file that was being read is closed, and the next one
            // begins. The last hunk of the file belongs to that file.
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            if !path.is_empty() {
                files.push(FileDiff {
                    path: path.clone(),
                    hunks: std::mem::take(&mut hunks),
                });
            }
            path = String::new();
            continue;
        }
        if line.starts_with("+++ ") {
            // The name the file has now.
            path = line[4..].trim_start_matches("b/").to_string();
            continue;
        }
        if line.starts_with("--- ") {
            // A file that is gone has no new name; the old one is its name.
            if path.is_empty() {
                path = line[4..].trim_start_matches("a/").to_string();
            }
            continue;
        }
        if line.starts_with("@@") {
            if let Some(hunk) = current.take() {
                hunks.push(hunk);
            }
            let (old_start, old_lines, new_start, new_lines) = read_header(line);
            old_line = old_start;
            new_line = new_start;
            current = Some(Hunk {
                old_start,
                old_lines,
                new_start,
                new_lines,
                lines: Vec::new(),
            });
            continue;
        }
        let hunk = match current.as_mut() {
            Some(hunk) => hunk,
            None => continue,
        };
        if line.starts_with('+') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Added,
                text: line[1..].to_string(),
                old_line: None,
                new_line: Some(new_line),
            });
            new_line += 1;
        } else if line.starts_with('-') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Removed,
                text: line[1..].to_string(),
                old_line: Some(old_line),
                new_line: None,
            });
            old_line += 1;
        } else if line.starts_with(' ') {
            hunk.lines.push(HunkLine {
                kind: HunkLineKind::Context,
                text: line[1..].to_string(),
                old_line: Some(old_line),
                new_line: Some(new_line),
            });
            old_line += 1;
            new_line += 1;
        }
    }
    if let Some(hunk) = current.take() {
        hunks.push(hunk);
    }
    if !path.is_empty() {
        files.push(FileDiff { path, hunks });
    }
    files
}

/// name answers the comparison as a person reads it.
pub fn comparison_name(comparison: Comparison) -> &'static str {
    comparison.name()
}

