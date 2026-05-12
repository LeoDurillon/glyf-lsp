use std::{collections::HashMap, sync::OnceLock};

use dashmap::DashMap;
use tower_lsp::lsp_types::InitializeParams;

use crate::consts::BASE_SNIPPET;

static SNIPPETS: OnceLock<HashMap<String, String>> = OnceLock::new();

pub struct SnippetStore {
    entries: DashMap<String, String>,
}

impl SnippetStore {
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
        }
    }

    fn get_default_snippet(&self) -> &HashMap<String, String> {
        let base_snippet = SNIPPETS.get_or_init(|| {
            BASE_SNIPPET
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        });

        return base_snippet;
    }

    /// Reads `initialization_options.snippets` and stores every entry.
    pub fn load_from_params(&self, params: &InitializeParams) {
        let defaults = self.get_default_snippet();
        for (k, v) in defaults {
            self.entries.insert(k.clone(), v.clone());
        }
        let Some(opts) = params.initialization_options.as_ref() else {
            return;
        };
        let Some(map) = opts.get("snippets").and_then(|s| s.as_object()) else {
            return;
        };
        for (k, v) in map {
            if let Some(expansion) = v.as_str() {
                self.entries.insert(k.clone(), expansion.to_string());
            }
        }
    }

    /// Returns a plain `HashMap` snapshot suitable for passing to the parser.
    /// DashMap doesn't implement Deref<Target=HashMap>, so a conversion is needed.
    pub fn to_hashmap(&self) -> HashMap<String, String> {
        self.entries
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect()
    }
}

impl Default for SnippetStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Build `InitializeParams` with the given value as `initialization_options`.
    fn params(opts: serde_json::Value) -> InitializeParams {
        InitializeParams {
            initialization_options: Some(opts),
            ..Default::default()
        }
    }

    // -------------------------------------------------------------------------
    // SnippetStore::new
    // -------------------------------------------------------------------------

    #[test]
    fn new_store_is_empty() {
        assert!(SnippetStore::new().to_hashmap().is_empty());
    }

    // -------------------------------------------------------------------------
    // SnippetStore::load_from_params
    // -------------------------------------------------------------------------

    #[test]
    fn no_initialization_options_is_noop() {
        let store = SnippetStore::new();
        store.load_from_params(&InitializeParams {
            initialization_options: None,
            ..Default::default()
        });
        assert!(store.to_hashmap().len() == BASE_SNIPPET.len());
    }

    #[test]
    fn missing_snippets_key_is_noop() {
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({ "other": "irrelevant" })));
        assert!(store.to_hashmap().len() == BASE_SNIPPET.len());
    }

    #[test]
    fn non_object_snippets_value_is_noop() {
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({ "snippets": "not-an-object" })));
        assert_eq!(store.to_hashmap().len(), BASE_SNIPPET.len());
    }

    #[test]
    fn populates_entries_from_object() {
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({
            "snippets": { "mc": "MyComponent", "btn": "MyButton" }
        })));
        let map = store.to_hashmap();
        assert_eq!(map.get("mc").map(String::as_str), Some("MyComponent"));
        assert_eq!(map.get("btn").map(String::as_str), Some("MyButton"));
    }

    #[test]
    fn non_string_values_are_skipped() {
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({
            "snippets": { "valid": "div", "num": 42, "obj": {} }
        })));
        let map = store.to_hashmap();
        assert_eq!(map.len(), BASE_SNIPPET.len() + 1);
        assert!(map.contains_key("valid"));
    }

    // -------------------------------------------------------------------------
    // SnippetStore::to_hashmap
    // -------------------------------------------------------------------------

    #[test]
    fn to_hashmap_contains_all_loaded_entries() {
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({
            "snippets": { "a": "div", "b": "span", "c": "p" }
        })));
        assert_eq!(store.to_hashmap().len(), BASE_SNIPPET.len() + 2);
    }

    #[test]
    fn to_hashmap_returns_independent_snapshot() {
        // Mutating the returned map must not affect the store
        let store = SnippetStore::new();
        store.load_from_params(&params(json!({ "snippets": { "mc": "MyComponent" } })));
        let mut map = store.to_hashmap();
        map.insert("extra".to_string(), "Extra".to_string());
        // original store still has only one entry
        assert_eq!(store.to_hashmap().len(), BASE_SNIPPET.len() + 1);
    }
}
