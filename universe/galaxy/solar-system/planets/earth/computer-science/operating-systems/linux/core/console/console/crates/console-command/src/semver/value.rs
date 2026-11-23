use super::parse;
use console_protocol::{
    CustomValue, ShellError, Span, Value,
    ast::{Comparison, Operator},
    casing::Casing,
};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

/// A semantic version value, optionally carrying a display prefix from loose parsing.
///
/// Equality and ordering compare only [`version`]; `prefix` is presentation metadata
/// (e.g. `"v"` from `v1.2.3` parsed with `--loose`). That means `v1.0.0` and `1.0.0`
/// compare equal, and command `example` `result:` values that use [`SemverValue`] only
/// lock the version identity—not the display form. Prefer `display()` / `to text` when
/// tests need to assert a preserved prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemverValue {
    pub version: semver::Version,
    /// Original loose prefix such as `"v"`, `"v."`, or `"v:"`. Empty when none.
    #[serde(default)]
    pub prefix: String,
}

/// Compares only the underlying SemVer version; `prefix` is ignored.
impl PartialEq for SemverValue {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
    }
}

impl Eq for SemverValue {}

impl PartialOrd for SemverValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SemverValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.version.cmp(&other.version)
    }
}

#[typetag::serde]
impl console_protocol::CustomValue for SemverValue {
    fn clone_value(&self, span: Span) -> Value {
        Value::custom(Box::new(self.clone()), span)
    }

    fn type_name(&self) -> String {
        "semver".to_string()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        Ok(Value::string(self.display(), span))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }

    /// Hashes the version only: `partial_cmp` ignores the display prefix.
    fn hash_value(&self, mut state: &mut dyn Hasher) {
        self.version.hash(&mut state);
    }

    fn partial_cmp(&self, other: &Value) -> Option<Ordering> {
        match other {
            Value::Custom { val, .. } => {
                // Prefer a direct downcast when both values share the same crate instance.
                // Fall back to type_name + base-value parse so comparison still works when
                // TypeIds diverge (e.g. unit tests that pull console-command in twice via
                // console-test-support).
                if let Some(other) = val.as_any().downcast_ref::<SemverValue>() {
                    return self.version.partial_cmp(&other.version);
                }
                if val.type_name() == self.type_name() {
                    return val
                        .to_base_value(other.span())
                        .ok()
                        .and_then(|value| match value {
                            // Accept loose prefixes so cross-crate fallback still works
                            // when display form is e.g. "v1.2.3".
                            Value::String { val, .. } => {
                                parse::parse_version(&val, true).ok().map(|(v, _)| v)
                            }
                            _ => None,
                        })
                        .and_then(|other_version| self.version.partial_cmp(&other_version));
                }
                None
            }
            Value::String { val, .. } => parse::parse_version(val, true)
                .ok()
                .and_then(|(other_version, _)| self.version.partial_cmp(&other_version)),
            _ => None,
        }
    }

    fn follow_path_string(
        &self,
        self_span: Span,
        column_name: String,
        path_span: Span,
        _optional: bool,
        casing: Casing,
    ) -> Result<Value, ShellError> {
        let col = match casing {
            Casing::Sensitive => column_name,
            Casing::Insensitive => column_name.to_lowercase(),
        };

        match col.as_str() {
            "major" => Ok(Value::int(self.version.major as i64, path_span)),
            "minor" => Ok(Value::int(self.version.minor as i64, path_span)),
            "patch" => Ok(Value::int(self.version.patch as i64, path_span)),
            "pre" => Ok(Value::string(self.version.pre.to_string(), path_span)),
            "build" => Ok(Value::string(self.version.build.to_string(), path_span)),
            "prefix" => Ok(Value::string(self.prefix.clone(), path_span)),
            _ => Err(ShellError::CantFindColumn {
                col_name: col,
                span: Some(path_span),
                src_span: self_span,
            }),
        }
    }

    fn operation(
        &self,
        lhs_span: Span,
        operator: Operator,
        op: Span,
        right: &Value,
    ) -> Result<Value, ShellError> {
        match operator {
            Operator::Comparison(Comparison::In) => {
                if let Value::Custom { val, .. } = right
                    && let Some(range) = val
                        .as_any()
                        .downcast_ref::<super::range::SemverRangeValue>()
                {
                    return Ok(Value::bool(range.requirement.matches(&self.version), op));
                }
                Err(ShellError::OperatorIncompatibleTypes {
                    op: operator,
                    lhs: console_protocol::Type::Custom("semver".into()),
                    rhs: right.get_type(),
                    op_span: op,
                    lhs_span,
                    rhs_span: right.span(),
                    help: Some("expected a semver-range on the right side"),
                })
            }
            Operator::Comparison(
                comparison @ (Comparison::Equal
                | Comparison::NotEqual
                | Comparison::LessThan
                | Comparison::LessThanOrEqual
                | Comparison::GreaterThan
                | Comparison::GreaterThanOrEqual),
            ) => match CustomValue::partial_cmp(self, right) {
                Some(ordering) => {
                    let result = match comparison {
                        Comparison::Equal => ordering == Ordering::Equal,
                        Comparison::NotEqual => ordering != Ordering::Equal,
                        Comparison::LessThan => ordering == Ordering::Less,
                        Comparison::LessThanOrEqual => {
                            matches!(ordering, Ordering::Less | Ordering::Equal)
                        }
                        Comparison::GreaterThan => ordering == Ordering::Greater,
                        Comparison::GreaterThanOrEqual => {
                            matches!(ordering, Ordering::Greater | Ordering::Equal)
                        }
                        _ => unreachable!("matched only equality/ordering comparisons above"),
                    };
                    Ok(Value::bool(result, op))
                }
                None => Err(ShellError::OperatorIncompatibleTypes {
                    op: operator,
                    lhs: console_protocol::Type::Custom("semver".into()),
                    rhs: right.get_type(),
                    op_span: op,
                    lhs_span,
                    rhs_span: right.span(),
                    help: Some("expected another semver or a valid version string"),
                }),
            },
            _ => Err(ShellError::OperatorUnsupportedType {
                op: operator,
                unsupported: console_protocol::Type::Custom(self.type_name().into()),
                op_span: op,
                unsupported_span: lhs_span,
                help: None,
            }),
        }
    }
}

impl SemverValue {
    pub fn new(version: semver::Version) -> Self {
        Self {
            version,
            prefix: String::new(),
        }
    }

    pub fn with_prefix(version: semver::Version, prefix: impl Into<String>) -> Self {
        Self {
            version,
            prefix: prefix.into(),
        }
    }

    /// Display form including any loose prefix (e.g. `v1.2.3`).
    pub fn display(&self) -> String {
        if self.prefix.is_empty() {
            self.version.to_string()
        } else {
            format!("{}{}", self.prefix, self.version)
        }
    }

    /// Parse from a string. When `loose` is true, accepts prefixes like `v`, `v.`, `v:`.
    pub fn parse(s: &str, loose: bool) -> Result<Self, semver::Error> {
        let (version, prefix) = parse::parse_version(s, loose)?;
        Ok(Self::with_prefix(version, prefix))
    }

    pub fn bump_major(&self) -> Self {
        Self::with_prefix(
            semver::Version {
                major: self.version.major + 1,
                minor: 0,
                patch: 0,
                pre: semver::Prerelease::EMPTY,
                build: semver::BuildMetadata::EMPTY,
            },
            self.prefix.clone(),
        )
    }

    pub fn bump_minor(&self) -> Self {
        Self::with_prefix(
            semver::Version {
                major: self.version.major,
                minor: self.version.minor + 1,
                patch: 0,
                pre: semver::Prerelease::EMPTY,
                build: semver::BuildMetadata::EMPTY,
            },
            self.prefix.clone(),
        )
    }

    pub fn bump_patch(&self) -> Self {
        Self::with_prefix(
            semver::Version {
                major: self.version.major,
                minor: self.version.minor,
                patch: self.version.patch + 1,
                pre: semver::Prerelease::EMPTY,
                build: semver::BuildMetadata::EMPTY,
            },
            self.prefix.clone(),
        )
    }

    pub fn bump_prerelease(&self, tag: &str) -> Result<Self, ShellError> {
        let current_pre = self.version.pre.as_str();

        let new_pre = if current_pre.is_empty() {
            format!("{}.1", tag)
        } else if current_pre.starts_with(tag) {
            if let Some(dot_pos) = current_pre.rfind('.') {
                let suffix = &current_pre[dot_pos + 1..];
                if let Ok(num) = suffix.parse::<u64>() {
                    format!("{}.{}", tag, num + 1)
                } else {
                    format!("{}.1", tag)
                }
            } else {
                format!("{}.1", tag)
            }
        } else {
            format!("{}.0", tag)
        };

        let pre = semver::Prerelease::new(&new_pre).map_err(|e| {
            ShellError::Generic(console_protocol::shell_error::generic::GenericError::new(
                "Invalid prerelease",
                e.to_string(),
                Span::unknown(),
            ))
        })?;

        Ok(Self::with_prefix(
            semver::Version {
                major: self.version.major,
                minor: self.version.minor,
                patch: self.version.patch,
                pre,
                build: self.version.build.clone(),
            },
            self.prefix.clone(),
        ))
    }

    pub fn bump_release(&self) -> Self {
        Self::with_prefix(
            semver::Version {
                major: self.version.major,
                minor: self.version.minor,
                patch: self.version.patch,
                pre: semver::Prerelease::EMPTY,
                build: semver::BuildMetadata::EMPTY,
            },
            self.prefix.clone(),
        )
    }

    pub fn set_build_metadata(&self, metadata: &str) -> Result<Self, ShellError> {
        let build = semver::BuildMetadata::new(metadata).map_err(|e| {
            ShellError::Generic(console_protocol::shell_error::generic::GenericError::new(
                "Invalid build metadata",
                e.to_string(),
                Span::unknown(),
            ))
        })?;

        Ok(Self::with_prefix(
            semver::Version {
                major: self.version.major,
                minor: self.version.minor,
                patch: self.version.patch,
                pre: self.version.pre.clone(),
                build,
            },
            self.prefix.clone(),
        ))
    }

    /// Convert a pipeline value into a [`SemverValue`].
    ///
    /// When `loose` is true, string inputs may use prefixes like `v1.2.3`.
    pub fn try_from_value(value: &Value, loose: bool) -> Result<Self, ShellError> {
        let span = value.span();

        match value {
            Value::String { val, .. } => {
                Self::parse(val, loose).map_err(|e| ShellError::IncorrectValue {
                    msg: format!("Value is not a valid semver version: {e}"),
                    val_span: span,
                    call_span: span,
                })
            }
            Value::Custom { val, .. } => {
                if let Some(semver) = val.as_any().downcast_ref::<Self>() {
                    Ok(semver.clone())
                } else {
                    Err(ShellError::CantConvert {
                        to_type: "semver".into(),
                        from_type: val.type_name(),
                        span,
                        help: None,
                    })
                }
            }
            x => Err(ShellError::CantConvert {
                to_type: "semver".into(),
                from_type: x.get_type().to_string(),
                span,
                help: None,
            }),
        }
    }

    /// For use by tests and examples only.
    pub fn test_value(s: &str) -> Value {
        let semver =
            Self::parse(s, true).unwrap_or_else(|_| Self::new(semver::Version::new(0, 0, 0)));
        Value::test_custom_value(Box::new(semver))
    }
}

impl<'a> TryFrom<&'a Value> for SemverValue {
    type Error = ShellError;

    fn try_from(value: &'a Value) -> Result<Self, Self::Error> {
        Self::try_from_value(value, false)
    }
}

impl Deref for SemverValue {
    type Target = semver::Version;

    fn deref(&self) -> &Self::Target {
        &self.version
    }
}
