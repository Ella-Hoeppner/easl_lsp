WIP Language Server and VS Code language client for [easl](https://github.com/Ella-Hoeppner/easl), a Lisp-like shader language that compiles to WGSL.

## VS Code Setup

**Build & install** (from repo root):
```
cargo build --release && cd vscode_client/easl-language-client && npm run build && vsce package && code --install-extension easl-language-client-0.1.0.vsix && cd ../../
```

## Features

- **Diagnostics** – parse errors and type errors underlined as you type
- **Hover** – shows the inferred type of any expression (move cursor over it)
- **Formatting** – Format Document (`shift+alt+f`) reformats the whole file
- **Rainbow bracket coloring** – depth-colored `()`, `[]`, `{}` via semantic tokens
- **Bracket matching & auto-close** – `(` inserts `()`, `[` inserts `[]`, `{` inserts `{}`
- **Structural editing:**
  - `ctrl+w` – expand selection to enclosing form
  - `ctrl+a` – move cursor to start of current form
  - `ctrl+d` – move cursor to end of current form

## To do

- Move selected form forward/backward in its parent (`ctrl+shift+a`/`d`)
- Go-to-definition on cmd-click for functions and types
- Hovering over a top-level definition name should show the full type/function signature
- Types with generics are sometimes displayed incorrectly (base type only, no generic args)
