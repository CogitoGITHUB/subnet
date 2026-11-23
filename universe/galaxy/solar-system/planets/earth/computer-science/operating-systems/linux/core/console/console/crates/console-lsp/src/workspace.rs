use crate::{
    Id, LanguageServer,
    ast::{self, find_id, find_reference_by_id},
    path_to_uri, span_to_range, uri_to_path,
};
use lsp_textdocument::FullTextDocument;
use lsp_types::{
    DocumentHighlight, DocumentHighlightKind, DocumentHighlightParams, Location,
    PrepareRenameResponse, ProgressToken, Range, ReferenceParams, RenameParams,
    TextDocumentPositionParams, TextEdit, Uri, WorkspaceEdit, WorkspaceFolder,
};
use miette::{IntoDiagnostic, Result, miette};
use console_protocol::{
    Span,
    engine::{EngineState, StateWorkingSet},
};
use console_utils::time::Instant;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::Path,
    sync::Arc,
};

/// Message type indicating ranges of interest in each doc
#[derive(Debug)]
pub(crate) struct RangePerDoc {
    pub uri: Uri,
    pub ranges: Vec<Range>,
}

/// Message sent from background thread to main
#[derive(Debug)]
pub(crate) enum InternalMessage {
    RangeMessage(RangePerDoc),
    Cancelled(ProgressToken),
    Finished(ProgressToken),
    OnGoing(ProgressToken, u32),
}

fn find_nu_scripts_in_folder(folder_uri: &Uri) -> Result<Vec<std::path::PathBuf>> {
    let path = uri_to_path(folder_uri);
    if !path.is_dir() {
        return Err(miette!("\nworkspace folder does not exist."));
    }

    let mut out = Vec::new();
    if console_experimental::DC_GLOB.get() {
        let iter = console_glob::dc_glob::glob_from(&path, "**/*.nu")
            .map_err(|err| miette!(err.to_string()))?;
        for entry in iter {
            // dc-glob returns paths relative to the folder it walked; a file URI needs the
            // absolute path the legacy backend returns.
            out.push(path.join(entry.map_err(|err| miette!(err.to_string()))?));
        }
    } else {
        let pattern = format!("{}/**/*.nu", path.to_string_lossy());
        let iter = console_glob::glob(&pattern, console_glob::Uninterruptible)
            .map_err(|err| miette!(err.to_string()))?;
        for entry in iter {
            out.push(entry.map_err(|err| miette!(err.to_string()))?);
        }
    }
    Ok(out)
}

/// HACK: when current file is imported (use keyword) by others in the workspace,
/// it will get parsed a second time via `parse_module_block`, so that its definitions'
/// ids are renewed, making it harder to track the references.
///
/// FIXME: cross-file shadowing can still cause false-positive/false-negative cases
///
/// This is a workaround to track the new id
struct IDTracker {
    /// ID to search, renewed on `parse_module_block`
    pub id: Id,
    /// Span of the original instance under the cursor
    pub span: Span,
    /// Name of the definition
    pub name: Box<[u8]>,
    /// Span of the original file where the request comes from
    pub file_span: Span,
    /// The redundant parsing should only happen once
    pub renewed: bool,
}

impl IDTracker {
    fn new(id: Id, span: Span, file_span: Span, working_set: &StateWorkingSet) -> Self {
        let name = match &id {
            Id::Variable(_, name) | Id::Module(_, name) => name.clone(),
            // NOTE: search by the canonical command name, some weird aliasing will be missing
            Id::Declaration(decl_id) => working_set.get_decl(*decl_id).name().as_bytes().into(),
            _ => working_set.get_span_contents(span).into(),
        };
        Self {
            id,
            span,
            name,
            file_span,
            renewed: false,
        }
    }
}

impl LanguageServer {
    /// Get initial workspace folders from initialization response
    pub(crate) fn initialize_workspace_folders(
        &mut self,
        init_params: serde_json::Value,
    ) -> Option<()> {
        if let Some(array) = init_params.get("workspaceFolders") {
            let folders: Vec<WorkspaceFolder> = serde_json::from_value(array.clone()).ok()?;
            for folder in folders {
                self.workspace_folders.insert(folder.name.clone(), folder);
            }
        }
        Some(())
    }

    /// Highlight all occurrences of the text at cursor, in current file
    pub(crate) fn document_highlight(
        &mut self,
        params: &DocumentHighlightParams,
    ) -> Option<Vec<DocumentHighlight>> {
        let path_uri = &params.text_document_position_params.text_document.uri;
        let mut engine_state = self.new_engine_state(Some(path_uri));
        let (block, file_span, working_set) =
            self.parse_file(&mut engine_state, path_uri, false)?;
        let docs = &self.docs.lock().ok()?;
        let file = docs.get_document(path_uri)?;
        let location = file.offset_at(params.text_document_position_params.position) as usize
            + file_span.start;
        let (id, cursor_span) = find_id(&block, &working_set, &location)?;
        let mut refs = find_reference_by_id(&block, &working_set, &id);
        let definition_span = Self::find_definition_span_by_id(&working_set, &id);
        if let Some(extra_span) =
            Self::reference_not_in_ast(&id, &working_set, definition_span, file_span, cursor_span)
            && !refs.contains(&extra_span)
        {
            refs.push(extra_span);
        }
        Some(
            refs.iter()
                .map(|span| DocumentHighlight {
                    range: span_to_range(span, file, file_span.start),
                    kind: Some(DocumentHighlightKind::TEXT),
                })
                .collect(),
        )
    }

    /// The rename request only happens after the client received a `PrepareRenameResponse`,
    /// and a new name typed in, could happen before ranges ready for all files in the workspace folder
    pub(crate) fn rename(&mut self, params: &RenameParams) -> Option<WorkspaceEdit> {
        // changes in WorkspaceEdit have mutable key
        #[allow(clippy::mutable_key_type)]
        let changes: HashMap<Uri, Vec<TextEdit>> = self
            .occurrences
            .iter()
            .map(|(uri, ranges)| {
                (
                    uri.clone(),
                    ranges
                        .iter()
                        .map(|range| TextEdit {
                            range: *range,
                            new_text: params.new_name.to_owned(),
                        })
                        .collect(),
                )
            })
            .collect();
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    }

    /// Goto references response
    /// # Arguments
    /// - `timeout`: timeout in milliseconds, when timeout
    ///    1. Respond with all ranges found so far
    ///    2. Cancel the background thread
    pub(crate) fn references(
        &mut self,
        params: &ReferenceParams,
        timeout: u128,
    ) -> Option<Vec<Location>> {
        self.occurrences = BTreeMap::new();
        // start with a clean engine state
        self.need_parse = true;
        let path_uri = &params.text_document_position.text_document.uri;
        let mut engine_state = self.new_engine_state(Some(path_uri));

        let (mut working_set, id, span, file_span) = self
            .parse_and_find(
                &mut engine_state,
                path_uri,
                params.text_document_position.position,
            )
            .ok()?;

        let mut id_tracker = IDTracker::new(id.clone(), span, file_span, &working_set);
        let Some(workspace_uri) = self
            .get_workspace_folder_by_uri(path_uri)
            .map(|folder| folder.uri.clone())
        else {
            let definition_span = Self::find_definition_span_by_id(&working_set, &id);
            return Some(
                Self::find_reference_in_file(
                    &mut working_set,
                    self.docs.lock().ok()?.get_document(path_uri)?,
                    uri_to_path(path_uri).as_path(),
                    &mut id_tracker,
                    definition_span,
                )
                .into_iter()
                .map(|range| Location {
                    uri: path_uri.clone(),
                    range,
                })
                .collect(),
            );
        };

        let token = params
            .work_done_progress_params
            .work_done_token
            .clone()
            .unwrap_or(ProgressToken::Number(1));

        // make sure the parsing result of current file is merged in the state
        let engine_state = self.new_engine_state(Some(path_uri));
        self.channels = self
            .find_reference_in_workspace(
                engine_state,
                workspace_uri,
                token.clone(),
                "Finding references ...".to_string(),
                id_tracker,
            )
            .ok();
        // TODO: WorkDoneProgress -> PartialResults for quicker response
        // currently not enabled by `lsp_types` but hackable in `server_capabilities` json
        let time_start = Instant::now();
        loop {
            if self.handle_internal_messages().ok()? {
                break;
            }
            if time_start.elapsed().as_millis() > timeout {
                self.send_progress_end(token, Some("Timeout".to_string()))
                    .ok()?;
                self.cancel_background_thread();
                self.channels = None;
                break;
            }
        }
        Some(
            self.occurrences
                .iter()
                .flat_map(|(uri, ranges)| {
                    ranges.iter().map(|range| Location {
                        uri: uri.clone(),
                        range: *range,
                    })
                })
                .collect(),
        )
    }

    /// 1. Parse current file to find the content at the cursor that is suitable for a workspace wide renaming
    /// 2. Parse all nu scripts in the same workspace folder, with the variable/command name in it.
    /// 3. Store the results in `self.occurrences` for later rename quest
    pub(crate) fn prepare_rename(&mut self, request: lsp_server::Request) -> Result<()> {
        let params: TextDocumentPositionParams =
            serde_json::from_value(request.params).into_diagnostic()?;
        self.occurrences = BTreeMap::new();
        // start with a clean engine state
        self.need_parse = true;

        let path_uri = &params.text_document.uri;
        let mut engine_state = self.new_engine_state(Some(path_uri));

        let (mut working_set, id, span, file_span) =
            self.parse_and_find(&mut engine_state, path_uri, params.position)?;

        if let Id::Value(_) = id {
            return Err(miette!("\nRename only works for variable/command."));
        }
        if Self::find_definition_span_by_id(&working_set, &id).is_none() {
            return Err(miette!(
                "\nDefinition not found.\nNot allowed to rename built-ins."
            ));
        }

        let docs = match self.docs.lock() {
            Ok(it) => it,
            Err(err) => return Err(miette!(err.to_string())),
        };
        let file = docs
            .get_document(path_uri)
            .ok_or_else(|| miette!("\nFailed to get document"))?;
        let range = span_to_range(&span, file, file_span.start);
        let response = PrepareRenameResponse::Range(range);
        self.connection
            .sender
            .send(lsp_server::Message::Response(lsp_server::Response {
                id: request.id,
                response_result: serde_json::to_value(response).map_err(|e| {
                    lsp_server::ResponseError {
                        code: 0,
                        message: e.to_string(),
                        data: None,
                    }
                }),
            }))
            .into_diagnostic()?;

        let mut id_tracker = IDTracker::new(id.clone(), span, file_span, &working_set);
        let Some(workspace_uri) = self
            .get_workspace_folder_by_uri(path_uri)
            .map(|folder| folder.uri.clone())
        else {
            let definition_span = Self::find_definition_span_by_id(&working_set, &id);
            self.occurrences.insert(
                path_uri.clone(),
                Self::find_reference_in_file(
                    &mut working_set,
                    file,
                    uri_to_path(path_uri).as_path(),
                    &mut id_tracker,
                    definition_span,
                ),
            );
            return Ok(());
        };
        // now continue parsing on other files in the workspace
        // make sure the parsing result of current file is merged in the state
        let engine_state = self.new_engine_state(Some(path_uri));
        self.channels = self
            .find_reference_in_workspace(
                engine_state,
                workspace_uri,
                ProgressToken::Number(0),
                "Preparing rename ...".to_string(),
                id_tracker,
            )
            .ok();
        Ok(())
    }

    fn find_reference_in_file(
        working_set: &mut StateWorkingSet,
        file: &FullTextDocument,
        fp: &Path,
        id_tracker: &mut IDTracker,
        definition_span: Option<Span>,
    ) -> Vec<Range> {
        let block = console_parser::parse(
            working_set,
            fp.to_str(),
            file.get_content(None).as_bytes(),
            false,
        );
        // NOTE: Renew the id if there's a module with the same span as the original file.
        // This requires that the initial parsing results get merged in the engine_state.
        // Pay attention to the `self.need_parse = true` and `merge_delta` assignments
        // in function `prepare_rename`/`references`
        if (!id_tracker.renewed)
            && working_set
                .find_module_by_span(id_tracker.file_span)
                .is_some()
        {
            if let Some(new_block) = working_set.find_block_by_span(id_tracker.file_span)
                && let Some((new_id, _)) =
                    ast::find_id(&new_block, working_set, &id_tracker.span.start)
            {
                id_tracker.id = new_id;
            }
            id_tracker.renewed = true;
        }
        let mut refs: Vec<Span> = find_reference_by_id(&block, working_set, &id_tracker.id);

        let file_span = working_set
            .get_span_for_filename(fp.to_string_lossy().as_ref())
            .unwrap_or(Span::unknown());
        if let Some(extra_span) = Self::reference_not_in_ast(
            &id_tracker.id,
            working_set,
            definition_span,
            file_span,
            id_tracker.span,
        ) && !refs.contains(&extra_span)
        {
            refs.push(extra_span)
        }

        // add_block to avoid repeated parsing
        working_set.add_block(block);
        refs.iter()
            .map(|span| span_to_range(span, file, file_span.start))
            .collect()
    }

    /// NOTE: for arguments whose declaration is in a signature
    /// which is not covered in the AST
    fn reference_not_in_ast(
        id: &Id,
        working_set: &StateWorkingSet,
        definition_span: Option<Span>,
        file_span: Span,
        sample_span: Span,
    ) -> Option<Span> {
        if let (Id::Variable(_, name_ref), Some(decl_span)) = (&id, definition_span)
            && file_span.contains_span(decl_span)
            && decl_span.end > decl_span.start
        {
            let content = working_set.get_span_contents(decl_span);
            let leading_dashes = content
                .iter()
                // remove leading dashes for flags
                .take_while(|c| *c == &b'-')
                .count();
            let start = decl_span.start + leading_dashes;
            return content.get(leading_dashes..).and_then(|name| {
                name.starts_with(name_ref).then_some(Span {
                    start,
                    end: start + sample_span.end - sample_span.start,
                })
            });
        }
        None
    }

    /// Time consuming task running in a background thread
    /// communicating with the main thread using `InternalMessage`
    fn find_reference_in_workspace(
        &self,
        engine_state: EngineState,
        workspace_uri: Uri,
        token: ProgressToken,
        message: String,
        mut id_tracker: IDTracker,
    ) -> Result<(
        crossbeam_channel::Sender<bool>,
        Arc<crossbeam_channel::Receiver<InternalMessage>>,
    )> {
        let (data_sender, data_receiver) = crossbeam_channel::unbounded::<InternalMessage>();
        let (cancel_sender, cancel_receiver) = crossbeam_channel::bounded::<bool>(1);
        let engine_state = Arc::new(engine_state);
        let text_documents = self.docs.clone();
        self.send_progress_begin(token.clone(), message)?;

        std::thread::spawn(move || -> Result<()> {
            let mut working_set = StateWorkingSet::new(&engine_state);
            let mut scripts: HashSet<_> = match find_nu_scripts_in_folder(&workspace_uri) {
                Ok(it) => it,
                Err(_) => {
                    data_sender
                        .send(InternalMessage::Cancelled(token.clone()))
                        .ok();
                    return Ok(());
                }
            }
            .into_iter()
            .collect();

            // For unsaved new files
            let mut opened_scripts = HashSet::new();
            let docs = match text_documents.lock() {
                Ok(it) => it,
                Err(err) => return Err(miette!(err.to_string())),
            };
            for uri in docs.documents().keys() {
                let fp = uri_to_path(uri);
                opened_scripts.insert(fp.clone());
                scripts.insert(fp);
            }
            drop(docs);

            let len = scripts.len();
            let definition_span = Self::find_definition_span_by_id(&working_set, &id_tracker.id);
            let bytes_to_search = id_tracker.name.to_owned();
            let finder = memchr::memmem::Finder::new(&bytes_to_search);

            for (i, fp) in scripts.iter().enumerate() {
                // cancel the loop on cancellation message from main thread
                if cancel_receiver.try_recv().is_ok() {
                    data_sender
                        .send(InternalMessage::Cancelled(token.clone()))
                        .into_diagnostic()?;
                    return Ok(());
                }
                let percentage = (i * 100 / len) as u32;
                let uri = path_to_uri(fp);
                let file = if opened_scripts.contains(fp) {
                    let docs = match text_documents.lock() {
                        Ok(it) => it,
                        Err(err) => return Err(miette!(err.to_string())),
                    };
                    let Some(file) = docs.get_document(&uri) else {
                        continue;
                    };
                    let doc_copy =
                        FullTextDocument::new("nu".to_string(), 0, file.get_content(None).into());
                    drop(docs);
                    doc_copy
                } else {
                    let file_bytes = match fs::read(fp) {
                        Ok(it) => it,
                        Err(_) => {
                            // continue on fs error
                            continue;
                        }
                    };
                    // skip if the file does not contain what we're looking for
                    if finder.find(&file_bytes).is_none() {
                        // progress without any data
                        data_sender
                            .send(InternalMessage::OnGoing(token.clone(), percentage))
                            .into_diagnostic()?;
                        continue;
                    }
                    FullTextDocument::new(
                        "nu".to_string(),
                        0,
                        String::from_utf8_lossy(&file_bytes).into(),
                    )
                };
                let ranges = Self::find_reference_in_file(
                    &mut working_set,
                    &file,
                    fp,
                    &mut id_tracker,
                    definition_span,
                );
                data_sender
                    .send(InternalMessage::RangeMessage(RangePerDoc { uri, ranges }))
                    .ok();
                data_sender
                    .send(InternalMessage::OnGoing(token.clone(), percentage))
                    .ok();
            }
            data_sender
                .send(InternalMessage::Finished(token))
                .into_diagnostic()
        });
        Ok((cancel_sender, Arc::new(data_receiver)))
    }

    fn get_workspace_folder_by_uri(&self, uri: &Uri) -> Option<&WorkspaceFolder> {
        let uri_string = uri.to_string();
        self.workspace_folders.iter().find_map(|(_, folder)| {
            uri_string
                .starts_with(&folder.uri.to_string())
                .then_some(folder)
        })
    }
}

