use lsp_types::{Hover, HoverContents, HoverParams, MarkupContent, MarkupKind};
use console_protocol::{PositionalArg, engine::Command};
use std::{borrow::Cow, fmt::Write};

use crate::{
    Id, LanguageServer,
    signature::{display_flag, doc_for_arg, get_signature_label},
};

impl LanguageServer {
    pub(crate) fn get_decl_description(decl: &dyn Command, skip_description: bool) -> String {
        let mut description = String::new();

        if !skip_description {
            // First description
            writeln!(description, "{}", decl.description().replace('\r', ""))
                .expect("writing to a String is infallible");

            // Additional description
            if !decl.extra_description().is_empty() {
                write!(description, "\n{}\n", decl.extra_description())
                    .expect("writing to a String is infallible");
            }
        }
        // Usage
        description.push_str("\n---\n### Usage \n```nu\n");
        let signature = decl.signature();
        description.push_str(&get_signature_label(&signature, true));
        description.push_str("\n```\n");

        // Flags
        if !signature.named.is_empty() {
            description.push_str("\n### Flags\n\n");
            let mut first = true;
            for named in signature.named {
                if first {
                    first = false;
                } else {
                    description.push('\n');
                }
                description.push_str("  ");
                description.push_str(&display_flag(&named, true));
                description.push_str(&doc_for_arg(
                    named.arg,
                    named.desc,
                    named.default_value,
                    false,
                ));
                description.push('\n');
            }
            description.push('\n');
        }

        // Parameters
        if !signature.required_positional.is_empty()
            || !signature.optional_positional.is_empty()
            || signature.rest_positional.is_some()
        {
            description.push_str("\n### Parameters\n\n");
            let mut first = true;
            let mut write_arg = |arg: PositionalArg, optional: bool| {
                if first {
                    first = false;
                } else {
                    description.push('\n');
                }
                write!(description, "  `{}`", arg.name).expect("writing to a String is infallible");
                description.push_str(&doc_for_arg(
                    Some(arg.shape),
                    arg.desc,
                    arg.default_value,
                    optional,
                ));
                description.push('\n');
            };
            for required_arg in signature.required_positional {
                write_arg(required_arg, false);
            }
            for optional_arg in signature.optional_positional {
                write_arg(optional_arg, true);
            }
            if let Some(arg) = signature.rest_positional {
                if !first {
                    description.push('\n');
                }
                write!(description, " `...{}`", arg.name)
                    .expect("writing to a String is infallible");
                description.push_str(&doc_for_arg(
                    Some(arg.shape),
                    arg.desc,
                    arg.default_value,
                    false,
                ));
                description.push('\n');
            }
            description.push('\n');
        }

        // Input/output types
        if !signature.input_output_types.is_empty() {
            description.push_str("\n### Input/output types\n");
            description.push_str("\n```nu\n");
            for input_output in &signature.input_output_types {
                writeln!(description, " {} | {}", input_output.0, input_output.1)
                    .expect("writing to a String is infallible");
            }
            description.push_str("\n```\n");
        }

        // Examples
        if !decl.examples().is_empty() {
            description.push_str("### Example(s)\n");
            for example in decl.examples() {
                write!(
                    description,
                    "  {}\n```nu\n  {}\n```\n",
                    example.description, example.example
                )
                .expect("writing to a String is infallible");
            }
        }
        description
    }

    pub(crate) fn hover(&mut self, params: &HoverParams) -> Option<Hover> {
        let path_uri = &params.text_document_position_params.text_document.uri;
        let mut engine_state = self.new_engine_state(Some(path_uri));
        let (working_set, id, _, _) = self
            .parse_and_find(
                &mut engine_state,
                path_uri,
                params.text_document_position_params.position,
            )
            .ok()?;

        let markdown_hover = |content: String| {
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                // TODO
                range: None,
            })
        };

        match id {
            Id::Variable(var_id, _) => {
                let var = working_set.get_variable(var_id);
                let value = var
                    .const_val
                    .clone()
                    .and_then(|v| v.coerce_into_string().ok())
                    .unwrap_or(String::from(if var.mutable {
                        "mutable"
                    } else {
                        "immutable"
                    }));
                let contents = format!("```\n{}\n``` \n---\n{}", var.ty, value);
                markdown_hover(contents)
            }
            Id::CellPath(var_id, cell_path) => {
                let var = working_set.get_variable(var_id);
                markdown_hover(
                    var.const_val
                        .as_ref()
                        .and_then(|val| val.follow_cell_path(&cell_path).ok())
                        .map(|val| {
                            let ty = val.get_type();
                            if let Ok(s) = val.coerce_str() {
                                format!("```\n{ty}\n```\n---\n{s}")
                            } else {
                                format!("```\n{ty}\n```")
                            }
                        })
                        .or_else(|| {
                            var.ty
                                .follow_cell_path(&cell_path)
                                .map(|ty| format!("```\n{ty}\n```"))
                        })
                        .unwrap_or("`unknown`".into()),
                )
            }
            Id::Declaration(decl_id) => markdown_hover(Self::get_decl_description(
                working_set.get_decl(decl_id),
                false,
            )),
            Id::Module(module_id, _) => {
                let (mut desc, extra) = working_set
                    .get_module_comments(module_id)
                    .map(|spans| working_set.build_desc(spans))?;
                desc.push_str("\n\n");
                desc.push_str(&extra);
                markdown_hover(desc)
            }
            Id::Value(t) => markdown_hover(format!("`{t}`")),
            Id::External(cmd) => {
                fn fix_manpage_ascii_shenanigans(text: &str) -> Cow<'_, str> {
                    if cfg!(windows) {
                        Cow::Borrowed(text)
                    } else {
                        let re =
                            fancy_regex::Regex::new(r".\x08").expect("regular expression error");
                        re.replace_all(text, "")
                    }
                }
                let command_output = if cfg!(windows) {
                    std::process::Command::new("powershell.exe")
                        .args(["-NoProfile", "-Command", "help", &cmd])
                        .output()
                } else {
                    std::process::Command::new("man").arg(&cmd).output()
                };
                let manpage_str = match command_output {
                    Ok(output) => console_utils::strip_ansi_likely(
                        fix_manpage_ascii_shenanigans(
                            String::from_utf8_lossy(&output.stdout).as_ref(),
                        )
                        .as_ref(),
                    )
                    .to_string(),
                    Err(_) => format!("No command help found for {}", cmd),
                };
                markdown_hover(manpage_str)
            }
        }
    }
}

