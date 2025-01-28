## Development

For debugging, you can test the extension in a new window (after running `npm build`) by simply pressing `f5` in vscode.

To build the extension for installation run `vsce package` (or if `vsce` isn't installed, run `npm install -g @vscode/vsce` first), then install the produced `.vsix` file. Installation can be done by right-clicking on the file in the explorer within vscode and selecting "Install Extension", or by doing `code --install-extension <extension-name>.vsix`.
