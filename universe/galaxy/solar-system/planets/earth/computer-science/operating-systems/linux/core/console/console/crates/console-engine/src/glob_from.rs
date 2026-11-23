use console_glob::MatchOptions;
use console_path::{absolute_with, expand_path_with};
use console_protocol::{
    NuGlob, ShellError, Signals, Span, Spanned, shell_error::generic::GenericError,
    shell_error::io::IoError,
};
use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

/// This function is like `console_glob::glob` from the `glob` crate, except it is relative to a given cwd.
///
/// It returns a tuple of two values: the first is an optional prefix that the expanded filenames share.
/// This prefix can be removed from the front of each value to give an approximation of the relative path
/// to the user
///
/// The second of the two values is an iterator over the matching filepaths.
///
/// With `require_literal_leading_dot` set in `options`, a glob matches a name that starts with
/// `.` only where the pattern spells that dot out, at any depth, on either glob backend. See
/// [`HiddenNameFilter`].
#[allow(clippy::type_complexity)]
pub fn glob_from(
    pattern: &Spanned<NuGlob>,
    cwd: &Path,
    span: Span,
    options: Option<MatchOptions>,
    signals: Signals,
) -> Result<
    (
        Option<PathBuf>,
        Box<dyn Iterator<Item = Result<PathBuf, ShellError>> + Send>,
    ),
    ShellError,
> {
    let no_glob_for_pattern = matches!(pattern.item, NuGlob::DoNotExpand(_));
    let pattern_span = pattern.span;
    let skip_hidden = options.is_some_and(|options| options.require_literal_leading_dot);
    let mut hidden_name_filter = None;
    let (prefix, pattern) = if console_glob::is_glob_with_backend(pattern.item.as_ref()) {
        // Pattern contains glob, split it
        let mut p = PathBuf::new();
        let path = PathBuf::from(&pattern.item.as_ref());
        let components = path.components();
        let mut counter = 0;

        // A component with pattern syntax ends the literal prefix. With dc-glob that includes
        // `{`, so `{src,tests}` in `{src,tests}/**/*.rs` is matched as alternatives.
        for c in components {
            if let Component::Normal(os) = c
                && console_glob::is_glob_component_with_backend(os.to_string_lossy().as_ref())
            {
                break;
            }
            p.push(c);
            counter += 1;
        }

        let mut just_pattern = PathBuf::new();
        for c in counter..path.components().count() {
            if let Some(comp) = path.components().nth(c) {
                just_pattern.push(comp);
            }
        }
        if no_glob_for_pattern {
            just_pattern = PathBuf::from(console_glob::escape_with_backend(
                &just_pattern.to_string_lossy(),
            ));
        }

        // Now expand `p` to get full prefix
        let path = expand_path_with(p, cwd, pattern.item.is_expand());
        let escaped_prefix = PathBuf::from(console_glob::escape_with_backend(&path.to_string_lossy()));

        if skip_hidden && !no_glob_for_pattern {
            hidden_name_filter = Some(HiddenNameFilter::new(path.clone(), &just_pattern));
        }

        (Some(path), escaped_prefix.join(just_pattern))
    } else {
        let path = PathBuf::from(&pattern.item.as_ref());
        let path = expand_path_with(path, cwd, pattern.item.is_expand());
        let is_symlink = match fs::symlink_metadata(&path) {
            Ok(attr) => attr.file_type().is_symlink(),
            Err(_) => false,
        };

        if is_symlink {
            // Escaped for the same reason as an existing path below.
            let pattern = PathBuf::from(console_glob::escape_with_backend(&path.to_string_lossy()));
            (path.parent().map(|parent| parent.to_path_buf()), pattern)
        } else {
            let path = match absolute_with(path.clone(), cwd) {
                // Our path might contain glob metacharacters too, or braces that dc-glob would
                // read as alternatives, so escape it to make glob match it literally. Escaping
                // leaves a path without them unchanged.
                Ok(p) if p.exists() => {
                    PathBuf::from(console_glob::escape_with_backend(&p.to_string_lossy()))
                }
                Ok(_) => {
                    return Err(IoError::new(
                        io::Error::from(io::ErrorKind::NotFound),
                        pattern_span,
                        path,
                    )
                    .into());
                }
                Err(err) => {
                    return Err(IoError::new(err, pattern_span, path).into());
                }
            };
            (path.parent().map(|parent| parent.to_path_buf()), path)
        }
    };

    let pattern = pattern.to_string_lossy().to_string();

    if console_experimental::DC_GLOB.get() {
        // The dc-glob traversal handles literal paths itself: a pattern that
        // names an existing path matches that path, and a pattern that still
        // contains glob metacharacters keeps going through the walker even
        // when a same-named path exists (e.g. a file named `*` must not make
        // bare `*` / `ls` return only that one entry). See #18631.
        //
        // The caller's match options carry over, so hidden names are skipped the same way the
        // legacy backend skips them (`rm *` must not reach dotfiles).
        let walk_options = console_glob::dc_glob::GlobWalkOptions {
            interrupt: signals.interrupt_flag(),
            ..console_glob::dc_glob::GlobWalkOptions::console_glob(options.unwrap_or_default())
        };
        let iter = console_glob::dc_glob::glob_with(cwd, &pattern, &walk_options).map_err(|e| {
            ShellError::Generic(GenericError::new(
                "Error extracting glob pattern",
                e.to_string(),
                span,
            ))
        })?;

        // dc-glob returns paths relative to the traversal start directory.
        // Join them with `prefix` to produce absolute paths, matching the
        // legacy backend's behaviour.
        let prefix_for_map = prefix.clone();
        let mapped = iter.map(move |x| match x {
            Ok(v) => {
                let v = match &prefix_for_map {
                    Some(p) if v.is_relative() => p.join(&v),
                    _ => v,
                };
                Ok(v)
            }
            Err(e) => Err(ShellError::Generic(GenericError::new(
                "Error extracting glob pattern",
                e.to_string(),
                span,
            ))),
        });

        Ok((prefix, keep_allowed(mapped, hidden_name_filter)))
    } else {
        let glob_options = options.unwrap_or_default();
        let glob = console_glob::glob_with(&pattern, glob_options, signals).map_err(|e| {
            ShellError::Generic(GenericError::new(
                "Error extracting glob pattern",
                e.to_string(),
                span,
            ))
        })?;

        let mapped = glob.map(move |x| match x {
            Ok(v) => Ok(v),
            Err(e) => Err(ShellError::Generic(GenericError::new(
                "Error extracting glob pattern",
                e.error().to_string(),
                span,
            ))),
        });

        Ok((prefix, keep_allowed(mapped, hidden_name_filter)))
    }
}

/// Boxes a glob's `matches`, dropping the ones `filter` doesn't allow. Errors are kept.
fn keep_allowed(
    matches: impl Iterator<Item = Result<PathBuf, ShellError>> + Send + 'static,
    filter: Option<HiddenNameFilter>,
) -> Box<dyn Iterator<Item = Result<PathBuf, ShellError>> + Send> {
    match filter {
        Some(filter) => Box::new(
            matches.filter(move |path| path.as_ref().map_or(true, |path| filter.allows(path))),
        ),
        None => Box::new(matches),
    }
}

/// Drops a glob's matches that reach a name starting with `.` where the pattern doesn't spell
/// that dot out, at any depth: `*`, `?`, `[...]` and `**` never match a hidden name on their own.
/// `.*` and `src/.cache/*` spell the dot out; so does the literal part of the pattern, the
/// directory the glob starts from, so `.cache/*` and a glob run inside a hidden directory work.
///
/// Both backends already hold `*`, `?` and `[...]` to this through
/// `require_literal_leading_dot`, but their `**` walks into hidden directories, so `glob_from`
/// checks every match here instead.
///
/// Only a whole `**` part counts as recursive. A dc-glob part like `{.a,b}` counts as one name
/// that doesn't spell a dot, so its hidden matches are dropped rather than kept.
struct HiddenNameFilter {
    /// The directory the glob starts from.
    start: PathBuf,
    /// The number of components in `start`.
    start_len: usize,
    /// The pattern's parts below `start`.
    parts: Vec<GlobPart>,
}

/// A part of a glob pattern, as far as hidden names go.
#[derive(Clone, Copy)]
enum GlobPart {
    /// `**`: any number of names, none of them hidden.
    Recurse,
    /// One name, hidden exactly when the part starts with `.`.
    One { hidden: bool },
}

impl HiddenNameFilter {
    fn new(start: PathBuf, pattern: &Path) -> Self {
        let parts = pattern
            .components()
            .map(|part| match part {
                Component::Normal(name) if name == "**" => GlobPart::Recurse,
                part => GlobPart::One {
                    hidden: is_hidden(part),
                },
            })
            .collect();
        Self {
            start_len: start.components().count(),
            start,
            parts,
        }
    }

    /// Whether the match `path` may be kept.
    fn allows(&self, path: &Path) -> bool {
        // Both backends build every match on `start`, so most matches, the ones with no hidden
        // name below it, are settled by skipping its components, without re-parsing it.
        debug_assert!(
            path.starts_with(&self.start),
            "{path:?} isn't a match below {:?}",
            self.start
        );
        if !path.components().skip(self.start_len).any(is_hidden) {
            return true;
        }
        // Should a match turn up outside `start` anyway, drop it rather than guess.
        let Ok(below) = path.strip_prefix(&self.start) else {
            return false;
        };
        let names: Vec<bool> = below.components().map(is_hidden).collect();
        dots_spelled_out(&self.parts, &names)
    }
}

/// Whether a component is a name that starts with `.`. `.` and `..` themselves aren't names.
fn is_hidden(component: Component) -> bool {
    matches!(component, Component::Normal(name) if name.as_encoded_bytes().first() == Some(&b'.'))
}

/// Whether the pattern `parts` can stand for a match whose names below the start directory are
/// hidden exactly where `names` is `true`.
fn dots_spelled_out(parts: &[GlobPart], names: &[bool]) -> bool {
    // `reach[j]`: the parts so far can stand for exactly the first `j` names.
    let mut reach = vec![false; names.len() + 1];
    reach[0] = true;
    for part in parts {
        match *part {
            GlobPart::Recurse => {
                for j in 1..=names.len() {
                    reach[j] |= reach[j - 1] && !names[j - 1];
                }
            }
            GlobPart::One { hidden } => {
                for j in (1..=names.len()).rev() {
                    reach[j] = reach[j - 1] && names[j - 1] == hidden;
                }
                reach[0] = false;
            }
        }
    }
    reach[names.len()]
}

