WIP Language Server for [EASL](https://github.com/Ella-Hoeppner/easl)

dev command to built rust code, build client, and install client:

```
cargo build --release; cd vscode_client/sse-language-client/; npm run build; vsce package; code --in
stall-extension sse-language-client-0.0.1.vsix; cd ../../
```

# to do
* rn you can't delete the openers or closers of enclosers by pressing backspace, which is great 90% of the time, but kinda sucks when the document is in an unparsable state and the user needs to manually create/delete unmatched enclosers. Therefore there needs to be a way for the client to check whether the document is currently parsing correctly, and allow deletion of enclosers when parsing is failing

* Typing a closing encloser while the document is parsing validly and the encloser being typed immediately follows the cursor should just move the cursor to the right, rather than typing the closer
  * i.e. if the cursor is at like `(|)` (where `|` is the curosr), and the user types `)`, that should change to `()|` rather than `()|)`
  * this needs to be disabled during parse failures, where the user might indeed need to insert a solo closer

* improve type descriptions on mouse hover for `let` bindings, right now you don't get their types and instead just get the type of the whole `let` block

* when surrounding a highlighted form in parentheses, the ending of the newly highlighted section is in the wrong place

* rename internally to easl_lsp

* ctrl-a when to the left of a prefix op doesn't work

* move selected form forward/backward in its parent with ctrl-shift-a/d

* when cursor is adjacent to an encloser, put a little highlight box around it and the corresponding opener/closer, like happens in other languages
