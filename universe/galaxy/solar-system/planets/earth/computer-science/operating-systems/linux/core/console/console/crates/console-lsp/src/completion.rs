use crate::{LanguageServer, span_to_range, uri_to_path};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionParams,
    CompletionResponse, CompletionTextEdit, Documentation, InsertTextFormat, MarkupContent,
    MarkupKind, Range, TextEdit,
};
use console_cli::{CompletionEngine, SemanticSuggestion, SuggestionKind};
use console_protocol::{
    PositionalArg, Span, SyntaxShape,
    engine::{CommandType, EngineState, Stack},
};

impl LanguageServer {
    pub(crate) fn complete(&mut self, params: &CompletionParams) -> Option<CompletionResponse> {
        let path_uri = &params.text_document_position.text_document.uri;
        let docs = self.docs.lock().ok()?;
        let file = docs.get_document(path_uri)?;
        let location = file.offset_at(params.text_document_position.position) as usize;
        let file_text = file.get_content(None).to_owned();
        drop(docs);
        // fallback to default completer where
        // the text is truncated to `location` and
        // an extra placeholder token is inserted for correct parsing
        let is_variable = file_text
            .get(..location)
            .and_then(|s| s.rsplit(' ').next())
            .is_some_and(|last_word| last_word.starts_with('$'));
        let need_fallback = location == 0
            || is_variable
            || file_text
                .get(location - 1..location)
                .and_then(|s| s.chars().next())
                .is_some_and(|c| c.is_whitespace() || "|(){}[]<>,:;".contains(c));

        self.need_parse |= need_fallback;
        let engine_state = self.new_engine_state(Some(path_uri));
        let completer = CompletionEngine::new(&engine_state, &Stack::new());
        let results = if need_fallback {
            completer.fetch_completions_at(&file_text[..location], location)
        } else {
            let file_path = uri_to_path(path_uri);
            let filename = file_path.to_str()?;
            completer.fetch_completions_within_file(filename, location, &file_text)
        };

        let docs = self.docs.lock().ok()?;
        let file = docs.get_document(path_uri)?;
        (!results.is_empty()).then_some(CompletionResponse::Array(
            results
                .into_iter()
                .map(|r| {
                    let reedline_span = r.suggestion.span;
                    Self::completion_item_from_suggestion(
                        &engine_state,
                        r,
                        span_to_range(&Span::new(reedline_span.start, reedline_span.end), file, 0),
                    )
                })
                .collect(),
        ))
    }

    fn completion_item_from_suggestion(
        engine_state: &EngineState,
        suggestion: SemanticSuggestion,
        range: Range,
    ) -> CompletionItem {
        let mut snippet_text = suggestion.suggestion.value.clone();
        let mut doc_string = suggestion.suggestion.extra.map(|ex| ex.join("\n"));
        let mut insert_text_format = None;
        let mut idx = 0;
        // use snippet as `insert_text_format` for command argument completion
        if let Some(SuggestionKind::Command(_, Some(decl_id))) = suggestion.kind {
            // NOTE: for new commands defined in current context,
            // which are not present in the engine state, skip the documentation and snippet.
            if engine_state.num_decls() > decl_id.get() {
                let cmd = engine_state.get_decl(decl_id);
                doc_string = Some(Self::get_decl_description(cmd, true));
                insert_text_format = Some(InsertTextFormat::SNIPPET);
                let signature = cmd.signature();
                // add curly brackets around block arguments
                // and keywords, e.g. `=` in `alias foo = bar`
                let mut arg_wrapper = |arg: &PositionalArg,
                                       text: String,
                                       optional: bool|
                 -> String {
                    idx += 1;
                    match &arg.shape {
                        SyntaxShape::Block | SyntaxShape::MatchBlock => {
                            format!("{{ ${{{idx}:{text}}} }}")
                        }
                        SyntaxShape::Keyword(kwd, _) => {
                            // NOTE: If optional, the keyword should also be in a placeholder so that it can be removed easily.
                            // Here we choose to use nested placeholders. Note that some editors don't fully support this format,
                            // but usually they will simply ignore the inner ones, so it should be fine.
                            if optional {
                                idx += 1;
                                format!(
                                    "${{{}:{} ${{{}:{}}}}}",
                                    idx - 1,
                                    String::from_utf8_lossy(kwd),
                                    idx,
                                    text
                                )
                            } else {
                                format!("{} ${{{}:{}}}", String::from_utf8_lossy(kwd), idx, text)
                            }
                        }
                        _ => format!("${{{idx}:{text}}}"),
                    }
                };

                for required in signature.required_positional {
                    snippet_text.push(' ');
                    snippet_text
                        .push_str(arg_wrapper(&required, required.name.clone(), false).as_str());
                }
                for optional in signature.optional_positional {
                    snippet_text.push(' ');
                    snippet_text.push_str(
                        arg_wrapper(&optional, format!("{}?", optional.name), true).as_str(),
                    );
                }
                if let Some(rest) = signature.rest_positional {
                    idx += 1;
                    snippet_text.push_str(format!(" ${{{}:...{}}}", idx, rest.name).as_str());
                }
            }
        }
        // no extra space for a command with args expanded in the snippet
        if idx == 0 && suggestion.suggestion.append_whitespace {
            snippet_text.push(' ');
        }

        let text_edit = Some(CompletionTextEdit::Edit(TextEdit {
            range,
            new_text: snippet_text,
        }));

        CompletionItem {
            label: suggestion.suggestion.value,
            label_details: suggestion
                .kind
                .as_ref()
                .map(|kind| match kind {
                    SuggestionKind::Value(t) => t.to_string(),
                    SuggestionKind::Command(cmd, _) => cmd.to_string(),
                    SuggestionKind::Module => "module".to_string(),
                    SuggestionKind::Operator => "operator".to_string(),
                    SuggestionKind::Variable => "variable".to_string(),
                    SuggestionKind::Flag => "flag".to_string(),
                    _ => String::new(),
                })
                .map(|s| CompletionItemLabelDetails {
                    detail: None,
                    description: Some(s),
                }),
            detail: suggestion.suggestion.description,
            documentation: doc_string.map(|value| {
                Documentation::MarkupContent(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                })
            }),
            kind: Self::lsp_completion_item_kind(suggestion.kind),
            text_edit,
            insert_text_format,
            ..Default::default()
        }
    }

    fn lsp_completion_item_kind(
        suggestion_kind: Option<SuggestionKind>,
    ) -> Option<CompletionItemKind> {
        suggestion_kind.and_then(|suggestion_kind| match suggestion_kind {
            SuggestionKind::Value(t) => match t {
                console_protocol::Type::String => Some(CompletionItemKind::VALUE),
                _ => None,
            },
            SuggestionKind::CellPath => Some(CompletionItemKind::PROPERTY),
            SuggestionKind::Command(c, _) => match c {
                CommandType::Keyword => Some(CompletionItemKind::KEYWORD),
                CommandType::Builtin => Some(CompletionItemKind::FUNCTION),
                CommandType::Alias => Some(CompletionItemKind::REFERENCE),
                CommandType::External => Some(CompletionItemKind::INTERFACE),
                CommandType::Custom | CommandType::Plugin => Some(CompletionItemKind::METHOD),
            },
            SuggestionKind::Directory => Some(CompletionItemKind::FOLDER),
            SuggestionKind::File => Some(CompletionItemKind::FILE),
            SuggestionKind::Flag => Some(CompletionItemKind::FIELD),
            SuggestionKind::Module => Some(CompletionItemKind::MODULE),
            SuggestionKind::Operator => Some(CompletionItemKind::OPERATOR),
            SuggestionKind::Variable => Some(CompletionItemKind::VARIABLE),
        })
    }
}

