mod consts;
pub mod helper;
pub mod snippet_store;

use std::{collections::HashMap, sync::LazyLock};

use dashmap::DashMap;
use glyf_core::{compress, config::Config, parser::GlyfError};
use regex::Regex;
use tower_lsp::{
    Client, LanguageServer,
    jsonrpc::Result,
    lsp_types::{
        CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams,
        CodeActionProviderCapability, CodeActionResponse, CompletionItem, CompletionItemKind,
        CompletionOptions, CompletionParams, CompletionResponse, CompletionTextEdit,
        DidChangeTextDocumentParams, DidOpenTextDocumentParams, Documentation, InitializeParams,
        InitializeResult, InitializedParams, InsertTextFormat, MessageType, Position, Range,
        ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit,
        WorkspaceEdit,
    },
};

pub use snippet_store::SnippetStore;

use crate::helper::{
    abbreviation_range, compute_tag_opening_closing_range, extract_abbreviation, extract_range,
    insert_tabstops,
};

pub struct GlyfLsp {
    pub client: Client,
    pub documents: DashMap<String, String>,
    pub snippets: SnippetStore,
}

const TRIGGER_CHARACTERS: &[&str] = &[".", ":", ">", "+", "*", "("];
static PLACEHOLDER_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{\d+\}").unwrap());
static ONE_LINE_TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^<.+/>$|^<.+?>[\w\s]+</\w+>$)").unwrap());
static HTML_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^<.+)").unwrap());

impl GlyfLsp {
    fn create_documentation(&self, expanded: &str) -> Documentation {
        Documentation::String(PLACEHOLDER_REGEX.replace_all(expanded, "|").into_owned())
    }

    async fn log_glyf_error(&self, err: GlyfError) {
        self.client
            .log_message(MessageType::ERROR, format!("Glyf error: {}", err))
            .await;
    }

    async fn expand_abbreviation(&self, abbr: &str, uri: String) -> Option<String> {
        let mode = match uri.split('.').next_back() {
            Some("jsx") | Some("tsx") => glyf_core::config::ParserMode::JSX,
            _ => glyf_core::config::ParserMode::HTML,
        };

        match glyf_core::expand(
            abbr,
            None,
            Some(Config::new(mode, self.snippets.to_hashmap())),
        ) {
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
            filter_text: Some(abbr.to_string()),
            sort_text: Some("~".to_string()),
            preselect: Some(true),
            text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                range,
                new_text: expanded,
            })),
            ..Default::default()
        }
    }

    fn get_next_input_suggestion(&self, abbr: &str, range: Range) -> Option<Vec<CompletionItem>> {
        let abbr_end = abbr.split(['>', '+']).next_back()?;
        if abbr_end.trim().is_empty() {
            return None;
        }

        let suggested_completion = self
            .snippets
            .to_hashmap()
            .keys()
            .filter(|k| k.starts_with(abbr_end) && k.len() > abbr_end.len())
            .map(|k| {
                let expanded = format!("{}{}", &abbr[..abbr.len() - abbr_end.len()], k);
                CompletionItem {
                    label: format!("Glyf: {}", k),
                    kind: Some(CompletionItemKind::TEXT),
                    documentation: Some(self.create_documentation(k)),
                    detail: Some("Expand next input".into()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    filter_text: Some(format!(
                        "...{}",
                        &expanded[expanded.len().saturating_sub(10)..]
                    )),
                    sort_text: Some("|".to_string()),
                    preselect: Some(true),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: expanded,
                    })),
                    ..Default::default()
                }
            })
            .collect::<Vec<CompletionItem>>();

        Some(suggested_completion)
    }

    fn get_range_from_content(&self, range: Range, content: &str) -> Option<Range> {
        let pos = range.start;
        let line = content.lines().nth(pos.line as usize).unwrap_or("");
        if !HTML_TAG_REGEX.is_match(line.trim()) {
            return None;
        }
        if ONE_LINE_TAG_REGEX.is_match(line.trim()) {
            return Some(Range {
                start: Position {
                    line: pos.line,
                    character: (line.len() - line.trim_start().len()) as u32,
                },
                end: Position {
                    line: pos.line,
                    character: line.trim_end().len() as u32,
                },
            });
        }
        compute_tag_opening_closing_range(content, pos)
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
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
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

        let Some(expanded) = self.expand_abbreviation(abbr, uri).await else {
            return Ok(None);
        };

        let range = abbreviation_range(pos, abbr.len());

        let item = self.build_completion_item(abbr, insert_tabstops(&expanded), range);

        let Some(suggestions) = self.get_next_input_suggestion(abbr, range) else {
            return Ok(Some(CompletionResponse::Array(vec![item])));
        };

        let mut item = vec![item];
        item.extend(suggestions);

        Ok(Some(CompletionResponse::Array(item)))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri.to_string();
        let Some(content) = self.documents.get(&uri) else {
            return Ok(None);
        };

        let range = if params.range.start == params.range.end {
            self.get_range_from_content(params.range, &content)
        } else {
            Some(params.range)
        };

        if range.is_none() {
            return Ok(None);
        }

        let range = range.unwrap();

        let selected = extract_range(&content, range);
        if selected.trim().is_empty() {
            return Ok(None);
        }

        let Ok(abbreviation) = compress(&selected.replace("  ", "").replace("\n", " ")) else {
            return Ok(None);
        };

        let changes = HashMap::from([(
            params.text_document.uri.clone(),
            vec![TextEdit {
                range,
                new_text: abbreviation.clone(),
            }],
        )]);

        let action = CodeAction {
            title: "Compress to Glyf abbreviation".to_string(),
            kind: Some(CodeActionKind::REFACTOR_REWRITE),
            edit: Some(WorkspaceEdit {
                changes: Some(changes),
                ..Default::default()
            }),
            ..Default::default()
        };

        Ok(Some(vec![CodeActionOrCommand::CodeAction(action)]))
    }
}
