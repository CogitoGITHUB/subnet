use crate::{Id, LanguageServer, path_to_uri, span_to_range, uri_to_path};
use lsp_textdocument::{FullTextDocument, TextDocuments};
use lsp_types::{
    DocumentSymbolParams, DocumentSymbolResponse, Location, Range, SymbolInformation, SymbolKind,
    Uri, WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use console_protocol::{
    DeclId, ModuleId, Span, VarId,
    engine::{CachedFile, EngineState, StateWorkingSet},
};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
    hash::{Hash, Hasher},
};

/// Struct stored in cache, uri not included
#[derive(Clone, Debug, Eq, PartialEq)]
struct Symbol {
    name: String,
    kind: SymbolKind,
    range: Range,
}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.range.start.hash(state);
        self.range.end.hash(state);
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.kind == other.kind {
            return self.range.start.cmp(&other.range.start);
        }
        match (self.kind, other.kind) {
            (SymbolKind::FUNCTION, _) => Ordering::Less,
            (_, SymbolKind::FUNCTION) => Ordering::Greater,
            _ => self.range.start.cmp(&other.range.start),
        }
    }
}

impl Symbol {
    fn to_symbol_information(&self, uri: &Uri) -> SymbolInformation {
        #[allow(deprecated)]
        SymbolInformation {
            location: Location {
                uri: uri.clone(),
                range: self.range,
            },
            name: self.name.to_owned(),
            kind: self.kind,
            container_name: None,
            deprecated: None,
            tags: None,
        }
    }
}

/// Cache symbols for each opened file to avoid repeated parsing
pub(crate) struct SymbolCache {
    /// Fuzzy matcher for symbol names
    matcher: Matcher,
    /// File Uri --> Symbols
    cache: BTreeMap<Uri, Vec<Symbol>>,
    /// If marked as dirty, parse on next request
    dirty_flags: BTreeMap<Uri, bool>,
}

impl SymbolCache {
    pub fn new() -> Self {
        SymbolCache {
            matcher: Matcher::new({
                let mut cfg = Config::DEFAULT;
                cfg.prefer_prefix = true;
                cfg
            }),
            cache: BTreeMap::new(),
            dirty_flags: BTreeMap::new(),
        }
    }

    pub fn mark_dirty(&mut self, uri: Uri, flag: bool) {
        self.dirty_flags.insert(uri, flag);
    }

    fn get_symbol_by_id(
        working_set: &StateWorkingSet,
        id: Id,
        doc: &FullTextDocument,
        doc_span: &Span,
    ) -> Option<Symbol> {
        match id {
            Id::Declaration(decl_id) => {
                let decl = working_set.get_decl(decl_id);
                let span = working_set.get_block(decl.block_id()?).span?;
                // multi-doc working_set, returns None if the Id is in other files
                if !doc_span.contains(span.start) {
                    return None;
                }
                Some(Symbol {
                    name: decl.name().to_string(),
                    kind: SymbolKind::FUNCTION,
                    range: span_to_range(&span, doc, doc_span.start),
                })
            }
            Id::Variable(var_id, _) => {
                let var = working_set.get_variable(var_id);
                let span = var.declaration_span;
                if !doc_span.contains(span.start) || span.end == span.start {
                    return None;
                }
                let range = span_to_range(&span, doc, doc_span.start);
                let name = doc.get_content(Some(range));
                Some(Symbol {
                    name: name.to_string(),
                    kind: SymbolKind::VARIABLE,
                    range,
                })
            }
            Id::Module(module_id, _) => {
                let module = working_set.get_module(module_id);
                let span = module.span?;
                if !doc_span.contains(span.start) {
                    return None;
                }
                Some(Symbol {
                    name: String::from_utf8(module.name()).ok()?,
                    kind: SymbolKind::MODULE,
                    range: span_to_range(&span, doc, doc_span.start),
                })
            }
            _ => None,
        }
    }

    fn extract_all_symbols(
        working_set: &StateWorkingSet,
        doc: &FullTextDocument,
        cached_file: &CachedFile,
    ) -> Vec<Symbol> {
        let mut all_symbols: Vec<Symbol> = (0..working_set.num_decls())
            .filter_map(|id| {
                Self::get_symbol_by_id(
                    working_set,
                    Id::Declaration(DeclId::new(id)),
                    doc,
                    &cached_file.covered_span,
                )
            })
            .chain((0..working_set.num_vars()).filter_map(|id| {
                Self::get_symbol_by_id(
                    working_set,
                    Id::Variable(VarId::new(id), [].into()),
                    doc,
                    &cached_file.covered_span,
                )
            }))
            .chain((0..working_set.num_modules()).filter_map(|id| {
                Self::get_symbol_by_id(
                    working_set,
                    Id::Module(ModuleId::new(id), [].into()),
                    doc,
                    &cached_file.covered_span,
                )
            }))
            // TODO: same variable symbol can be duplicated with different VarId
            .collect::<HashSet<Symbol>>()
            .into_iter()
            .collect();
        all_symbols.sort();
        all_symbols
    }

    /// Update the symbols of given uri if marked as dirty
    pub fn update(&mut self, uri: &Uri, engine_state: &EngineState, docs: &TextDocuments) {
        if *self.dirty_flags.get(uri).unwrap_or(&true) {
            let mut working_set = StateWorkingSet::new(engine_state);
            let content = docs
                .get_document_content(uri, None)
                .expect("Failed to get_document_content!")
                .as_bytes();
            console_parser::parse(
                &mut working_set,
                Some(
                    uri_to_path(uri)
                        .to_str()
                        .expect("Failed to convert pathbuf to string"),
                ),
                content,
                false,
            );
            for cached_file in working_set.files() {
                let path = std::path::Path::new(&*cached_file.name);
                if !path.is_file() {
                    continue;
                }
                let target_uri = path_to_uri(path);
                let new_symbols = Self::extract_all_symbols(
                    &working_set,
                    docs.get_document(&target_uri)
                        .unwrap_or(&FullTextDocument::new(
                            "nu".to_string(),
                            0,
                            String::from_utf8((*cached_file.content).to_vec())
                                .expect("Invalid UTF-8"),
                        )),
                    cached_file,
                );
                self.cache.insert(target_uri.clone(), new_symbols);
                self.mark_dirty(target_uri, false);
            }
            self.mark_dirty(uri.clone(), false);
        };
    }

    pub fn drop(&mut self, uri: &Uri) {
        self.cache.remove(uri);
        self.dirty_flags.remove(uri);
    }

    pub fn update_all(&mut self, engine_state: &EngineState, docs: &TextDocuments) {
        for uri in docs.documents().keys() {
            self.update(uri, engine_state, docs);
        }
    }

    pub fn get_symbols_by_uri(&self, uri: &Uri) -> Option<Vec<SymbolInformation>> {
        Some(
            self.cache
                .get(uri)?
                .iter()
                .map(|s| s.to_symbol_information(uri))
                .collect(),
        )
    }

    pub fn get_fuzzy_matched_symbols(&mut self, query: &str) -> Vec<SymbolInformation> {
        let pat = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        self.cache
            .iter()
            .flat_map(|(uri, symbols)| symbols.iter().map(|s| s.to_symbol_information(uri)))
            .filter_map(|s| {
                let mut buf = Vec::new();
                let name = Utf32Str::new(&s.name, &mut buf);
                pat.score(name, &mut self.matcher)?;
                Some(s)
            })
            .collect()
    }

    pub fn any_dirty(&self) -> bool {
        self.dirty_flags.values().any(|f| *f)
    }
}

impl LanguageServer {
    pub(crate) fn document_symbol(
        &mut self,
        params: &DocumentSymbolParams,
    ) -> Option<DocumentSymbolResponse> {
        let uri = &params.text_document.uri;
        let engine_state = self.new_engine_state(Some(uri));
        let docs = self.docs.lock().ok()?;
        docs.get_document(uri)?;
        self.symbol_cache.update(uri, &engine_state, &docs);
        self.symbol_cache
            .get_symbols_by_uri(uri)
            .map(DocumentSymbolResponse::Flat)
    }

    pub(crate) fn workspace_symbol(
        &mut self,
        params: &WorkspaceSymbolParams,
    ) -> Option<WorkspaceSymbolResponse> {
        if self.symbol_cache.any_dirty() {
            let engine_state = self.new_engine_state(None);
            let docs = self.docs.lock().ok()?;
            self.symbol_cache.update_all(&engine_state, &docs);
        }
        Some(WorkspaceSymbolResponse::Flat(
            self.symbol_cache
                .get_fuzzy_matched_symbols(params.query.as_str()),
        ))
    }
}

