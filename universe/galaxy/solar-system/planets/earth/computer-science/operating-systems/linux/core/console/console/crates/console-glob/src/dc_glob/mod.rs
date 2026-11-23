//! Experimental dc-glob backend imported from glob_experiment by Devyn Cairns for evaluation in Nushell.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::Pattern;

mod compiler;
mod globber;
mod matcher;
mod parser;

const GLOB_CHARS: &[char] = &['*', '?', '['];

/// Options for dc-glob filesystem traversal.
///
/// Patterns follow console-glob's rules, with `{a,b}` alternatives added: a trailing `**` matches the
/// directories below its prefix but not the prefix itself, and `\` is a literal character.
#[derive(Debug, Clone, Default)]
pub struct GlobWalkOptions {
    /// Maximum recursion depth. `None` means unbounded.
    pub max_depth: Option<usize>,
    /// Follow directory symlinks while traversing.
    pub follow_symlinks: bool,
    /// Exclusion glob patterns used to filter and prune traversal. They match the way
    /// [`DcPattern`] does: console-glob's rules, `{a,b}` included.
    pub excludes: Vec<String>,
    /// Optional interrupt flag. When set to `true` the traversal stops as soon
    /// as possible and the iterator ends.
    pub interrupt: Option<Arc<AtomicBool>>,
    /// When `true`, match path components case-insensitively (ASCII only).
    pub ignore_case: bool,
    /// When `true`, `*`, `?` and `[...]` don't match a `.` at the start of a name, so a hidden
    /// name matches only where the pattern spells its dot out. Mirrors
    /// [`MatchOptions::require_literal_leading_dot`](crate::MatchOptions).
    pub require_literal_leading_dot: bool,
    /// When `true`, `**` drops a directory whose name starts with `.`: it neither descends into
    /// it nor lets the rest of the pattern match it. Mirrors
    /// [`MatchOptions::recursive_match_hidden_dir`](crate::MatchOptions) set to `false`.
    pub skip_hidden_in_recursion: bool,
}

impl GlobWalkOptions {
    /// Options that give the results console-glob gives for `options`: directory symlinks are
    /// followed, and case and hidden names are handled the way `options` asks.
    pub fn console_glob(options: crate::MatchOptions) -> Self {
        GlobWalkOptions {
            follow_symlinks: true,
            ignore_case: !options.case_sensitive,
            require_literal_leading_dot: options.require_literal_leading_dot,
            skip_hidden_in_recursion: !options.recursive_match_hidden_dir,
            ..GlobWalkOptions::default()
        }
    }
}

/// Return true if the given pattern contains glob metacharacters.
///
/// Braces don't count: a word like `HEAD@{1}` or `{a,b}` without `*`, `?` or `[` is not
/// expanded, so it reaches commands and external programs unchanged.
pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(GLOB_CHARS)
}

/// Return true if a path component of a glob has pattern syntax, so the walk has to match it
/// rather than start below it.
///
/// Unlike [`is_glob`], `{` counts, so `{src,tests}` in `{src,tests}/**/*.rs` is matched as
/// alternatives rather than taken as a directory name.
pub fn is_glob_component(component: &str) -> bool {
    is_glob(component) || component.contains('{')
}

/// Escape glob metacharacters, braces included, so a pattern is matched literally.
pub fn escape(pattern: &str) -> String {
    let mut escaped = String::with_capacity(pattern.len());
    for ch in pattern.chars() {
        if matches!(ch, '?' | '*' | '[' | ']' | '{' | '}') {
            escaped.push('[');
            escaped.push(ch);
            escaped.push(']');
        } else {
            escaped.push(ch);
        }
    }
    escaped
}

/// Reject the patterns console-glob rejects, such as `***`, `a**` or an unclosed `[`.
///
/// console-glob parses the pattern both as a whole and one path component at a time, so `a/[b/c]` is
/// rejected too. Braces are literal to console-glob, so `**` can't be part of `{...}`.
fn check_nu_glob_syntax(pattern: &str) -> anyhow::Result<()> {
    crate::Pattern::new(pattern)?;
    for component in pattern.split(std::path::is_separator) {
        crate::Pattern::new(component)?;
    }
    Ok(())
}

/// Expand `pattern` relative to `relative_to` using the experimental dc-glob backend, with the
/// results console-glob's `glob` gives: directory symlinks are followed.
pub fn glob_from(
    relative_to: impl AsRef<Path>,
    pattern: impl AsRef<str>,
) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<PathBuf>> + Send>> {
    glob_with(
        relative_to,
        pattern,
        &GlobWalkOptions::console_glob(crate::MatchOptions::default()),
    )
}

/// Expand `pattern` relative to `relative_to` using dc-glob with traversal options.
pub fn glob_with(
    relative_to: impl AsRef<Path>,
    pattern: impl AsRef<str>,
    options: &GlobWalkOptions,
) -> anyhow::Result<Box<dyn Iterator<Item = anyhow::Result<PathBuf>> + Send>> {
    let pattern = pattern.as_ref().to_owned();
    // A mistyped `rm ***` fails rather than expanding.
    check_nu_glob_syntax(&pattern)?;
    let parsed = parser::parse(&pattern);
    // The fast path compares only the file name, which can't tell whether a leading `.` came
    // from a literal in the pattern, so the matcher handles those patterns.
    let fast_path = if options.require_literal_leading_dot {
        None
    } else {
        detect_recursive_suffix_fast_path(&parsed)
    };
    let mut include_program = compiler::compile_with_options(&parsed, options.ignore_case)?;
    // As with console-glob, a pattern that ends with a separator, like `*/`, matches only
    // directories. Parsing drops that separator, so it is checked here.
    include_program.dirs_only |= pattern.ends_with(std::path::is_separator);
    include_program.literal_leading_dot = options.require_literal_leading_dot;
    include_program.skip_hidden_in_recursion = options.skip_hidden_in_recursion;

    let exclude_programs = options
        .excludes
        .iter()
        .map(|exclude| DcPattern::new(exclude))
        .collect::<anyhow::Result<Vec<_>>>()?;

    Ok(Box::new(globber::glob_with_options(
        relative_to.as_ref().to_path_buf(),
        Arc::new(include_program),
        globber::TraversalOptions {
            max_depth: options.max_depth,
            follow_symlinks: options.follow_symlinks,
        },
        exclude_programs,
        globber::InterruptFlag(options.interrupt.clone()),
        fast_path,
    )))
}

fn detect_recursive_suffix_fast_path(
    pattern: &parser::Pattern,
) -> Option<globber::RecursiveFastPath> {
    use parser::AstNode;

    let nodes = pattern.nodes.as_slice();
    if nodes.len() < 3 {
        return None;
    }

    let recurse_index = nodes
        .windows(2)
        .position(|window| matches!(window, [AstNode::Recurse, AstNode::Separator]))?;

    let tail = &nodes[(recurse_index + 2)..];
    if tail.is_empty() {
        return None;
    }

    let static_prefix_only = nodes[..recurse_index].iter().all(|node| {
        matches!(
            node,
            AstNode::Prefix(_)
                | AstNode::RootDir
                | AstNode::CurDir
                | AstNode::ParentDir
                | AstNode::LiteralString(_)
                | AstNode::Separator
        )
    });

    if !static_prefix_only || tail.iter().any(|node| matches!(node, AstNode::Separator)) {
        return None;
    }

    if let [AstNode::Wildcard] = tail {
        return Some(globber::RecursiveFastPath::Suffix(Box::<[u8]>::default()));
    }

    if let [AstNode::Wildcard, AstNode::LiteralString(bytes)] = tail {
        return Some(globber::RecursiveFastPath::Suffix(
            bytes.clone().into_boxed_slice(),
        ));
    }

    let tokens = tail
        .iter()
        .map(|node| match node {
            AstNode::LiteralString(bytes) => Some(globber::BasenameToken::Literal(
                bytes.clone().into_boxed_slice(),
            )),
            AstNode::Wildcard => Some(globber::BasenameToken::Wildcard),
            AstNode::AnyCharacter => Some(globber::BasenameToken::AnyCharacter),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;

    Some(globber::RecursiveFastPath::BasenamePattern(
        tokens.into_boxed_slice(),
    ))
}

/// Return a formatted AST dump for a glob pattern.
pub fn debug_parse(pattern: impl AsRef<str>) -> String {
    let parsed = parser::parse(pattern.as_ref());
    format!("{parsed:#?}")
}

/// Return a formatted compiled-program dump for a glob pattern.
pub fn debug_compile(pattern: impl AsRef<str>) -> anyhow::Result<String> {
    let parsed = parser::parse(pattern.as_ref());
    let compiled = compiler::compile(&parsed)?;
    Ok(format!("{compiled:#?}"))
}

/// Return whether `path` matches `pattern` as a complete dc-glob match.
pub fn debug_matches(pattern: impl AsRef<str>, path: impl AsRef<Path>) -> anyhow::Result<bool> {
    let parsed = parser::parse(pattern.as_ref());
    let compiled = compiler::compile(&parsed)?;
    Ok(matcher::path_matches(path.as_ref(), false, &compiled).valid_as_complete_match)
}

/// A glob pattern for matching paths without filesystem traversal, as `du --exclude`,
/// `watch --glob` and dc-glob's excludes do.
///
/// It matches the way console-glob's [`Pattern`] does, with `{a,b}` alternatives added: each
/// combination of choices is a console-glob pattern, and a path matches when one of them does. So as
/// with console-glob, `*` also matches `/` here, and `target/` doesn't match `target`.
#[derive(Clone)]
pub struct DcPattern {
    pattern: String,
    choices: Vec<Pattern>,
    options: crate::MatchOptions,
}

impl std::fmt::Debug for DcPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DcPattern")
            .field("pattern", &self.pattern)
            .finish()
    }
}

impl DcPattern {
    /// Compile a glob pattern. Returns an error if the pattern is invalid, as console-glob's
    /// [`Pattern::new`] does.
    pub fn new(pattern: &str) -> anyhow::Result<Self> {
        Self::compile(pattern, false)
    }

    /// Compile a glob pattern with case-insensitive matching (ASCII only).
    pub fn with_ignore_case(pattern: &str) -> anyhow::Result<Self> {
        Self::compile(pattern, true)
    }

    fn compile(pattern: &str, case_insensitive: bool) -> anyhow::Result<Self> {
        // Braces are literal to console-glob, so this also rejects `**` inside `{...}`.
        Pattern::new(pattern)?;
        let choices = parser::expand_alternatives(pattern)?
            .iter()
            .map(|choice| Pattern::new(choice))
            .collect::<Result<_, _>>()?;
        Ok(DcPattern {
            pattern: pattern.to_owned(),
            choices,
            options: crate::MatchOptions {
                case_sensitive: !case_insensitive,
                ..crate::MatchOptions::default()
            },
        })
    }

    /// Return `true` if the given path matches this pattern.
    pub fn matches_path(&self, path: &Path) -> bool {
        self.matches_path_with(path, self.options)
    }

    /// Return `true` if the given path matches this pattern with `options`, as
    /// [`Pattern::matches_path_with`] does.
    pub fn matches_path_with(&self, path: &Path, options: crate::MatchOptions) -> bool {
        self.choices
            .iter()
            .any(|choice| choice.matches_path_with(path, options))
    }
}

