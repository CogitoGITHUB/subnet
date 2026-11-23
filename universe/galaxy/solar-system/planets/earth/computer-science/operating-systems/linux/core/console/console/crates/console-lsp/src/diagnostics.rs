use crate::{LanguageServer, span_to_range};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, PublishDiagnosticsParams, Uri,
    notification::{Notification, PublishDiagnostics},
};
use miette::{IntoDiagnostic, Result, miette};

impl LanguageServer {
    pub(crate) fn publish_diagnostics_for_file(&mut self, uri: Uri) -> Result<()> {
        let mut engine_state = self.new_engine_state(Some(&uri));
        engine_state.generate_nu_constant();

        let Some((_, span, working_set)) = self.parse_file(&mut engine_state, &uri, true) else {
            return Ok(());
        };

        let mut diagnostics = PublishDiagnosticsParams {
            uri: uri.clone(),
            diagnostics: Vec::new(),
            version: None,
        };

        let docs = match self.docs.lock() {
            Ok(it) => it,
            Err(err) => return Err(miette!(err.to_string())),
        };
        let file = docs
            .get_document(&uri)
            .ok_or_else(|| miette!("\nFailed to get document"))?;
        for err in working_set.parse_errors.iter() {
            let message = err.to_string();

            diagnostics.diagnostics.push(Diagnostic {
                range: span_to_range(&err.span(), file, span.start),
                severity: Some(DiagnosticSeverity::ERROR),
                message,
                ..Default::default()
            });
        }

        for warn in working_set.parse_warnings.iter() {
            let message = warn.to_string();

            diagnostics.diagnostics.push(Diagnostic {
                range: span_to_range(&warn.span(), file, span.start),
                severity: Some(DiagnosticSeverity::WARNING),
                message,
                ..Default::default()
            });
        }

        self.connection
            .sender
            .send(lsp_server::Message::Notification(
                lsp_server::Notification::new(PublishDiagnostics::METHOD.to_string(), diagnostics),
            ))
            .into_diagnostic()
    }
}

