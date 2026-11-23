use std::path::Path;

use crate::{Id, LanguageServer, path_to_uri, span_to_range};
use lsp_textdocument::FullTextDocument;
use lsp_types::{GotoDefinitionParams, GotoDefinitionResponse, Location};
use console_protocol::Span;
use console_protocol::engine::{CachedFile, StateWorkingSet};

impl LanguageServer {
    fn get_location_by_span<'a>(
        &self,
        files: impl Iterator<Item = &'a CachedFile>,
        span: &Span,
    ) -> Option<Location> {
        for cached_file in files.into_iter() {
            if cached_file.covered_span.contains(span.start) {
                let path = Path::new(&*cached_file.name);
                // skip console-std files
                // TODO: maybe find them in vendor directories?
                if path.is_relative() {
                    let _ = self.send_log_message(
                        lsp_types::MessageType::WARNING,
                        format!(
                            "Location found in file {path:?}, but absolute path is expected. Skipping..."
                        ),
                    );
                    continue;
                }
                let target_uri = path_to_uri(path);
                if let Some(file) = self.docs.lock().ok()?.get_document(&target_uri) {
                    return Some(Location {
                        uri: target_uri,
                        range: span_to_range(span, file, cached_file.covered_span.start),
                    });
                } else {
                    if !path.is_file() {
                        return None;
                    }
                    // in case where the document is not opened yet,
                    // typically included by the `use/source` command
                    let temp_doc = FullTextDocument::new(
                        "nu".to_string(),
                        0,
                        String::from_utf8_lossy(cached_file.content.as_ref()).to_string(),
                    );
                    return Some(Location {
                        uri: target_uri,
                        range: span_to_range(span, &temp_doc, cached_file.covered_span.start),
                    });
                }
            }
        }
        None
    }

    pub(crate) fn find_definition_span_by_id(
        working_set: &StateWorkingSet,
        id: &Id,
    ) -> Option<Span> {
        match id {
            Id::Declaration(decl_id) => {
                let block_id = working_set.get_decl(*decl_id).block_id()?;
                working_set.get_block(block_id).span
            }
            Id::Variable(var_id, _) => {
                let var = working_set.get_variable(*var_id);
                Some(var.declaration_span)
            }
            Id::Module(module_id, _) => {
                let module = working_set.get_module(*module_id);
                module.span
            }
            Id::CellPath(var_id, cell_path) => {
                let var = working_set.get_variable(*var_id);
                Some(
                    var.const_val
                        .as_ref()
                        .and_then(|val| val.follow_cell_path(cell_path).ok())
                        .map(|val| val.span())
                        .unwrap_or(var.declaration_span),
                )
            }
            _ => None,
        }
    }

    pub(crate) fn goto_definition(
        &mut self,
        params: &GotoDefinitionParams,
    ) -> Option<GotoDefinitionResponse> {
        let path_uri = &params.text_document_position_params.text_document.uri;
        let mut engine_state = self.new_engine_state(Some(path_uri));
        let (working_set, id, _, _) = self
            .parse_and_find(
                &mut engine_state,
                path_uri,
                params.text_document_position_params.position,
            )
            .ok()?;

        Some(GotoDefinitionResponse::Scalar(self.get_location_by_span(
            working_set.files(),
            &Self::find_definition_span_by_id(&working_set, &id)?,
        )?))
    }
}

