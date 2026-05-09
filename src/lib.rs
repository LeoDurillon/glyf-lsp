pub mod helper;
pub mod snippet_store;

use std::sync::LazyLock;

use dashmap::DashMap;
use glyf_core::parser::GlyfError;
use regex::Regex;
use tower_lsp::{
    Client, LanguageServer,
    jsonrpc::Result,
    lsp_types::{
        CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams,
        CompletionResponse, CompletionTextEdit, DidChangeTextDocumentParams,
        DidOpenTextDocumentParams, Documentation, InitializeParams, InitializeResult,
        InitializedParams, InsertTextFormat, MessageType, Range, ServerCapabilities,
        TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
    },
};

pub use snippet_store::SnippetStore;

use crate::helper::{abbreviation_range, extract_abbreviation, insert_tabstops};

pub struct GlyfLsp {
    pub client: Client,
    pub documents: DashMap<String, String>,
    pub snippets: SnippetStore,
}

const TRIGGER_CHARACTERS: &[&str] = &[".", ":", ">", "+", "*", "("];
static PLACEHOLDER_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{\d+\}").unwrap());

impl GlyfLsp {
    fn create_documentation(&self, expanded: &str) -> Documentation {
        Documentation::String(PLACEHOLDER_REGEX.replace_all(expanded, "|").into_owned())
    }

    async fn log_glyf_error(&self, err: GlyfError) {
        self.client
            .log_message(MessageType::ERROR, format!("Glyf error: {}", err))
            .await;
    }

    async fn expand_abbreviation(&self, abbr: &str) -> Option<String> {
        let custom = self.snippets.to_hashmap();
        match glyf_core::expand(abbr, None, Some(&custom)) {
            Ok(html) => Some(html),
            Err(err) => {
                self.log_glyf_error(err).await;
                None
            }
        }
    }

    fn build_completion_item(&self, abbr: &str, expanded: String, range: Range) -> CompletionItem {
        CompletionItem {
            label: format!("Glyf: {}", abbr),
            kind: Some(CompletionItemKind::SNIPPET),
            documentation: Some(self.create_documentation(&expanded)),
            detail: Some("Expand Glyf abbreviation".into()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            sort_text: Some("!".to_string()),
            preselect: Some(true),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: expanded,
            })),
            ..Default::default()
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for GlyfLsp {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        self.snippets.load_from_params(&params);

        let completion_options = CompletionOptions {
            trigger_characters: Some(TRIGGER_CHARACTERS.iter().map(|&s| s.into()).collect()),
            ..Default::default()
        };

        let text_document_sync = TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL);

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                completion_provider: Some(completion_options),
                text_document_sync: Some(text_document_sync),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "glyf-lsp started")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.client
            .log_message(MessageType::INFO, "Shutting down glyf-lsp")
            .await;
        Ok(())
    }

    // Keep documents in sync so we can read them on completion
    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.documents.insert(
            params.text_document.uri.to_string(),
            params.text_document.text,
        );
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .insert(params.text_document.uri.to_string(), change.text);
        }
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri.to_string();
        let pos = params.text_document_position.position;

        let Some(content) = self.documents.get(&uri) else {
            return Ok(None);
        };

        let line = content.lines().nth(pos.line as usize).unwrap_or("");
        let abbr = extract_abbreviation(line, pos.character);

        let Some(expanded) = self.expand_abbreviation(abbr).await else {
            return Ok(None);
        };

        let range = abbreviation_range(pos, abbr.len());

        let item = self.build_completion_item(abbr, insert_tabstops(&expanded), range);

        Ok(Some(CompletionResponse::Array(vec![item])))
    }
}
