//! End-to-end tests that drive `LspService` directly as a `tower::Service`,
//! bypassing any real stdio transport.
//!
//! Each test creates a fresh service, performs the LSP initialization
//! handshake (`initialize` → `initialized`), then exercises the behaviour
//! under test.  Server-to-client notifications (e.g. `window/logMessage`)
//! are silently discarded because the `ClientSocket` is dropped immediately
//! after the service is created.

use dashmap::DashMap;
use glyf_lsp::{GlyfLsp, SnippetStore};
use serde_json::{Value, json};
use tower::{Service, ServiceExt};
use tower_lsp::{LspService, jsonrpc};

// ── Harness ───────────────────────────────────────────────────────────────────

/// Creates a fresh `LspService`.  The `ClientSocket` (server-to-client
/// notification channel) is dropped immediately so that `log_message` calls
/// inside the server fail silently instead of blocking.
fn make_service() -> LspService<GlyfLsp> {
    LspService::new(|client| GlyfLsp {
        client,
        documents: DashMap::new(),
        snippets: SnippetStore::new(),
    })
    .0 // discard ClientSocket
}

/// Sends one JSON-RPC message through `svc` and returns the serialized
/// response.  Notifications (no `id`) produce `Value::Null`.
async fn call(svc: &mut LspService<GlyfLsp>, req: Value) -> Value {
    let request: jsonrpc::Request = serde_json::from_value(req).unwrap();
    svc.ready()
        .await
        .unwrap()
        .call(request)
        .await
        .unwrap()
        .map(|r| serde_json::to_value(r).unwrap())
        .unwrap_or(Value::Null)
}

/// Performs the mandatory LSP handshake: `initialize` request followed by
/// the `initialized` notification.  Must be called before any other request.
async fn setup(svc: &mut LspService<GlyfLsp>) {
    call(
        svc,
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        }),
    )
    .await;
    call(
        svc,
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
}

/// Like `setup`, but passes custom snippet aliases through `initializationOptions`.
async fn setup_with_snippets(svc: &mut LspService<GlyfLsp>, snippets: Value) {
    call(
        svc,
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "initialize",
            "params": {
                "capabilities": {},
                "initializationOptions": { "snippets": snippets }
            }
        }),
    )
    .await;
    call(
        svc,
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
}

async fn open(svc: &mut LspService<GlyfLsp>, uri: &str, text: &str) {
    call(
        svc,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": uri, "languageId": "html", "version": 1, "text": text }
            }
        }),
    )
    .await;
}

async fn change(svc: &mut LspService<GlyfLsp>, uri: &str, text: &str) {
    call(
        svc,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": text }]
            }
        }),
    )
    .await;
}

/// Requests completion and returns the `result` value (array or null).
async fn complete(
    svc: &mut LspService<GlyfLsp>,
    id: u32,
    uri: &str,
    line: u32,
    character: u32,
) -> Value {
    let resp = call(
        svc,
        json!({
            "jsonrpc": "2.0", "id": id,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }),
    )
    .await;
    resp["result"].clone()
}

// ── initialize ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn initialize_advertises_completion_and_sync_capabilities() {
    let mut svc = make_service();
    let resp = call(
        &mut svc,
        json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "initialize",
            "params": { "capabilities": {} }
        }),
    )
    .await;

    let caps = &resp["result"]["capabilities"];
    assert!(
        caps["completionProvider"].is_object(),
        "completionProvider missing"
    );
    assert!(
        !caps["textDocumentSync"].is_null(),
        "textDocumentSync missing"
    );
}

// ── completion ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn completion_returns_null_for_unopened_document() {
    let mut svc = make_service();
    setup(&mut svc).await;

    let result = complete(&mut svc, 2, "file:///unknown.html", 0, 3).await;
    assert!(result.is_null());
}

#[tokio::test]
async fn completion_expands_simple_element() {
    let mut svc = make_service();
    setup(&mut svc).await;
    open(&mut svc, "file:///test.html", "div").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 3).await;

    assert!(result.is_array());
    let item = &result[0];
    assert_eq!(item["label"], json!("Glyf: div"));
    assert_eq!(item["textEdit"]["newText"], json!("<div>${1}</div>"));
    // range covers exactly "div" (chars 0–3)
    assert_eq!(item["textEdit"]["range"]["start"]["character"], json!(0u32));
    assert_eq!(item["textEdit"]["range"]["end"]["character"], json!(3u32));
}

#[tokio::test]
async fn completion_expands_child_abbreviation() {
    let mut svc = make_service();
    setup(&mut svc).await;
    open(&mut svc, "file:///test.html", "ul>li").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 5).await;
    assert_eq!(
        result[0]["textEdit"]["newText"],
        json!("<ul>\n\t<li>${1}</li>\n</ul>")
    );
}

#[tokio::test]
async fn completion_expands_self_closing_builtin_snippet() {
    let mut svc = make_service();
    setup(&mut svc).await;
    open(&mut svc, "file:///test.html", "br").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 2).await;
    assert_eq!(result[0]["textEdit"]["newText"], json!("<br />"));
}

#[tokio::test]
async fn completion_strips_return_prefix() {
    let mut svc = make_service();
    setup(&mut svc).await;
    // JSX return statement — cursor at end of "return div"
    open(&mut svc, "file:///test.tsx", "return div").await;

    let result = complete(&mut svc, 2, "file:///test.tsx", 0, 10).await;
    assert_eq!(result[0]["textEdit"]["newText"], json!("<div>${1}</div>"));
}

#[tokio::test]
async fn completion_returns_null_for_malformed_abbreviation() {
    let mut svc = make_service();
    setup(&mut svc).await;
    open(&mut svc, "file:///test.html", "div(unclosed").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 12).await;
    assert!(result.is_null());
}

// ── document sync ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn did_change_updates_document_for_next_completion() {
    let mut svc = make_service();
    setup(&mut svc).await;
    open(&mut svc, "file:///test.html", "div").await;
    change(&mut svc, "file:///test.html", "span").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 4).await;
    assert_eq!(result[0]["textEdit"]["newText"], json!("<span>${1}</span>"));
}

// ── custom snippets ───────────────────────────────────────────────────────────

#[tokio::test]
async fn custom_snippet_is_expanded_in_completion() {
    let mut svc = make_service();
    setup_with_snippets(&mut svc, json!({ "mc": "MyComponent" })).await;
    open(&mut svc, "file:///test.tsx", "mc").await;

    let result = complete(&mut svc, 2, "file:///test.tsx", 0, 2).await;
    assert_eq!(
        result[0]["textEdit"]["newText"],
        json!("<MyComponent>${1}</MyComponent>")
    );
}

#[tokio::test]
async fn custom_snippet_overrides_builtin_in_completion() {
    let mut svc = make_service();
    // built-in "btn" → "button"; custom entry shadows it
    setup_with_snippets(&mut svc, json!({ "btn": "MyButton" })).await;
    open(&mut svc, "file:///test.tsx", "btn").await;

    let result = complete(&mut svc, 2, "file:///test.tsx", 0, 3).await;
    assert_eq!(
        result[0]["textEdit"]["newText"],
        json!("<MyButton>${1}</MyButton>")
    );
}

#[tokio::test]
async fn custom_multi_element_snippet_renders_full_tree() {
    let mut svc = make_service();
    setup_with_snippets(&mut svc, json!({ "card": "div.card>p.card-body" })).await;
    open(&mut svc, "file:///test.tsx", "card").await;

    let result = complete(&mut svc, 2, "file:///test.tsx", 0, 4).await;
    assert_eq!(
        result[0]["textEdit"]["newText"],
        json!("<div class=\"card\">\n\t<p class=\"card-body\">${1}</p>\n</div>")
    );
}

// ── documentation ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn documentation_replaces_tab_stop_placeholders_with_pipe() {
    let mut svc = make_service();
    setup(&mut svc).await;
    // built-in "a:blank" expands to an anchor whose href value is "${0}"
    open(&mut svc, "file:///test.html", "a:blank").await;

    let result = complete(&mut svc, 2, "file:///test.html", 0, 7).await;
    let doc = result[0]["documentation"].as_str().unwrap_or("");
    assert!(
        doc.contains('|'),
        "documentation should replace ${{N}} with |"
    );
    assert!(
        !doc.contains("${0}"),
        "documentation should not expose raw tab stops"
    );
}
