WIP Language Server for [EASL](https://github.com/Ella-Hoeppner/easl)

dev command to built rust code, build client, and install client:

```
cargo build --release; cd vscode_client/sse-language-client/; npm run build; vsce package; code --install-extension sse-language-client-0.0.1.vsix; cd ../../
```

# to do
* display at least some kind of error when parsing fails
  * rn SSE doesn't give much info when parsing fails, but it would be better than nothing to just like highlight the whole document with an error to be like "u messed up, might wanna hit ctrl-z until you're back to a valid state" whenever parsing fails
    * but a better solution would be to change SSE so that when parsing fails it gives better information. Like, at the very least it should say "parsing failed *at this particular character*", or maybe even better "parsing failed at this particular character, and here's the partial AST that I'd built up until this point so you can still do some highlighting in the valid part of the ile"

* Typing a closing encloser while the document is parsing validly and the encloser being typed immediately follows the cursor should just move the cursor to the right, rather than typing the closer
  * i.e. if the cursor is at like `(|)` (where `|` is the curosr), and the user types `)`, that should change to `()|` rather than `()|)`
  * this needs to be disabled during parse failures, where the user might indeed need to insert a solo closer

* improve type descriptions on mouse hover for `let` bindings, right now you don't get their types and instead just get the type of the whole `let` block

* rename internally to easl_lsp

* ctrl-a when to the left of a prefix op doesn't work

* move selected form forward/backward in its parent with ctrl-shift-a/d

* when cursor is adjacent to an encloser, put a little highlight box around it and the corresponding opener/closer, like happens in other languages
