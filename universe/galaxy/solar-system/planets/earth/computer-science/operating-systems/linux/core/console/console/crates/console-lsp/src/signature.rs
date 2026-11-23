use lsp_types::{
    Documentation, MarkupContent, MarkupKind, ParameterInformation, SignatureHelp,
    SignatureHelpParams, SignatureInformation,
};
use console_cli::{SignatureLabel, find_active_call};
use console_protocol::{Flag, PositionalArg, Signature, SyntaxShape, Value, engine::StateWorkingSet};

use crate::{LanguageServer, uri_to_path};
use std::fmt::Write;

pub(crate) fn display_flag(flag: &Flag, verbitam: bool) -> String {
    let md_backtick = if verbitam { "`" } else { "" };
    let mut text = String::new();
    if let Some(short_flag) = flag.short {
        write!(text, "{md_backtick}-{short_flag}{md_backtick}")
            .expect("writing to a String is infallible");
    }
    if let Some(long) = flag.long_name() {
        if flag.short.is_some() {
            text.push_str(", ");
        }
        write!(text, "{md_backtick}--{long}{md_backtick}")
            .expect("writing to a String is infallible");
    }
    text
}

pub(crate) fn doc_for_arg(
    syntax_shape: Option<SyntaxShape>,
    desc: String,
    default_value: Option<Value>,
    optional: bool,
) -> String {
    let mut text = String::new();
    if let Some(mut shape) = syntax_shape {
        if let SyntaxShape::Keyword(_, inner_shape) = shape {
            shape = *inner_shape;
        }
        write!(text, ": `<{shape}>`").expect("writing to a String is infallible");
    }
    if !(desc.is_empty() && default_value.is_none()) || optional {
        text.push_str(" -")
    };
    if !desc.is_empty() {
        write!(text, " {desc}").expect("writing to a String is infallible");
    };
    if let Some(value) = default_value.as_ref().and_then(|v| v.coerce_str().ok()) {
        write!(
            text,
            " ({}default: `{value}`)",
            if optional { "optional, " } else { "" }
        )
        .expect("writing to a String is infallible");
    } else if optional {
        text.push_str(" (optional)");
    }
    text
}

pub(crate) fn get_signature_label(signature: &Signature, indent: bool) -> String {
    let label = SignatureLabel::new(signature);
    if indent {
        format!("  {label}")
    } else {
        label.to_string()
    }
}

impl LanguageServer {
    pub(crate) fn get_signature_help(
        &mut self,
        params: &SignatureHelpParams,
    ) -> Option<SignatureHelp> {
        let path_uri = &params.text_document_position_params.text_document.uri;
        let docs = self.docs.lock().ok()?;
        let file = docs.get_document(path_uri)?;
        let location = file.offset_at(params.text_document_position_params.position) as usize;
        let file_text = file.get_content(None).to_owned();
        drop(docs);

        let engine_state = self.new_engine_state(Some(path_uri));
        let mut working_set = StateWorkingSet::new(&engine_state);

        // NOTE: in case the cursor is at the end of the call expression
        let need_placeholder = location == 0
            || file_text
                .get(location - 1..location)
                .is_some_and(|s| s.chars().all(|c| c.is_whitespace()));
        let file_path = uri_to_path(path_uri);
        let filename = if need_placeholder {
            "lsp_signature_helper_temp_file"
        } else {
            file_path.to_str()?
        };

        let block = if need_placeholder {
            console_parser::parse(
                &mut working_set,
                Some(filename),
                format!(
                    "{}a{}",
                    file_text.get(..location).unwrap_or_default(),
                    file_text.get(location..).unwrap_or_default()
                )
                .as_bytes(),
                false,
            )
        } else {
            console_parser::parse(
                &mut working_set,
                Some(filename),
                file_text.as_bytes(),
                false,
            )
        };
        let span = working_set.get_span_for_filename(filename)?;

        let pos_to_search = location.saturating_add(span.start).saturating_sub(1);
        let active_call = find_active_call(&block, &working_set, pos_to_search, false)?;
        let active_signature = working_set.get_decl(active_call.call.decl_id).signature();
        let label = get_signature_label(&active_signature, false);

        let param_num_before_pos = active_call.positionals_before(pos_to_search + 1) as u32;

        let str_to_doc = |s: String| {
            Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: s,
            }))
        };
        let arg_to_param_info = |arg: PositionalArg, optional: bool| ParameterInformation {
            label: lsp_types::ParameterLabel::Simple(arg.name),
            documentation: str_to_doc(doc_for_arg(
                Some(arg.shape),
                arg.desc,
                arg.default_value,
                optional,
            )),
        };
        let flag_to_param_info = |flag: Flag| ParameterInformation {
            label: lsp_types::ParameterLabel::Simple(display_flag(&flag, false)),
            documentation: str_to_doc(doc_for_arg(flag.arg, flag.desc, flag.default_value, false)),
        };

        // positional args
        let mut parameters: Vec<ParameterInformation> = active_signature
            .required_positional
            .into_iter()
            .map(|arg| arg_to_param_info(arg, false))
            .chain(
                active_signature
                    .optional_positional
                    .into_iter()
                    .map(|arg| arg_to_param_info(arg, true)),
            )
            .collect();
        if let Some(rest_arg) = active_signature.rest_positional {
            parameters.push(arg_to_param_info(rest_arg, false));
        }

        let max_idx = parameters.len().saturating_sub(1) as u32;
        let active_parameter = Some(param_num_before_pos.min(max_idx));
        // also include flags in the end, just for documentation
        parameters.extend(active_signature.named.into_iter().map(flag_to_param_info));

        Some(SignatureHelp {
            signatures: vec![SignatureInformation {
                label,
                documentation: str_to_doc(active_signature.description),
                parameters: Some(parameters),
                active_parameter,
            }],
            active_signature: Some(0),
            active_parameter,
        })
    }
}

