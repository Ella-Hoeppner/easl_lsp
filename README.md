Language Server and VS Code language client for [easl](https://github.com/Ella-Hoeppner/easl).

## VS Code Setup

**Prerequisites:** Rust 1.90 or newer, Node.js with npm, VS Code's `code` command on the PATH, and `vsce` (`npm install -g @vscode/vsce`).

**Build & install** (from repo root):
```
cargo build --release && cd vscode_client/easl-language-client && npm install && npm run build && vsce package && code --install-extension easl-language-client-0.1.0.vsix && cd ../../
```

## Features

- **Diagnostics** – errors are underlined as you type. Imports are followed using the unsaved contents of open files, and an error inside an imported file is shown on the `import` that brings it in, linking to where it is.
- **Hover** – over an expression, its inferred type; over the name of a function, type, variant, or top-level variable (where it's defined or where it's used), its declaration, with every overload of an overloaded function. Imported names are shown the way the file writes them (`geo/Point` for a file imported as `geo`).
- **Go to definition** – `cmd`-click (or `F12`) on a name jumps to its definition: a function, type, variant, or top-level variable in whichever file it's in, or a local variable or parameter.
- **Completion** – suggests builtins, keywords, the names used in the file, and everything reachable through its imports (`geo/Point`, `geo/Shape/Circle`, ...), fuzzy-matched as you type.
- **Formatting** – Format Document (`shift+alt+f`) reformats the whole file.
- **Syntax highlighting** – keywords, numbers, strings, and comments, with brackets colored by nesting depth.
- **Bracket matching & auto-close** – `(` inserts `()`, `[` inserts `[]`, `{` inserts `{}`.
- **Structural editing:**
  - `ctrl+w` – expand the selection to the enclosing form
  - `ctrl+a` / `ctrl+d` – move the cursor to the start / end of the current form
  - `ctrl+shift+a` / `ctrl+shift+d` – move the current form backward / forward past its neighbor
