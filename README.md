# glyf-lsp

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

A Language Server Protocol implementation for [glyph](https://github.com/LeoDurillon/glyf-core)
abbreviation expansion. Write compact abbreviations and get full HTML or JSX structures as
editor completions.

```
ul>li.item*3  →  <ul>
                   <li class="item"></li>
                   <li class="item"></li>
                   <li class="item"></li>
                 </ul>
```

Powered by [`glyf-core`](https://crates.io/crates/glyf-core).

---

## Installation

### From source

Requires the [Rust toolchain](https://rustup.rs).

```sh
git clone https://github.com/LeoDurillon/glyf-lsp
cd glyf-lsp
cargo build --release
# Binary is at ./target/release/glyf-lsp
```

### From a GitHub release

Download the pre-built binary for your platform from the
[Releases](https://github.com/LeoDurillon/glyf-lsp/releases) page and place it
somewhere on your `$PATH`.

---

## Editor configuration

### Zed

The Zed extension is not yet published to the marketplace.
Install it as a dev extension by following the instructions in the
[zed-glyf repository](https://github.com/LeoDurillon/zed-glyf).

---

## Initialization options

The server reads configuration from `initializationOptions` at startup.

### `snippets`

A map of custom aliases that extend or override the
[built-in snippet table](https://crates.io/crates/glyf-core).

```json
{
  "initializationOptions": {
    "snippets": {
      "<alias>": "<glyf abbreviation>"
    }
  }
}
```

| Key | Value | Result |
|-----|-------|--------|
| `"mc"` | `"MyComponent"` | `mc` → `<MyComponent></MyComponent>` |
| `"card"` | `"div.card>p.card-body"` | `card` → full card layout |
| `"btn"` | `"MyButton"` | overrides built-in `btn` → `<MyButton></MyButton>` |

Custom snippets follow the same expansion rules as built-ins, including
multi-element expansions — any abbreviation containing `>` or `+` expands
into a full element tree.

---

## How it works

1. The server syncs every open document in full on `textDocument/didOpen` and
   `textDocument/didChange`.
2. On `textDocument/completion`, it reads the text on the cursor line up to the
   cursor position, strips any `return ` prefix (for JSX return statements), and
   attempts to expand the result as a glyf abbreviation.
3. If the abbreviation is valid, a single `CompletionItem` is returned with
   the expanded HTML/JSX as the replacement text and a human-readable preview
   in the documentation field.
4. If the abbreviation is malformed (unmatched brackets, no tag name), the
   request returns no completions and the error is logged to the client.

**Completion is triggered automatically** on `.`, `:`, `>`, `+`, `*`, and `(`.

---

## License

MIT — see [LICENSE](LICENSE).
