use pest::error::Error as PestError;
use rayon::prelude::*;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::config::{Style, parse_style_string};
use crate::context::{Context, Shell};
use crate::segment::Segment;

use super::model::*;
use super::parser::{Rule, parse};

#[derive(Clone)]
enum VariableValue<'a> {
    Plain(Cow<'a, str>),
    NoEscapingPlain(Cow<'a, str>),
    Styled(Vec<Segment>),
    Meta(Vec<FormatElement<'a>>),
}

impl Default for VariableValue<'_> {
    fn default() -> Self {
        Self::Plain(Cow::Borrowed(""))
    }
}

type VariableMapType<'a> =
    BTreeMap<String, Option<Result<VariableValue<'a>, StringFormatterError>>>;
type StyleVariableMapType<'a> =
    BTreeMap<String, Option<Result<Cow<'a, str>, StringFormatterError>>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StringFormatterError {
    Custom(String),
    Parse(Box<PestError<Rule>>),
}

impl fmt::Display for StringFormatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(error) => write!(f, "{error}"),
            Self::Parse(error) => write!(f, "{error}"),
        }
    }
}

impl Error for StringFormatterError {}

impl From<String> for StringFormatterError {
    fn from(error: String) -> Self {
        Self::Custom(error)
    }
}

pub struct StringFormatter<'a> {
    format: Vec<FormatElement<'a>>,
    variables: VariableMapType<'a>,
    style_variables: StyleVariableMapType<'a>,
}

impl<'a> StringFormatter<'a> {
    /// Creates an instance of `StringFormatter` from a format string
    ///
    /// This method will throw an Error when the given format string fails to parse.
    pub fn new(format: &'a str) -> Result<Self, StringFormatterError> {
        let format = parse(format).map_err(StringFormatterError::Parse)?;

        // Cache all variables
        let variables = format
            .get_variables()
            .into_iter()
            .map(|key| (key.to_string(), None))
            .collect::<VariableMapType>();

        let style_variables = format
            .get_style_variables()
            .into_iter()
            .map(|key| (key.to_string(), None))
            .collect::<StyleVariableMapType>();

        Ok(Self {
            format,
            variables,
            style_variables,
        })
    }

    /// A `StringFormatter` that does no formatting, parse just returns the raw text
    pub fn raw(text: &'a str) -> Self {
        Self {
            format: vec![FormatElement::Text(text.into())],
            variables: BTreeMap::new(),
            style_variables: BTreeMap::new(),
        }
    }

    /// Maps variable name to its value
    ///
    /// You should provide a function or closure that accepts the variable name `name: &str` as a
    /// parameter and returns the one of the following values:
    ///
    /// - `None`: This variable will be reserved for further mappers. If it is `None` when
    ///   `self.parse()` is called, it will be dropped.
    ///
    /// - `Some(Err(StringFormatterError))`: This variable will throws `StringFormatterError` when
    ///   `self.parse()` is called. Return this if some fatal error occurred and the format string
    ///   should not be rendered.
    ///
    /// - `Some(Ok(_))`: The value of this variable will be displayed in the format string.
    ///
    #[must_use]
    pub fn map<T, M>(mut self, mapper: M) -> Self
    where
        T: Into<Cow<'a, str>>,
        M: Fn(&str) -> Option<Result<T, StringFormatterError>> + Sync,
    {
        self.variables
            .par_iter_mut()
            .filter(|(_, value)| value.is_none())
            .for_each(|(key, value)| {
                *value = mapper(key).map(|var| var.map(|var| VariableValue::Plain(var.into())));
            });
        self
    }

    /// Maps variable name into a value which is wrapped to prevent escaping later
    ///
    /// This should be used for variables that should not be escaped before inclusion in the prompt
    ///
    /// See `StringFormatter::map` for description on the parameters.
    ///
    #[must_use]
    pub fn map_no_escaping<T, M>(mut self, mapper: M) -> Self
    where
        T: Into<Cow<'a, str>>,
        M: Fn(&str) -> Option<Result<T, StringFormatterError>> + Sync,
    {
        self.variables
            .par_iter_mut()
            .filter(|(_, value)| value.is_none())
            .for_each(|(key, value)| {
                *value = mapper(key)
                    .map(|var| var.map(|var| VariableValue::NoEscapingPlain(var.into())));
            });
        self
    }

    /// Maps a meta-variable to a format string containing other variables.
    ///
    /// This function should be called **before** other map methods so that variables found in
    /// the format strings of meta-variables can be cached properly.
    ///
    /// See `StringFormatter::map` for description on the parameters.
    #[must_use]
    pub fn map_meta<M>(mut self, mapper: M) -> Self
    where
        M: Fn(&str, &BTreeSet<String>) -> Option<&'a str> + Sync,
    {
        let variables = self.get_variables();
        let (variables, style_variables) = self
            .variables
            .iter_mut()
            .filter(|(_, value)| value.is_none())
            .fold(
                (VariableMapType::new(), StyleVariableMapType::new()),
                |(mut v, mut sv), (key, value)| {
                    *value = mapper(key, &variables).map(|format| {
                        StringFormatter::new(format).map(|formatter| {
                            let StringFormatter {
                                format,
                                mut variables,
                                mut style_variables,
                            } = formatter;

                            // Add variables in meta variables to self
                            v.append(&mut variables);
                            sv.append(&mut style_variables);

                            VariableValue::Meta(format)
                        })
                    });

                    (v, sv)
                },
            );

        self.variables.extend(variables);
        self.style_variables.extend(style_variables);

        self
    }

    /// Maps variable name to an array of segments
    ///
    /// See `StringFormatter::map` for description on the parameters.
    #[must_use]
    pub fn map_variables_to_segments<M>(mut self, mapper: M) -> Self
    where
        M: Fn(&str) -> Option<Result<Vec<Segment>, StringFormatterError>> + Sync,
    {
        self.variables
            .par_iter_mut()
            .filter(|(_, value)| value.is_none())
            .for_each(|(key, value)| {
                *value = mapper(key).map(|var| var.map(VariableValue::Styled));
            });
        self
    }

    /// Maps variable name in a style string to its value
    ///
    /// See `StringFormatter::map` for description on the parameters.
    #[must_use]
    pub fn map_style<T, M>(mut self, mapper: M) -> Self
    where
        T: Into<Cow<'a, str>>,
        M: Fn(&str) -> Option<Result<T, StringFormatterError>> + Sync,
    {
        self.style_variables
            .par_iter_mut()
            .filter(|(_, value)| value.is_none())
            .for_each(|(key, value)| {
                *value = mapper(key).map(|var| var.map(Into::into));
            });
        self
    }

    /// Parse the format string and consume self.
    ///
    /// This method will throw an Error in the following conditions:
    ///
    /// - Format string in meta variables fails to parse
    /// - Variable mapper returns an error.
    pub fn parse(
        self,
        default_style: Option<Style>,
        context: Option<&Context>,
    ) -> Result<Vec<Segment>, StringFormatterError> {
        fn parse_textgroup<'a>(
            textgroup: TextGroup<'a>,
            variables: &'a VariableMapType<'a>,
            style_variables: &'a StyleVariableMapType<'a>,
            context: Option<&Context>,
        ) -> Result<Vec<Segment>, StringFormatterError> {
            let style = parse_style(textgroup.style, style_variables, context).transpose()?;

            // Empty textgroups still produce a segment to preserve style for prev_fg/prev_bg references
            if textgroup.format.is_empty() {
                return Ok(Segment::from_text(style, ""));
            }
            parse_format(textgroup.format, style, variables, style_variables, context)
        }

        fn parse_style<'a>(
            style: Vec<StyleElement>,
            variables: &'a StyleVariableMapType<'a>,
            context: Option<&Context>,
        ) -> Option<Result<Style, StringFormatterError>> {
            let style_strings = style
                .into_iter()
                .map(|style| match style {
                    StyleElement::Text(text) => Ok(text),
                    StyleElement::Variable(name) => {
                        let variable = variables.get(name.as_ref()).unwrap_or(&None);
                        match variable {
                            Some(style_string) => style_string.clone(),
                            None => Ok("".into()),
                        }
                    }
                })
                .collect::<Result<Vec<Cow<str>>, StringFormatterError>>();
            style_strings
                .map(|style_strings| {
                    let style_string: String =
                        style_strings.iter().flat_map(|s| s.chars()).collect();
                    parse_style_string(&style_string, context)
                })
                .transpose()
        }

        fn parse_format<'a>(
            format: Vec<FormatElement<'a>>,
            style: Option<Style>,
            variables: &'a VariableMapType<'a>,
            style_variables: &'a StyleVariableMapType<'a>,
            context: Option<&Context>,
        ) -> Result<Vec<Segment>, StringFormatterError> {
            let results: Result<Vec<Vec<Segment>>, StringFormatterError> = format
                .into_iter()
                .map(|el| {
                    match el {
                        FormatElement::Text(text) => Ok(Segment::from_text(
                            style,
                            shell_prompt_escape(
                                text,
                                match context {
                                    None => Shell::Unknown,
                                    Some(c) => c.shell,
                                },
                            ),
                        )),
                        FormatElement::TextGroup(textgroup) => {
                            parse_textgroup(textgroup, variables, style_variables, context)
                        }
                        FormatElement::Variable(name) => variables
                            .get(name.as_ref())
                            .expect("Uncached variable found")
                            .as_ref()
                            .map_or_else(
                                || Ok(Vec::new()),
                                |segments| match segments.clone()? {
                                    VariableValue::Styled(segments) => Ok(segments
                                        .into_iter()
                                        .map(|mut segment| {
                                            // Derive upper style if the style of segments are none.
                                            segment.set_style_if_empty(style);
                                            segment
                                        })
                                        .collect()),
                                    VariableValue::Plain(text) => Ok(Segment::from_text(
                                        style,
                                        shell_prompt_escape(
                                            text,
                                            match context {
                                                None => Shell::Unknown,
                                                Some(c) => c.shell,
                                            },
                                        ),
                                    )),
                                    VariableValue::NoEscapingPlain(text) => {
                                        Ok(Segment::from_text(style, text))
                                    }
                                    VariableValue::Meta(format) => {
                                        let formatter = StringFormatter {
                                            format,
                                            variables: clone_without_meta(variables),
                                            style_variables: style_variables.clone(),
                                        };
                                        formatter.parse(style, context)
                                    }
                                },
                            ),
                        FormatElement::Conditional(format) => {
                            // Show the conditional format string if all the variables inside are not
                            // none or empty string.
                            fn should_show_elements<'a>(
                                format_elements: &[FormatElement],
                                variables: &'a VariableMapType<'a>,
                            ) -> bool {
                                format_elements.get_variables().iter().any(|var| {
                                    variables
                                        .get(var.as_ref())
                                        // false if can't find the variable in format string
                                        .is_some_and(|map_result| {
                                            let map_result = map_result.as_ref();
                                            map_result
                                                .and_then(|result| result.as_ref().ok())
                                                // false if the variable is None or Err, or a meta variable
                                                // that shouldn't show
                                                .is_some_and(|result| match result {
                                                    // If the variable is a meta variable, also
                                                    // check the format string inside it.
                                                    VariableValue::Meta(meta_elements) => {
                                                        let meta_variables =
                                                            clone_without_meta(variables);
                                                        should_show_elements(
                                                            meta_elements,
                                                            &meta_variables,
                                                        )
                                                    }
                                                    VariableValue::Plain(plain_value) => {
                                                        !plain_value.is_empty()
                                                    }
                                                    VariableValue::NoEscapingPlain(
                                                        no_escaping_plain_value,
                                                    ) => !no_escaping_plain_value.is_empty(),
                                                    VariableValue::Styled(segments) => segments
                                                        .iter()
                                                        .any(|x| !x.value().is_empty()),
                                                })
                                        })
                                })
                            }

                            let should_show: bool = should_show_elements(&format, variables);

                            if should_show {
                                parse_format(format, style, variables, style_variables, context)
                            } else {
                                Ok(Vec::new())
                            }
                        }
                    }
                })
                .collect();
            Ok(results?.into_iter().flatten().collect())
        }

        parse_format(
            self.format,
            default_style,
            &self.variables,
            &self.style_variables,
            context,
        )
    }
}

impl VariableHolder<String> for StringFormatter<'_> {
    fn get_variables(&self) -> BTreeSet<String> {
        self.variables.keys().cloned().collect()
    }
}

impl StyleVariableHolder<String> for StringFormatter<'_> {
    fn get_style_variables(&self) -> BTreeSet<String> {
        self.style_variables.keys().cloned().collect()
    }
}

fn clone_without_meta<'a>(variables: &VariableMapType<'a>) -> VariableMapType<'a> {
    variables
        .iter()
        .map(|(key, value)| {
            let value = match value {
                Some(Ok(value)) => match value {
                    VariableValue::Meta(_) => None,
                    other => Some(Ok(other.clone())),
                },
                Some(Err(e)) => Some(Err(e.clone())),
                None => None,
            };
            (key.clone(), value)
        })
        .collect()
}

/// Escape interpretable characters for the shell prompt
pub fn shell_prompt_escape<T>(text: T, shell: Shell) -> String
where
    T: Into<String>,
{
    // Handle other interpretable characters
    match shell {
        // Bash might interpret backslashes, backticks and $
        // see #658 for more details
        Shell::Bash => text
            .into()
            .replace('\\', r"\\")
            .replace('$', r"\$")
            .replace('`', r"\`"),
        Shell::Zsh => {
            // % is an escape in zsh, see PROMPT in `man zshmisc`
            text.into().replace('%', "%%")
        }
        _ => text.into(),
    }
}

