use crate::{LanguageServer, span_to_range};
use lsp_textdocument::FullTextDocument;
use lsp_types::{
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, InlayHintTooltip, MarkupContent,
    MarkupKind, Position, Range,
};
use console_protocol::{
    Type,
    ast::{Argument, Block, Expr, Expression, Operator, Traverse},
    engine::StateWorkingSet,
};
use std::sync::Arc;

fn type_short_name(t: &Type) -> String {
    match t {
        Type::Record(_) => String::from("record"),
        Type::Table(_) => String::from("table"),
        Type::List(_) => String::from("list"),
        _ => t.to_string(),
    }
}

fn extract_inlay_hints_from_expression(
    expr: &Expression,
    working_set: &StateWorkingSet,
    offset: &usize,
    file: &FullTextDocument,
) -> Vec<InlayHint> {
    match &expr.expr {
        Expr::BinaryOp(lhs, op, rhs) => {
            if let Expr::Operator(Operator::Assignment(_)) = op.expr {
                let position = span_to_range(&lhs.span, file, *offset).end;
                let type_rhs = type_short_name(&rhs.ty);
                let type_lhs = type_short_name(&lhs.ty);
                let type_string = match (type_lhs.as_str(), type_rhs.as_str()) {
                    ("any", _) => type_rhs,
                    (_, "any") => type_lhs,
                    _ => type_lhs,
                };
                vec![InlayHint {
                    kind: Some(InlayHintKind::TYPE),
                    label: InlayHintLabel::String(format!(": {type_string}")),
                    position,
                    text_edits: None,
                    tooltip: None,
                    data: None,
                    padding_left: None,
                    padding_right: None,
                }]
            } else {
                vec![]
            }
        }
        Expr::VarDecl(var_id) => {
            let position = span_to_range(&expr.span, file, *offset).end;
            // skip if the type is already specified in code
            if file
                .get_content(Some(Range {
                    start: position,
                    end: Position {
                        line: position.line,
                        character: position.character + 1,
                    },
                }))
                .contains(':')
            {
                return vec![];
            }
            let var = working_set.get_variable(*var_id);
            let type_string = type_short_name(&var.ty);
            vec![InlayHint {
                kind: Some(InlayHintKind::TYPE),
                label: InlayHintLabel::String(format!(": {type_string}")),
                position,
                text_edits: None,
                tooltip: None,
                data: None,
                padding_left: None,
                padding_right: None,
            }]
        }
        Expr::Call(call) => {
            let decl = working_set.get_decl(call.decl_id);
            // skip those defined outside of the project
            let Some(block_id) = decl.block_id() else {
                return vec![];
            };
            if working_set.get_block(block_id).span.is_none() {
                return vec![];
            };
            let signatures = decl.signature();
            let signatures = [
                signatures.required_positional,
                signatures.optional_positional,
            ]
            .concat();
            let arguments = &call.arguments;
            let mut sig_idx = 0;
            let mut hints = Vec::new();
            for arg in arguments {
                match arg {
                    // skip the rest when spread/unknown arguments encountered
                    Argument::Spread(_) | Argument::Unknown(_) => {
                        sig_idx = signatures.len();
                        continue;
                    }
                    Argument::Positional(_) => {
                        if let Some(sig) = signatures.get(sig_idx) {
                            sig_idx += 1;
                            let position = span_to_range(&arg.span(), file, *offset).start;
                            hints.push(InlayHint {
                                kind: Some(InlayHintKind::PARAMETER),
                                label: InlayHintLabel::String(format!("{}:", sig.name)),
                                position,
                                text_edits: None,
                                tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("`{}: {}`", sig.shape, sig.desc),
                                })),
                                data: None,
                                padding_left: None,
                                padding_right: None,
                            });
                        }
                    }
                    // skip current for flags
                    _ => {
                        continue;
                    }
                }
            }
            hints
        }
        _ => vec![],
    }
}

impl LanguageServer {
    pub(crate) fn get_inlay_hints(&mut self, params: &InlayHintParams) -> Option<Vec<InlayHint>> {
        self.inlay_hints.get(&params.text_document.uri).cloned()
    }

    pub(crate) fn extract_inlay_hints(
        working_set: &StateWorkingSet,
        block: &Arc<Block>,
        offset: usize,
        file: &FullTextDocument,
    ) -> Vec<InlayHint> {
        let closure = |e| extract_inlay_hints_from_expression(e, working_set, &offset, file);
        let mut results = Vec::new();
        block.flat_map(working_set, &closure, &mut results);
        results
    }
}

