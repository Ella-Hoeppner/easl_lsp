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

      const position = editor.selection.active;

      if (text == "(") {
        await editor.edit(editBuilder => {
          editBuilder.insert(position, "()");
        });
        const newPosition = position.translate(0, 1);
        editor.selection = new vscode.Selection(newPosition, newPosition);
        return;
      }

      await vscode.commands.executeCommand('default:type', { text });
    })
  );

  context.subscriptions.push(
    vscode.commands.registerCommand('extension.handleBackspace', async () => {
      console.log("backspace");
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