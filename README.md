WIP Language Server for [EASL](https://github.com/Ella-Hoeppner/easl)

## Neo (current)

The `neo/` directory contains a reimplemented, cleaner LSP that uses standard LSP
features (hover, semantic tokens, formatting) rather than custom commands, and
properly registers bracket pairs/auto-close via `language-configuration.json`.

**One-time setup** (from `neo/vscode_client/easl-language-client/`):
```
npm install
```

**Build & install** (from repo root):
```
cargo build --release --bin easl_lsp && cd neo/vscode_client/easl-language-client && npm run build && vsce package && code --install-extension easl-language-client-0.1.0.vsix && cd ../../../
```

> Note: if you have the old `sse-language-client` extension installed, disable or
> uninstall it to avoid conflicts (both claim `.easl` files).

### Features
- **Diagnostics** – parse errors and type errors underlined as you type
- **Hover** – shows the inferred type of any expression (move cursor over it)
- **Formatting** – Format Document (`shift+alt+f`) reformats the whole file
- **Rainbow bracket coloring** – depth-colored `()`, `[]`, `{}` via semantic tokens
- **Bracket matching** – VS Code highlights matching brackets natively
- **Auto-close brackets** – `(` inserts `()`, `[` inserts `[]`, `{` inserts `{}`
- **Structural editing:**
  - `ctrl+w` – expand selection to enclosing form
  - `ctrl+a` – move cursor to start of current form
  - `ctrl+d` – move cursor to end of current form

---

## Old (legacy, `src/` + `vscode_client/`)

```
cargo build --release; cd vscode_client/sse-language-client/; npm run build; vsce package; code --install-extension sse-language-client-0.0.1.vsix; cd ../../
```

# to do
* move selected form forward/backward in its parent with ctrl-shift-a/d

* go-to definition on cmd-click for functions and types

* Hovering over a name bound to a top-level definition (whether `defn`, `def`,
  `override`, or `var`) should give more info, like display the whole type
  definition/function signature

* types with generics are displayed wrong sometimes (just the base type name,
  no generic args)
