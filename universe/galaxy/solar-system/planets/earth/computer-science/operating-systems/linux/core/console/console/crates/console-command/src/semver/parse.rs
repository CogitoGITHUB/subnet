//! Loose semver parsing helpers.
//!
//! Strict SemVer 2.0.0 does not allow a leading `v` (or similar). In practice many
//! tools publish tags like `v1.2.3`, `v.1.2.3`, or `v:1.2.3`. Loose mode strips a
//! single leading prefix of that form before parsing.

/// Strip a loose leading version prefix: `v`/`V`, optionally followed by `.`, `:`,
/// `-`, or `_`, only when the remainder starts with a digit.
///
/// Returns `(prefix, rest)`. When no prefix is recognized, returns `("", s)`.
///
/// # Examples
///
/// - `v1.2.3` → `("v", "1.2.3")`
/// - `V1.2.3` → `("V", "1.2.3")`
/// - `v.1.2.3` → `("v.", "1.2.3")`
/// - `v:1.2.3` → `("v:", "1.2.3")`
/// - `v-1.2.3` → `("v-", "1.2.3")`
/// - `v_1.2.3` → `("v_", "1.2.3")`
/// - `1.2.3` → `("", "1.2.3")`
pub fn strip_loose_version_prefix(s: &str) -> (&str, &str) {
    let after_v = match s.strip_prefix('v').or_else(|| s.strip_prefix('V')) {
        Some(rest) => rest,
        None => return ("", s),
    };

    // v.1.2.3, v:1.2.3, v-1.2.3, or v_1.2.3
    if let Some(after_sep) = after_v
        .strip_prefix('.')
        .or_else(|| after_v.strip_prefix(':'))
        .or_else(|| after_v.strip_prefix('-'))
        .or_else(|| after_v.strip_prefix('_'))
        && after_sep.starts_with(|c: char| c.is_ascii_digit())
    {
        let prefix_len = s.len() - after_sep.len();
        return (&s[..prefix_len], after_sep);
    }

    // v1.2.3
    if after_v.starts_with(|c: char| c.is_ascii_digit()) {
        let prefix_len = s.len() - after_v.len();
        return (&s[..prefix_len], after_v);
    }

    ("", s)
}

/// Parse a version string, optionally accepting a loose leading prefix.
///
/// Returns the parsed version and the captured prefix (empty when strict or none).
pub fn parse_version(s: &str, loose: bool) -> Result<(semver::Version, String), semver::Error> {
    match semver::Version::parse(s) {
        Ok(version) => Ok((version, String::new())),
        Err(strict_err) => {
            if !loose {
                return Err(strict_err);
            }
            let (prefix, rest) = strip_loose_version_prefix(s);
            if prefix.is_empty() {
                return Err(strict_err);
            }
            let version = semver::Version::parse(rest)?;
            Ok((version, prefix.to_string()))
        }
    }
}

/// Normalize a version requirement for loose parsing by stripping `v`/`V`
/// (optionally followed by `.`, `:`, `-`, or `_`) before version numbers at
/// comparator boundaries.
///
/// # Examples
///
/// - `>=v1.0.0` → `>=1.0.0`
/// - `^v1.2.3` → `^1.2.3`
/// - `v1.2.3` → `1.2.3`
/// - `>=v:1.0.0, <v.2.0.0` → `>=1.0.0, <2.0.0`
/// - `>=v-1.0.0, <v_2.0.0` → `>=1.0.0, <2.0.0`
pub fn normalize_loose_range(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;

    while !rest.is_empty() {
        let at_boundary = out.is_empty()
            || matches!(
                out.chars().next_back(),
                Some(' ' | ',' | '<' | '>' | '=' | '^' | '~' | '!')
            );

        if at_boundary {
            let (prefix, after) = strip_loose_version_prefix(rest);
            if !prefix.is_empty() {
                rest = after;
                continue;
            }
        }

        let mut chars = rest.chars();
        let Some(ch) = chars.next() else {
            break;
        };
        out.push(ch);
        rest = chars.as_str();
    }

    out
}

/// Parse a version requirement string, optionally accepting loose version prefixes.
pub fn parse_version_req(s: &str, loose: bool) -> Result<semver::VersionReq, semver::Error> {
    match semver::VersionReq::parse(s) {
        Ok(req) => Ok(req),
        Err(strict_err) => {
            if !loose {
                return Err(strict_err);
            }
            let normalized = normalize_loose_range(s);
            if normalized == s {
                return Err(strict_err);
            }
            semver::VersionReq::parse(&normalized)
        }
    }
}

