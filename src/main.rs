use dashmap::DashMap;
use glyf_lsp::{GlyfLsp, SnippetStore};
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    // The LSP server communicates over stdio — Zed spawns it and
    // reads/writes via its stdin/stdout
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| GlyfLsp {
        client,
        documents: DashMap::new(),
        snippets: SnippetStore::new(),
    });

    Server::new(stdin, stdout, socket).serve(service).await;
}
