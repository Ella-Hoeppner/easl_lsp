import * as path from 'path';
import * as vscode from 'vscode';
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient;
let outputChannel: vscode.OutputChannel;

// --- Rainbow encloser colors ---
// Must match the token type indices in the Rust server (encloserD0..D6 = indices 4..10).
const ENCLOSER_COLORS: Record<string, string> = {
  encloserD0: '#FFFFFF',
  encloserD1: '#FF6666',
  encloserD2: '#88FF88',
  encloserD3: '#8888FF',
  encloserD4: '#FFFF88',
  encloserD5: '#88FFFF',
  encloserD6: '#FF88FF',
};

// --- Helpers ---

function selectionPositions(editor: vscode.TextEditor) {
  const { start, end } = editor.selection;
  return [
    {
      textDocument: { uri: editor.document.uri.toString() },
      position: { line: start.line, character: start.character },
    },
    {
      textDocument: { uri: editor.document.uri.toString() },
      position: { line: end.line, character: end.character },
    },
  ];
}

async function sendSelectionCommand(
  commandName: string,
  editor: vscode.TextEditor
): Promise<unknown> {
  return client.sendRequest('workspace/executeCommand', {
    command: commandName,
    arguments: [selectionPositions(editor)],
  });
}

// --- Structural editing commands ---

async function expandSelection(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  try {
    const result = (await sendSelectionCommand(
      'easl.expandSelection',
      editor
    )) as [number, number, number, number] | null;
    if (result) {
      editor.selection = new vscode.Selection(
        new vscode.Position(result[0], result[1]),
        new vscode.Position(result[2], result[3])
      );
    }
  } catch (error) {
    outputChannel.appendLine(`expandSelection error: ${error}`);
  }
}

async function moveCursorToStart(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  try {
    const result = (await sendSelectionCommand(
      'easl.moveCursorToStart',
      editor
    )) as [number, number] | null;
    if (result) {
      const pos = new vscode.Position(result[0], result[1]);
      editor.selection = new vscode.Selection(pos, pos);
    }
  } catch (error) {
    outputChannel.appendLine(`moveCursorToStart error: ${error}`);
  }
}

async function moveCursorToEnd(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  try {
    const result = (await sendSelectionCommand(
      'easl.moveCursorToEnd',
      editor
    )) as [number, number] | null;
    if (result) {
      const pos = new vscode.Position(result[0], result[1]);
      editor.selection = new vscode.Selection(pos, pos);
    }
  } catch (error) {
    outputChannel.appendLine(`moveCursorToEnd error: ${error}`);
  }
}

// --- Activation ---

export function activate(context: vscode.ExtensionContext): void {
  outputChannel = vscode.window.createOutputChannel('Easl Language Client');

  // Server binary is bundled in the same directory as the extension JS.
  const binaryName =
    process.platform === 'win32' ? 'easl_lsp.exe' : 'easl_lsp';
  const serverPath = path.join(__dirname, binaryName);

  const serverOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: 'file', language: 'easl' }],
    outputChannel,
  };

  // Register structural editing commands.
  context.subscriptions.push(
    vscode.commands.registerCommand('easl.expandSelection', expandSelection),
    vscode.commands.registerCommand(
      'easl.moveCursorToStart',
      moveCursorToStart
    ),
    vscode.commands.registerCommand('easl.moveCursorToEnd', moveCursorToEnd)
  );

  // Apply rainbow encloser colors to the user's global settings so they
  // persist across VS Code restarts. The token types (encloserD0..D6) are
  // declared in package.json's contributes.semanticTokenTypes.
  const cfg = vscode.workspace.getConfiguration();
  const existing =
    cfg.get<Record<string, unknown>>(
      'editor.semanticTokenColorCustomizations'
    ) ?? {};
  const existingRules =
    (existing['rules'] as Record<string, string> | undefined) ?? {};

  // Only update if any of our colors are missing/changed.
  const needsUpdate = Object.entries(ENCLOSER_COLORS).some(
    ([id, color]) => existingRules[id] !== color
  );
  if (needsUpdate) {
    cfg.update(
      'editor.semanticTokenColorCustomizations',
      { ...existing, rules: { ...existingRules, ...ENCLOSER_COLORS } },
      vscode.ConfigurationTarget.Global
    );
  }

  client = new LanguageClient(
    'easlLanguageServer',
    'Easl Language Server',
    serverOptions,
    clientOptions
  );

  client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) return undefined;
  return client.stop();
}
