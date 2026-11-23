//! Shared KDL conversion logic for `from kdl` / `to kdl`.
//!
//! Two data models:
//! - **nodes** — KDL document AST as node rows (`name`/`args`/`props`/`children`/`type`)
//! - **jik** — JSON-in-KDL mapping of ordinary Nushell values
//!
//! Language versions are selected with `--spec 1|2` (default 2).

mod jik;
mod nodes;
mod types;

pub(crate) use jik::{jik_document_to_value, value_to_jik_document};
pub(crate) use nodes::{node_rows_to_kdl_document, nodes_document_to_value};
pub(crate) use types::NonRoundtrip;

use kdl::{KdlDocument, KdlError};
use console_engine::command_prelude::*;
use console_protocol::{
    DEFAULT_ERROR_CONTEXT, PipelineMetadata, shell_error::generic::GenericError,
    truncated_source_window,
};

/// Private metadata key written by `from kdl` so `to kdl` can restore format/spec defaults.
pub(crate) const KDL_META_KEY: &str = "shell_kdl";

/// Schema version embedded in metadata.
const KDL_META_VERSION: &str = "v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum KdlSpec {
    V1,
    #[default]
    V2,
}

impl KdlSpec {
    pub(crate) fn from_i64(value: i64, span: Span) -> Result<Self, ShellError> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            _ => Err(ShellError::IncorrectValue {
                msg: "KDL --spec must be 1 or 2".into(),
                val_span: span,
                call_span: span,
            }),
        }
    }

    pub(crate) fn as_i64(self) -> i64 {
        match self {
            Self::V1 => 1,
            Self::V2 => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KdlFormat {
    Nodes,
    Jik,
}

impl KdlFormat {
    pub(crate) fn parse(value: &str, span: Span) -> Result<Self, ShellError> {
        match value {
            "nodes" => Ok(Self::Nodes),
            "jik" => Ok(Self::Jik),
            _ => Err(ShellError::IncorrectValue {
                msg: "KDL --format must be 'nodes' or 'jik'".into(),
                val_span: span,
                call_span: span,
            }),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nodes => "nodes",
            Self::Jik => "jik",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct KdlMetadata {
    pub format: KdlFormat,
    pub spec: KdlSpec,
}

impl KdlMetadata {
    pub(crate) fn write_to(self, metadata: &mut PipelineMetadata, span: Span) {
        metadata.custom.insert(
            KDL_META_KEY,
            Value::record(
                record! {
                    "version" => Value::string(KDL_META_VERSION, span),
                    "format" => Value::string(self.format.as_str(), span),
                    "spec" => Value::int(self.spec.as_i64(), span),
                },
                span,
            ),
        );
    }

    pub(crate) fn read_from(metadata: &PipelineMetadata) -> Option<Self> {
        let value = metadata.custom.get(KDL_META_KEY)?;
        let record = value.as_record().ok()?;
        let version = record.get("version")?.as_str().ok()?;
        if version != KDL_META_VERSION {
            return None;
        }
        let format =
            KdlFormat::parse(record.get("format")?.as_str().ok()?, Span::unknown()).ok()?;
        let spec = KdlSpec::from_i64(record.get("spec")?.as_int().ok()?, Span::unknown()).ok()?;
        Some(Self { format, spec })
    }

    pub(crate) fn clear(metadata: &mut PipelineMetadata) {
        metadata.custom.remove(KDL_META_KEY);
        // Drop legacy marker from older from kdl implementations.
        metadata.custom.remove("shell_kdl_canonical");
    }
}

pub(crate) fn parse_kdl_document(
    input: &str,
    spec: KdlSpec,
    span: Span,
) -> Result<KdlDocument, ShellError> {
    let result = match spec {
        KdlSpec::V1 => KdlDocument::parse_v1(input),
        KdlSpec::V2 => KdlDocument::parse(input),
    };
    result.map_err(|err| kdl_error_to_shell_error(input, span, &err))
}

pub(crate) fn document_to_string(mut document: KdlDocument, spec: KdlSpec) -> String {
    document.autoformat();
    match spec {
        KdlSpec::V1 => {
            document.ensure_v1();
            document.to_string()
        }
        KdlSpec::V2 => {
            document.ensure_v2();
            document.to_string()
        }
    }
}

pub(crate) fn kdl_error_to_shell_error(input: &str, span: Span, err: &KdlError) -> ShellError {
    if let Some(diagnostic) = err.diagnostics.first() {
        let diagnostic_message = kdl_diagnostics_message(&err.diagnostics);
        let byte_offset = diagnostic.span.offset();
        let (src, label_span) = truncated_source_window(
            input,
            Span::new(byte_offset, byte_offset),
            DEFAULT_ERROR_CONTEXT,
        );

        return ShellError::Generic(
            GenericError::new(
                "Error while parsing KDL text",
                "error parsing KDL text",
                span,
            )
            .with_inner([ShellError::OutsideSpannedLabeledError {
                src,
                error: "Error while parsing KDL text".into(),
                msg: diagnostic_message,
                span: label_span,
            }]),
        );
    }

    ShellError::CantConvert {
        to_type: format!("structured kdl data ({err})"),
        from_type: "string".into(),
        span,
        help: None,
    }
}

pub(crate) fn kdl_diagnostics_message(diagnostics: &[kdl::KdlDiagnostic]) -> String {
    if diagnostics.len() == 1 {
        return kdl_diagnostic_message(&diagnostics[0]);
    }

    diagnostics
        .iter()
        .enumerate()
        .map(|(index, diagnostic)| {
            format!(
                "diagnostic {}:\n{}",
                index + 1,
                kdl_diagnostic_message(diagnostic)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn kdl_diagnostic_message(diagnostic: &kdl::KdlDiagnostic) -> String {
    let mut parts = Vec::new();

    if let Some(message) = &diagnostic.message {
        parts.push(message.clone());
    }

    if let Some(label) = &diagnostic.label
        && parts.last() != Some(label)
    {
        parts.push(label.clone());
    }

    if let Some(help) = &diagnostic.help {
        parts.push(format!("help: {help}"));
    }

    if parts.is_empty() {
        "error parsing KDL text".to_owned()
    } else {
        parts.join("\n")
    }
}

pub(crate) fn resolve_non_roundtrip(
    serialize: bool,
    non_roundtrip: Option<Spanned<String>>,
    engine_state: &EngineState,
) -> Result<NonRoundtrip, ShellError> {
    match (serialize, non_roundtrip.as_ref().map(|nr| nr.item.as_str())) {
        (false, None | Some("error")) => Ok(NonRoundtrip::Error),
        (true, None | Some("lossy")) => Ok(NonRoundtrip::Lossy {
            engine_state: Box::new(engine_state.clone()),
        }),
        (false, Some("null")) => Ok(NonRoundtrip::Null),
        (false, Some("lossy")) => Ok(NonRoundtrip::Lossy {
            engine_state: Box::new(engine_state.clone()),
        }),
        (false, Some(_)) => Err(ShellError::IncompatibleParametersSingle {
            msg: "expected `error`, `null` or `lossy`".into(),
            span: non_roundtrip.expect("non_roundtrip is some").span,
        }),
        (true, Some(_)) => Err(ShellError::IncompatibleParameters {
            left_message: "this is a shorthand to".into(),
            left_span: Span::unknown(),
            right_message: "this with `lossy`".into(),
            right_span: non_roundtrip.expect("non_roundtrip is some").span,
        }),
    }
}

/// Identifier helper that clears inherited formatting.
pub(crate) fn identifier_for(key: &str) -> kdl::KdlIdentifier {
    let mut identifier = kdl::KdlIdentifier::from(key.to_owned());
    identifier.clear_format();
    identifier
}

