use super::StringFormatter;
use super::string_formatter::StringFormatterError;
use crate::segment;
use std::ops::Deref;
use std::sync::LazyLock;
use versions::Versioning;

pub struct VersionFormatter<'a> {
    formatter: StringFormatter<'a>,
}

impl<'a> VersionFormatter<'a> {
    /// Creates an instance of a `VersionFormatter` from a format string
    ///
    /// Like the `StringFormatter`, this will throw an error when the string isn't
    /// parseable.
    pub fn new(format: &'a str) -> Result<Self, StringFormatterError> {
        let formatter = StringFormatter::new(format)?;

        Ok(Self { formatter })
    }

    /// Format version string using provided format
    pub fn format_version(
        version: &'a str,
        format: &'a str,
    ) -> Result<String, StringFormatterError> {
        Self::new(format).and_then(|formatter| formatter.format(version))
    }

    /// Formats a version structure into a readable string
    pub fn format(self, version: &'a str) -> Result<String, StringFormatterError> {
        let parsed = LazyLock::new(|| Versioning::new(version));
        let formatted = self
            .formatter
            .map(|variable| match variable {
                "raw" => Some(Ok(version.to_string())),
                "major" => match parsed.deref().as_ref() {
                    Some(Versioning::Ideal(v)) => Some(Ok(v.major.to_string())),
                    Some(Versioning::General(v)) => Some(Ok(v.nth(0)?.to_string())),
                    _ => None,
                },
                "minor" => match parsed.deref().as_ref() {
                    Some(Versioning::Ideal(v)) => Some(Ok(v.minor.to_string())),
                    Some(Versioning::General(v)) => Some(Ok(v.nth(1)?.to_string())),
                    _ => None,
                },
                "patch" => match parsed.deref().as_ref() {
                    Some(Versioning::Ideal(v)) => Some(Ok(v.patch.to_string())),
                    Some(Versioning::General(v)) => Some(Ok(v.nth(2)?.to_string())),
                    _ => None,
                },
                _ => None,
            })
            .parse(None, None);

        formatted.map(|segments| {
            segments
                .iter()
                .map(segment::Segment::value)
                .collect::<String>()
        })
    }

    pub fn format_module_version(
        module_name: &str,
        version: &str,
        version_format: &str,
    ) -> Option<String> {
        match VersionFormatter::format_version(version, version_format) {
            Ok(formatted) => Some(formatted),
            Err(error) => {
                log::warn!("Error formatting `{module_name}` version:\n{error}");
                Some(format!("v{version}"))
            }
        }
    }
}

