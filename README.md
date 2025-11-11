WIP Language Server for [EASL](https://github.com/Ella-Hoeppner/easl)

The following command will build rust code, build the client, and then install the client:

```
cargo build --release; cd vscode_client/sse-language-client/; npm run build; vsce package; code --install-extension sse-language-client-0.0.1.vsix; cd ../../
```

# to do
* ctrl-a when to the left of a prefix op doesn't work

* don't re-typecheck everything for every hover, instead just do typechecking whenever a change occurs and cache the type annotation info

* types with generics are displayed wrong, usually just with the base type name and no generic args

* hovering over the args list gives weird results, it just 

* move selected form forward/backward in its parent with ctrl-shift-a/d

* when cursor is adjacent to an encloser, put a little highlight box around it and the corresponding opener/closer, like happens in other languages

* go-to definition on cmd-click for functions and types

* should make the names of types in type hover info clickable to go to the definition

* Hovering over a name bound to a top-level definition (whether `defn`, `def`, `override`, or `var`) should give more info, like display the whole type definition/function signature or something like that.
