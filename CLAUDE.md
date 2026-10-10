# Easl LSP

Language server and VS Code extension for [Easl](https://github.com/Ella-Hoeppner/easl), a Lisp-like shader language that compiles to WGSL.

## Repo structure

```
src/
  main.rs       # binary entry point – starts the tower-lsp server
  server.rs     # open-document state, analysis scheduling, LSP handlers
  analysis.rs   # compiles a document (with its imports) into diagnostics,
                # type hints, and the names it can refer to
  source.rs     # SourceText: byte offsets <-> LSP positions (UTF-16)
  syntax.rs     # semantic tokens and identifier collection (parse-only)
  completion.rs # completion candidates and their kinds
  structural.rs # structural edits (moving a form past its neighbor)
vscode_client/
  easl-language-client/
    src/extension.ts          # VS Code extension entry point
    language-configuration.json  # bracket pairs, auto-close, comments
    syntaxes/easl.tmLanguage.json  # TextMate grammar: strings and comments only,
                                   # so VS Code's bracket matching skips them
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

## Server architecture

### Analysis model (`analysis.rs`)

Every open document is analyzed as the **main file of its own program**: it's
loaded with `load_and_parse_easl_multidocument_with_lookup_function`, which
follows its imports. The lookup prefers the text of open documents (keyed by
canonical path) over the files on disk, so unsaved edits are seen by every
importer. A file module never sees the files that import it, so a document's
own diagnostics don't depend on who imports it — this is what makes
per-document analysis exactly right.

- Errors located in the analyzed document become diagnostics there, with the
  error's secondary positions as `relatedInformation` (which can point into
  other files)
- Errors located in an imported file are reported on the analyzed document's
  `import` form that (transitively) brings that file in, prefixed
  `in <file>:`, with the real location as related information. When the
  imported file is itself open, it also shows its own errors in place
- Errors with no position go at the start of the document; parse failures in
  any file are placed the same way
- Validation (`validate_raw_program`) runs only when module resolution
  (`Program::from_easl_documents`) succeeded, like the CLI
- Type hints (for hover) come from `gather_type_annotations` after
  validation, restricted to positions in the analyzed document
- From `Program::main_file_index` (filled by module resolution, so
  available even when validation fails):
  - `names`: every bare name and path (`geo/Point`, `geo/Shape/Circle`) the
    document can refer to, with its `NameKind`, for completion
  - `references`: each name in the document referring to top-level
    definitions, with the definitions' name locations (every overload's) —
    for go-to-definition and hover
  - `definition_names`: the names of the document's own definitions
  - `written_names`: how the document writes each reachable definition
    whose internal name differs (`geometry/Point` → `geo/Point`). Type
    hints, declarations, and diagnostic messages are passed through
    `written_names` (token-wise replacement; differing internal names always
    contain `/`, so ordinary words are never touched)
- Local variables come from `easl::compiler::info::local_references` (also
  computed before validation, which renames bindings): each name in a body
  referring to a parameter, `let` binding, lambda parameter, `match`
  binding, or `for` variable, with the binding's position. They join
  `references` with no declaration, so hovering a local still shows its
  type
- Declarations shown on hover come from `easl::compiler::info::
  definition_signatures`, computed **before** validation (which rewrites
  definitions), keyed by each definition's name position; a definition's
  declaration is the one whose key is the smallest span containing its name
  (a variant's key is the whole variant)
- A compiler panic is caught and shown as an "internal compiler error"
  diagnostic
- A document that isn't a file on disk (untitled) is compiled alone

### Scheduling (`server.rs`)

`OpenDocument` holds the current text (`SourceText`), the client's version,
a text `revision`, and the results of the last analysis (type hints, the
canonical paths it depends on, completion names).

- Every text change replaces the text immediately (formatting and structural
  editing always see current text), bumps `revision`, and schedules an
  analysis of the document **and of every open document whose dependencies
  include it**
- An analysis waits `ANALYSIS_DELAY` (150ms) and runs only if no newer
  analysis of the document was scheduled meanwhile (`latest_analysis`
  ticket); its results are dropped if a newer one was scheduled while it ran.
  Diagnostics are published under the state lock, so they can't arrive out
  of order
- Hover answers only when the type hints were computed from the current
  text revision
- Closing a document clears its diagnostics and reschedules its dependents
  (they now read the file from disk)
- `.easl` file changes on disk are watched (`workspace/didChangeWatchedFiles`,
  registered dynamically in `initialized`): a changed file that isn't open
  reschedules its dependents; a created file reschedules everything (it may
  satisfy a missing import)

### Positions (`source.rs`)

Compiler spans are byte offsets; LSP positions are zero-based lines with
**UTF-16** columns. All conversions go through `SourceText` (`position`,
`offset`, `range`, `line_pieces` for splitting multi-line spans into
single-line semantic tokens). Don't use fsexp's row/column helpers — they
count bytes.

### LSP capabilities advertised

| Capability | Method |
|---|---|
| Text sync (FULL) | `did_open`, `did_change`, `did_close` |
| Hover | `textDocument/hover` — on a name referring to definitions, their declarations; on a definition's own name, its declaration; otherwise the innermost typed expression's type, preferring fully known types |
| Go to definition | `textDocument/definition` — the definitions a name refers to (several for overloads), in any file, or a local's binding |
| Semantic tokens | `textDocument/semanticTokens/full` |
| Formatting | `textDocument/formatting` (skipped when the document doesn't parse) |
| Completion | `textDocument/completion` (`completion.rs`) — every candidate on every request, for the editor to fuzzy-filter: `names` from the last analysis (with their kinds), words written in the document (locals, mostly), and builtins (functions, types, `Option`'s variants, macros, keywords; operator-only names and `$`-internal names left out). `sort_text` ranks them in that order. Triggered by `-` and `/` |
| Execute command | `workspace/executeCommand` (structural editing) |

`executeCommandProvider` is intentionally **not** advertised in capabilities.
If it were, `vscode-languageclient` would auto-register the command names as VS
Code commands, conflicting with the extension's own `registerCommand` calls.
The server still handles `workspace/executeCommand` requests fine without advertising it.

### Semantic token legend (`syntax.rs`)

| Index | Type | Used for |
|---|---|---|
| 0 | `comment` | `;` comments, `#_` commented-out forms |
| 1 | `keyword` | `defn`, `let`, `mod`, `use`, `import`, … (`KEYWORDS`) |
| 2 | `number` | numeric literals (`0`, `-1.5`, `2u`, `3i`, `4.f`) |
| 3 | `operator` | prefix operators (`:`, `@`, `~`) |
| 4–10 | `encloserD0`–`encloserD6` | bracket pairs colored by nesting depth mod 7 |
| 11 | `string` | string literals |

The extension refers to the encloser types by name. Colors for
`encloserD0`–`encloserD6` are written to the user's global
`editor.semanticTokenColorCustomizations` setting by the extension on
activation (only if they're missing or changed).

### Structural editing commands

Handled in `execute_command`. Each command takes one argument: a JSON array
`[startPositionParams, endPositionParams]` (two `TextDocumentPositionParams`).

- `easl.expandSelection` → returns `[startLine, startCol, endLine, endCol]`
- `easl.moveCursorToStart` → returns `[line, col]`
- `easl.moveCursorToEnd` → returns `[line, col]`
- `easl.moveFormForward` / `easl.moveFormBackward` → returns
  `{range: [sl, sc, el, ec], newText, selection: [sl, sc, el, ec]}` (the
  selection is in the edited text), or null. Swaps the innermost form
  containing the selection whose parent is a bracketed form (or the
  document) with its next/previous non-comment sibling — so a form under an
  operator (`@cpu (defn …)`, `x: f32`) moves with its operator, and a cursor
  on a word moves that word within its list (`structural::move_form`)

The extension calls these via `client.sendRequest('workspace/executeCommand', {...})`.

### Tests

`cargo test` runs unit tests for `SourceText` and `syntax`, and analysis
tests that write small multi-file projects to a temp directory (imports
resolving, imported-file errors landing on the import, unsaved buffers
overriding files, missing/unparsable imports, type hints, references and
declarations), completion tests, and `structural` tests.

## VS Code extension (`vscode_client/easl-language-client/src/extension.ts`)

- Language ID: `easl`, file extensions: `.easl`
- Hover, diagnostics, semantic tokens, and formatting are all handled automatically
  by `LanguageClient` — no special code needed in the extension for these.
- Structural editing commands are registered manually with `registerCommand` and
  call the server via `client.sendRequest('workspace/executeCommand', {...})`.
  The move-form commands apply the returned edit with `editor.edit`, then
  set the selection.
- Rainbow bracket colors are applied once to `editor.semanticTokenColorCustomizations`
  in the user's global settings.
