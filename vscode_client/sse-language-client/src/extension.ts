import * as path from 'path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
  Executable
} from 'vscode-languageclient/node';

let client: LanguageClient;

export class TypeHoverProvider implements vscode.HoverProvider {
  async provideHover(
    document: vscode.TextDocument,
    position: vscode.Position,
    token: vscode.CancellationToken
  ): Promise<vscode.Hover | null> {
    const result = await vscode.commands.executeCommand(
      'getTypeInfo',
      document.uri.toString(),
      position.line,
      position.character
    ) as string | undefined;
    if (!result) return null;

    const range = new vscode.Range(position, position);

    return new vscode.Hover([
      new vscode.MarkdownString(result)
    ], range);
  }
}


function selectionPositions(editor: vscode.TextEditor) {
  const selection = editor.selection;
  return [{
    textDocument: { uri: editor.document.uri.toString() },
    position: {
      line: selection.start.line,
      character: selection.start.character
    },
  },
  {
    textDocument: { uri: editor.document.uri.toString() },
    position: {
      line: selection.end.line,
      character: selection.end.character
    },
  }];
}

async function expandSelection() {
  const editor = vscode.window.activeTextEditor;
  if (editor) {
    try {
      const result = await vscode.commands.executeCommand(
        'expandSelection',
        selectionPositions(editor)
      ) as [number, number, number, number] | undefined;
      if (result !== undefined) {
        editor.selection = new vscode.Selection(
          new vscode.Position(result[0], result[1]),
          new vscode.Position(result[2], result[3]),
        );
      } else {
        console.error('result was undefined');
      }
    } catch (error) {
      console.error('Error calling custom LSP command:', error);
    }
  }
}

async function moveCursorToStart() {
  const editor = vscode.window.activeTextEditor;
  if (editor) {
    try {
      const result = await vscode.commands.executeCommand(
        'moveCursorToStart',
        selectionPositions(editor)
      ) as [number, number] | undefined;
      if (result !== undefined) {
        editor.selection = new vscode.Selection(
          new vscode.Position(result[0], result[1]),
          new vscode.Position(result[0], result[1]),
        );
      } else {
        console.error('result was undefined');
      }
    } catch (error) {
      console.error('Error calling custom LSP command:', error);
    }
  }
}

async function moveCursorToEnd() {
  const editor = vscode.window.activeTextEditor;
  if (editor) {
    try {
      const result = await vscode.commands.executeCommand(
        'moveCursorToEnd',
        selectionPositions(editor)
      ) as [number, number] | undefined;
      if (result !== undefined) {
        editor.selection = new vscode.Selection(
          new vscode.Position(result[0], result[1]),
          new vscode.Position(result[0], result[1]),
        );
      } else {
        console.error('result was undefined');
      }
    } catch (error) {
      console.error('Error calling custom LSP command:', error);
    }
  }
}

export class DocumentSemanticTokensProvider implements vscode.DocumentSemanticTokensProvider {
  async provideDocumentSemanticTokens(document: vscode.TextDocument): Promise<vscode.SemanticTokens> {
    console.log("provideDocumentSemanticTokens");
    const builder = new vscode.SemanticTokensBuilder();
    const result = await vscode.commands.executeCommand(
      'colorDocument',
      document.uri.toString()
    ) as [[number, number, number, number, number]];
    for (let [line, pos, len, type, modifier] of result) {
      builder.push(
        line,
        pos,
        len,
        type,
        modifier
      );
    }
    return builder.build();
  }
}

export function activate(context: vscode.ExtensionContext) {
  const serverPath = path.join(__dirname, 'sse_lsp');

  const runOptions: Executable = { command: serverPath, transport: TransportKind.stdio };
  const debugOptions: Executable = { command: serverPath, transport: TransportKind.stdio, args: ['--nolazy', '--inspect=6009'] };

  const serverOptions: ServerOptions = {
    run: runOptions,
    debug: debugOptions
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'sse' }],
  };

  // Register the cursor and selection commands
  for (let [commandName, commandHandler] of
    [['extension.moveCursorToStart', moveCursorToStart],
    ['extension.moveCursorToEnd', moveCursorToEnd],
    ['extension.expandSelection', expandSelection],
    ] as const) {
    context.subscriptions.push(
      vscode.commands.registerCommand(commandName, commandHandler)
    );
  }

  // Register the formatting provider
  context.subscriptions.push(
    vscode.languages.registerDocumentFormattingEditProvider('sse', {
      async provideDocumentFormattingEdits(
        document: vscode.TextDocument,
        options: vscode.FormattingOptions
      ): Promise<vscode.TextEdit[]> {
        try {
          const result = await vscode.commands.executeCommand(
            'formatDocument',
            document.uri.toString()
          ) as string | undefined | null;

          if (result !== undefined && result !== null) {
            return [new vscode.TextEdit(
              new vscode.Range(
                new vscode.Position(0, 0),
                document.lineAt(document.lineCount - 1).range.end
              ),
              result
            )];
          }
        } catch (error) {
          console.error('Error formatting document:', error);
        }
        return [];
      }
    })
  );

  // Register the type command handler
  context.subscriptions.push(
    vscode.commands.registerCommand('type', async args => {
      let text = args.text;
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== 'sse') {
        await vscode.commands.executeCommand('default:type', { text });
        return;
      }

      let match = [["(", ")"], ["[", "]"], ["{", "}"]].find(pair => text == pair[0]);
      if (match) {
        let { start, end } = editor.selection;
        if (start.isEqual(end)) {
          await editor.edit(editBuilder => {
            editBuilder.insert(start, match[0] + match[1]);
          });
          const newPosition = start.translate(0, 1);
          editor.selection = new vscode.Selection(newPosition, newPosition);
        } else {
          await editor.edit(editBuilder => {
            editBuilder.insert(end, match[1]);
            editBuilder.insert(start, match[0]);
          });
          editor.selection = new vscode.Selection(
            start.translate(0, 1),
            end.translate(0, 0)
          );
        }
        return;
      }

      await vscode.commands.executeCommand('default:type', { text });
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('extension.handleBackspace', async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor || editor.document.languageId !== 'sse') {
        await vscode.commands.executeCommand('deleteLeft');
        return;
      }

      const position = editor.selection.active;
      const line = editor.document.lineAt(position.line);
      const nextChar = line.text[Math.max(0, position.character - 1)];

      if (position == editor.selection.anchor && (nextChar == ")" || nextChar == "(")) {
        const newPosition = position.translate(0, -1);
        editor.selection = new vscode.Selection(newPosition, newPosition);
      } else {
        await vscode.commands.executeCommand('deleteLeft');
      }
    }));

  context.subscriptions.push(
    vscode.languages.registerDocumentSemanticTokensProvider(
      { language: "sse" },
      new DocumentSemanticTokensProvider(),
      {
        tokenTypes: [
          "comment",
          "keyword",
          "number",
          "operator",
          "encloser0",
          "encloser1",
          "encloser2",
          "encloser3",
          "encloser4",
          "encloser5",
          "encloser6",
        ],
        tokenModifiers: []
      }
    )
  );

  context.subscriptions.push(
    vscode.languages.registerHoverProvider(
      'sse',
      new TypeHoverProvider()
    )
  );

  vscode.workspace.getConfiguration().update(
    'editor.semanticTokenColorCustomizations',
    {
      rules: {
        'encloser0': '#FFFFFF',
        'encloser1': '#FF6666',
        'encloser2': '#88FF88',
        'encloser3': '#8888FF',
        'encloser4': '#FFFF88',
        'encloser5': '#88FFFF',
        'encloser6': '#FF88FF',
      }
    },
    true
  );

  client = new LanguageClient(
    'sseLanguageServer',
    'SSE Language Server',
    serverOptions,
    clientOptions
  );

  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}