#[cfg(not(target_os = "windows"))]
use super::utils::directory_nix as directory_utils;
#[cfg(target_os = "windows")]
use super::utils::directory_win as directory_utils;
use super::utils::path::PathExt as SPathExt;
use indexmap::IndexMap;
use path_slash::{PathBufExt, PathExt};
use regex::{Error, Regex};
use std::borrow::Cow;
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;

use super::{Context, Module};

use super::utils::directory::truncate;
use crate::config::{Either, ModuleConfig};
use crate::configs::directory::{DirectoryConfig, SubstitutionConfig};
use crate::formatter::StringFormatter;

/// Creates a module with the current logical or physical directory
///
/// Will perform path contraction, substitution, and truncation.
///
/// **Contraction**
/// - Paths beginning with the home directory or with a git repo right inside
///   the home directory will be contracted to `~`, or the set `HOME_SYMBOL`
/// - Paths containing a git repo will contract to begin at the repo root
///
/// **Substitution**
/// Paths will undergo user-provided substitutions of substrings
///
/// **Truncation**
/// Paths will be limited in length to `3` path components by default.
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("directory");
    let config: DirectoryConfig = DirectoryConfig::try_load(module.config);

    let home_dir = context
        .get_home()
        .expect("Unable to determine HOME_DIR for user");
    let physical_dir = &context.current_dir;
    let display_dir = if config.use_logical_path {
        &context.logical_dir
    } else {
        &context.current_dir
    };

    log::debug!("Home dir: {home_dir:?}");
    log::debug!("Physical dir: {physical_dir:?}");
    log::debug!("Display dir: {display_dir:?}");

    // Attempt repository path contraction (if we are in a git repository)
    // Otherwise use the logical path, automatically contracting
    let repo = if config.truncate_to_repo || config.repo_root_style.is_some() {
        context.get_git_repo().ok()
    } else {
        None
    };
    let dir_string = if config.truncate_to_repo {
        repo.and_then(|r| r.workdir.as_ref())
            .filter(|&root| root != &home_dir)
            .and_then(|root| contract_repo_path(display_dir, root))
    } else {
        None
    };

    let mut is_truncated = dir_string.is_some();

    // the home directory if required.
    let dir_string = dir_string
        .unwrap_or_else(|| contract_path(display_dir, &home_dir, config.home_symbol).to_string());

    #[cfg(windows)]
    let dir_string = remove_extended_path_prefix(dir_string);

    // Apply path substitutions
    let dir_string = match substitute_path(dir_string.clone(), &config.substitutions) {
        Ok(result) => result,
        Err(err) => {
            log::warn!("Invalid regex in directory substitutions: {err}");
            dir_string
        }
    };

    // Truncate the dir string to the maximum number of path components
    let dir_string =
        if let Some(truncated) = truncate(&dir_string, config.truncation_length as usize) {
            is_truncated = true;
            truncated
        } else {
            dir_string
        };

    let prefix = if is_truncated {
        // Substitutions could have changed the prefix, so don't allow them and
        // fish-style path contraction together
        if config.fish_style_pwd_dir_length > 0 && config.substitutions_empty() {
            // If user is using fish style path, we need to add the segment first
            let contracted_home_dir = contract_path(display_dir, &home_dir, config.home_symbol);
            to_fish_style(
                config.fish_style_pwd_dir_length as usize,
                &contracted_home_dir,
                &dir_string,
            )
        } else {
            String::from(config.truncation_symbol)
        }
    } else {
        String::new()
    };

    // The path is only split around the repo root when there is a root to
    // highlight and the logical path actually lies under it. A symlink pointing
    // sideways into a subdirectory of the repo resolves into it without sharing
    // a prefix with it, so it has no contracted form and is not split.
    let split_at_repo_root = repo
        .and_then(|r| r.workdir.as_ref())
        .filter(|_| config.repo_root_style.is_some())
        .and_then(|repo_root| contract_repo_path(display_dir, repo_root))
        .and_then(|contracted_path| {
            let repo_path_vec: Vec<&str> = contracted_path.split('/').collect();
            let after_repo_root = contracted_path.replacen(repo_path_vec[0], "", 1);
            let num_segments_after_root = after_repo_root.split('/').count();

            (config.truncation_length == 0
                || ((num_segments_after_root - 1) as i64) < config.truncation_length)
                .then(|| {
                    [
                        before_root_dir(&dir_string, &contracted_path).to_string(),
                        repo_path_vec[0].to_string(),
                        after_repo_root,
                    ]
                })
        });

    let path_vec = match split_at_repo_root {
        Some([before_root, root, after_root]) => [prefix + &before_root, root, after_root],
        // the whole path in one segment, with no repo root to highlight
        None => [String::new(), String::new(), prefix + dir_string.as_str()],
    };

    let path_vec = if config.use_os_path_sep {
        path_vec.map(|i| convert_path_sep(&i))
    } else {
        path_vec
    };

    let display_format = if path_vec[0].is_empty() && path_vec[1].is_empty() {
        config.format
    } else {
        config.repo_root_format
    };
    let repo_root_style = config.repo_root_style.unwrap_or(config.style);
    let before_repo_root_style = config.before_repo_root_style.unwrap_or(config.style);

    let parsed = StringFormatter::new(display_format).and_then(|formatter| {
        formatter
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                "read_only_style" => Some(Ok(config.read_only_style)),
                "repo_root_style" => Some(Ok(repo_root_style)),
                "before_repo_root_style" => Some(Ok(before_repo_root_style)),
                _ => None,
            })
            .map(|variable| match variable {
                "path" => Some(Ok(path_vec[2].as_str())),
                "before_root_path" => Some(Ok(path_vec[0].as_str())),
                "repo_root" => Some(Ok(path_vec[1].as_str())),
                "read_only" => {
                    if is_readonly_dir(physical_dir) {
                        Some(Ok(config.read_only))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `directory`:\n{error}");
            return None;
        }
    });

    Some(module)
}

#[cfg(windows)]
fn remove_extended_path_prefix(path: String) -> String {
    fn try_trim_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
        if !s.starts_with(prefix) {
            return None;
        }
        Some(&s[prefix.len()..])
    }
    // Trim any Windows extended-path prefix from the display path
    if let Some(unc) = try_trim_prefix(&path, r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    if let Some(p) = try_trim_prefix(&path, r"\\?\") {
        return p.to_string();
    }
    path
}

fn is_readonly_dir(path: &Path) -> bool {
    match directory_utils::is_write_allowed(path) {
        Ok(res) => !res,
        Err(e) => {
            log::debug!("Failed to determine read only status of directory '{path:?}': {e}");
            false
        }
    }
}

/// Contract the root component of a path
///
/// Replaces the `top_level_path` in a given `full_path` with the provided
/// `top_level_replacement`.
fn contract_path<'a>(
    full_path: &'a Path,
    top_level_path: &'a Path,
    top_level_replacement: &'a str,
) -> Cow<'a, str> {
    if !full_path.normalised_starts_with(top_level_path) {
        return full_path.to_slash_lossy();
    }

    if full_path.normalised_equals(top_level_path) {
        return Cow::from(top_level_replacement);
    }

    // Because we've done a normalised path comparison above
    // we can safely ignore the Prefix components when doing this
    // strip_prefix operation.
    let sub_path = full_path
        .without_prefix()
        .strip_prefix(top_level_path.without_prefix())
        .unwrap_or(full_path);

    Cow::from(format!(
        "{replacement}{separator}{path}",
        replacement = top_level_replacement,
        separator = "/",
        path = sub_path.to_slash_lossy()
    ))
}

/// Contract the root component of a path based on the real path
///
/// Replaces the `top_level_path` in a given `full_path` with the provided
/// `top_level_replacement` by walking ancestors and comparing its real path.
fn contract_repo_path(full_path: &Path, top_level_path: &Path) -> Option<String> {
    let top_level_real_path = real_path(top_level_path);
    // Walk ancestors to preserve logical path in `full_path`.
    // If we'd just `full_real_path.strip_prefix(top_level_real_path)`,
    // then it wouldn't preserve logical path. It would've returned physical path.
    for (i, ancestor) in full_path.ancestors().enumerate() {
        let ancestor_real_path = real_path(ancestor);
        if ancestor_real_path != top_level_real_path {
            continue;
        }

        let components: Vec<_> = full_path.components().collect();
        let repo_name = components[components.len() - i - 1]
            .as_os_str()
            .to_string_lossy();

        if i == 0 {
            return Some(repo_name.to_string());
        }

        let path = PathBuf::from_iter(&components[components.len() - i..]);
        return Some(format!(
            "{repo_name}{separator}{path}",
            repo_name = repo_name,
            separator = "/",
            path = path.to_slash_lossy()
        ));
    }
    None
}

fn real_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let path = path.as_ref();
    let mut buf = PathBuf::new();
    for component in path.components() {
        let next = buf.join(component);
        if let Ok(realpath) = next.read_link() {
            if realpath.is_absolute() {
                buf = realpath;
            } else {
                buf.push(realpath);
            }
        } else {
            buf = next;
        }
    }
    buf.canonicalize().unwrap_or_else(|_| path.into())
}

/// Perform a list of string substitutions on the path
///
/// Given a list of (from, to) pairs, this will perform the string
/// substitutions, in order, on the path. Any non-pair of strings is ignored.
fn substitute_path(
    dir_string: String,
    substitutions: &Either<Vec<SubstitutionConfig>, IndexMap<String, &str>>,
) -> Result<String, Error> {
    let substitutions: &Vec<SubstitutionConfig> = match substitutions {
        Either::First(vec) => vec,
        Either::Second(table) => &table
            .iter()
            .map(|(from, to)| SubstitutionConfig {
                from: String::from(from),
                to,
                regex: false,
            })
            .collect(),
    };

    let mut substituted_dir = dir_string;

    for substitution in substitutions {
        if substitution.regex {
            let re = Regex::new(substitution.from.as_str())?;
            substituted_dir = re
                .replace(substituted_dir.as_str(), substitution.to)
                .to_string()
        } else {
            substituted_dir = substituted_dir.replace(substitution.from.as_str(), substitution.to)
        }
    }
    Ok(substituted_dir)
}

/// Takes part before contracted path and replaces it with fish style path
///
/// Will take the first letter of each directory before the contracted path and
/// use that in the path instead. See the following example.
///
/// Absolute Path: `/Users/Bob/Projects/work/a_repo`
/// Contracted Path: `a_repo`
/// With Fish Style: `~/P/w/a_repo`
///
/// Absolute Path: `/some/Path/not/in_a/repo/but_nested`
/// Contracted Path: `in_a/repo/but_nested`
/// With Fish Style: `/s/P/n/in_a/repo/but_nested`
fn to_fish_style(pwd_dir_length: usize, dir_string: &str, truncated_dir_string: &str) -> String {
    let replaced_dir_string = dir_string.trim_end_matches(truncated_dir_string);
    let components = replaced_dir_string.split('/').collect::<Vec<&str>>();

    if components.is_empty() {
        return replaced_dir_string.to_string();
    }

    components
        .into_iter()
        .map(|word| -> String {
            let chars = UnicodeSegmentation::graphemes(word, true).collect::<Vec<&str>>();
            match word {
                "" => String::new(),
                _ if chars.len() <= pwd_dir_length => word.to_string(),
                _ if word.starts_with('.') => chars[..=pwd_dir_length].join(""),
                _ => chars[..pwd_dir_length].join(""),
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Convert the path separators in `path` to the OS specific path separators.
fn convert_path_sep(path: &str) -> String {
    PathBuf::from_slash(path).to_string_lossy().into_owned()
}

/// Get the path before the git repo root by trim the most right repo name.
fn before_root_dir<'a>(path: &'a str, repo: &'a str) -> &'a str {
    match path.rsplit_once(repo) {
        Some((a, _)) => a,
        None => path,
    }
}

