# Easl LSP

Language server and VS Code extension for [Easl](https://github.com/Ella-Hoeppner/easl), a Lisp-like shader language that compiles to WGSL.

## Repo structure

```
src/
  main.rs          # binary entry point – just starts the tower-lsp server
  server.rs        # all server logic
vscode_client/
  easl-language-client/
    src/extension.ts          # VS Code extension entry point
    language-configuration.json  # bracket pairs, auto-close, comments
    package.json
    webpack.config.js
    build.js                  # copies the Rust binary into out/
```

## Build & install

**One-time** (from `vscode_client/easl-language-client/`):
```
npm install
```

**Full build + install** (from repo root):
```
cargo build --release && cd vscode_client/easl-language-client && npm run build && vsce package && code --install-extension easl-language-client-0.1.0.vsix && cd ../../
```

`build.js` copies `target/release/easl_lsp` into `out/` alongside the bundled extension JS. The VSIX bundles both.

## Server architecture (`src/server.rs`)

### State

```rust
struct DocumentState {
    text: String,                                    // always current
    type_annotations: Vec<(SourceTrace, bool, String)>, // (trace, is_fully_known, description)
}
```

`text` is written **immediately** when `did_change` fires (before the compilation
is spawned). `type_annotations` is written only after compilation finishes.
This ensures formatting and structural editing always see the latest text, while
hover tolerates briefly-stale annotations.

### `update_document` flow

1. Acquire write lock → update `state.text` → release lock
2. `spawn_blocking` → run `compile_text` (parse + full compilation pipeline)
3. Acquire write lock → update `state.type_annotations` → release lock
4. `publish_diagnostics`

### LSP capabilities advertised

| Capability | Method |
|---|---|
| Text sync (FULL) | `did_open`, `did_change`, `did_close` |
| Hover | `textDocument/hover` |
| Semantic tokens | `textDocument/semanticTokens/full` |
| Formatting | `textDocument/formatting` |
| Execute command | `workspace/executeCommand` |

`executeCommandProvider` is intentionally **not** advertised in capabilities.
If it were, `vscode-languageclient` would auto-register the command names as VS
Code commands, conflicting with the extension's own `registerCommand` calls.
The server still handles `workspace/executeCommand` requests fine without advertising it.

### Semantic token legend (indices must stay in sync with extension)

| Index | Type | Used for |
|---|---|---|
| 0 | `comment` | `;` comments, `#_` commented-out forms |
| 1 | `keyword` | `defn`, `def`, `let`, `var`, `if`, etc. |
| 2 | `number` | numeric literals (`0`, `1.5`, `2u`, `3i`, `4f`) |
| 3 | `operator` | prefix operators (`:`, `@`, etc.) |
| 4–10 | `encloserD0`–`encloserD6` | bracket pairs colored by nesting depth mod 7 |

Colors for `encloserD0`–`encloserD6` are written to the user's global
`editor.semanticTokenColorCustomizations` setting by the extension on activation
(only if they're missing or changed).

### Structural editing commands

Handled in `execute_command`. Each command takes one argument: a JSON array
`[startPositionParams, endPositionParams]` (two `TextDocumentPositionParams`).

- `easl.expandSelection` → returns `[startLine, startCol, endLine, endCol]`
- `easl.moveCursorToStart` → returns `[line, col]`
- `easl.moveCursorToEnd` → returns `[line, col]`

The extension calls these via `client.sendRequest('workspace/executeCommand', {...})`.

## Easl compiler API used

All from the `easl` crate (`git = "https://github.com/Ella-Hoeppner/easl.git"`):

```rust
// Parsing
parse_easl(text)                  // EaslDocument — includes comment nodes, used for
                                  // coloring, formatting, structural editing
parse_easl_without_comments(text) // EaslDocument — for compilation

// Compilation
Program::from_easl_document(&doc, built_in_macros()) -> (Program, ErrorLog)
program.validate_raw_program() -> ErrorLog   // runs the full type-checking pipeline
program.gather_type_annotations() -> Vec<(SourceTrace, TypeState)>

// Formatting
format_document(document: EaslDocument) -> String   // takes ownership

// Position conversion (on EaslDocument / Document)
document.row_and_col_to_index(row, col) -> Result<usize, _>   // byte index, 0-based
document.index_to_row_and_col(index) -> Result<(row, col), _> // 0-based

// Structural editing (on Document)
document.expand_selection(&Range<usize>) -> Option<Range<usize>>
document.move_cursor_to_start(&Range<usize>) -> usize
document.move_cursor_to_end(&Range<usize>) -> usize

// Syntax coloring
document.gather_annotations(init_state, leaf_fn, encloser_fn, operator_fn)
    -> Vec<(Range<usize>, A)>   // None annotations are filtered out;
                                 // for enclosers, pushes opener span + closer span

// Parse errors
document.parsing_failures: Vec<ParseError>
// ParseError { pos: Range<usize>, kind: ParseErrorKind }
```

### TypeState

```rust
// #[derive(Debug, Clone)]
enum TypeState {
    Known(Type),
    Unknown,
    OneOf(Vec<Type>),
    UnificationVariable(Arc<RwLock<TypeState>>),
}
impl TypeState {
    fn check_is_fully_known(&self) -> bool { ... }
}
```

For display: `TypeDescription::from(t).to_string()` where `t: Type` (from `Known`
variant); `TypeStateDescription::from(ts).to_string()` for non-`Known` states.

### SourceTrace

```rust
struct SourceTrace {
    primary_position: Option<DocumentPosition>,
    secondary_positions: Vec<DocumentPosition>,
}
struct DocumentPosition {
    span: Range<usize>,  // byte range into the source text
    path: Vec<usize>,
}
```

For hover: find the annotation whose `primary_position.span` contains the cursor
byte index and has the smallest span length; prefer `is_fully_known` when tie-breaking.

### `gather_annotations` for coloring

The walk state is `(is_commented: bool, depth: usize)`.

- **Leaf callback**: returns `Some(token_type)` for keywords/numbers, `Some(TOK_COMMENT)` when commented, else `None`.
- **Encloser callback**: returns `Some(encloser_token_type(depth))` (or `Some(TOK_COMMENT)` when already commented). For `Encloser::LineComment` and `Encloser::BlockComment`, override the new walk state to `(true, depth)` so all children are colored as comments.
- **Operator callback**: `ExpressionComment` (`#_`) → new state `(true, depth)`, annotation `None`. Other operators → `Some(TOK_OPERATOR)` (or `Some(TOK_COMMENT)` when commented).

Semantic tokens must be sorted by `(line, col)` before delta-encoding.

## VS Code extension (`vscode_client/easl-language-client/src/extension.ts`)

- Language ID: `easl`, file extensions: `.easl`
- Hover, diagnostics, semantic tokens, and formatting are all handled automatically
  by `LanguageClient` — no special code needed in the extension for these.
- Structural editing commands are registered manually with `registerCommand` and
  call the server via `client.sendRequest('workspace/executeCommand', {...})`.
- Rainbow bracket colors are applied once to `editor.semanticTokenColorCustomizations`
  in the user's global settings.

## Known issues / future work

- Types with generics sometimes display incorrectly (base type only, no generic args)
- Move selected form forward/backward in its parent (`ctrl+shift+a`/`d`)
- Go-to-definition on cmd-click for functions and types
- Hovering over a top-level definition name should show the full type/function signature
