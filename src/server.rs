//! The language server: open-document state, scheduling analyses, and the
//! LSP request handlers.

use std::{
  collections::{HashMap, HashSet},
  path::PathBuf,
  sync::Arc,
  time::Duration,
};

use easl::{
  compiler::modules::NameKind, format::format_document, parse::parse_easl,
};
use serde_json::Value;
use tokio::sync::RwLock;
use tower_lsp::{
  Client, LanguageServer,
  jsonrpc::{Error, Result},
  lsp_types::{
    CompletionOptions, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentFormattingParams, ExecuteCommandParams,
    FileChangeType, FileSystemWatcher, GlobPattern, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, MarkupContent, MarkupKind, MessageType, OneOf, Position,
    Range as LspRange, Registration, SemanticTokens, SemanticTokensFullOptions,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities,
    TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url,
  },
};

use crate::{
  analysis::{AnalysisInput, TypeHint, analyze},
  completion::completion_items,
  source::SourceText,
  syntax::{identifiers, semantic_tokens, token_legend},
};

/// How long a document must go unchanged before it's analyzed, so typing
/// doesn't start a compilation per keystroke.
const ANALYSIS_DELAY: Duration = Duration::from_millis(150);

struct OpenDocument {
  /// The canonical path of the file, when the document is one on disk.
  path: Option<PathBuf>,
  source: Arc<SourceText>,
  /// The client's version of the text, for tagging diagnostics.
  version: i32,
  /// Counts text changes, so results computed from older text are
  /// recognizable.
  revision: u64,
  /// Counts scheduled analyses; only the latest one's results are kept.
  latest_analysis: u64,
  /// The text revision `type_hints` were computed from.
  analyzed_revision: Option<u64>,
  type_hints: Vec<TypeHint>,
  dependencies: HashSet<PathBuf>,
  /// The names the document can refer to, from the last analysis that got
  /// far enough to know them.
  names: Vec<(String, NameKind)>,
}

#[derive(Clone)]
pub struct Backend {
  client: Client,
  documents: Arc<RwLock<HashMap<Url, OpenDocument>>>,
}

impl Backend {
  pub fn new(client: Client) -> Self {
    Self {
      client,
      documents: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  /// Analyzes `uri` once it's gone `ANALYSIS_DELAY` without another
  /// analysis being requested, then publishes the results.
  fn schedule_analysis(&self, uri: Url) {
    let backend = self.clone();
    tokio::spawn(async move { backend.analyze_when_settled(uri).await });
  }

  async fn analyze_when_settled(&self, uri: Url) {
    let ticket = {
      let mut documents = self.documents.write().await;
      let Some(document) = documents.get_mut(&uri) else {
        return;
      };
      document.latest_analysis += 1;
      document.latest_analysis
    };
    tokio::time::sleep(ANALYSIS_DELAY).await;
    let (input, revision) = {
      let documents = self.documents.read().await;
      let Some(document) = documents.get(&uri) else {
        return;
      };
      if document.latest_analysis != ticket {
        return;
      }
      let open_documents = documents
        .values()
        .filter_map(|open| {
          Some((open.path.clone()?, open.source.text().to_string()))
        })
        .collect();
      let input = AnalysisInput {
        uri: uri.clone(),
        path: document.path.clone(),
        text: document.source.text().to_string(),
        open_documents,
      };
      (input, document.revision)
    };
    let Ok(analysis) =
      tokio::task::spawn_blocking(move || analyze(&input)).await
    else {
      return;
    };
    // Publishing under the lock keeps an older analysis's diagnostics from
    // landing after a newer one's.
    let mut documents = self.documents.write().await;
    let Some(document) = documents.get_mut(&uri) else {
      return;
    };
    if document.latest_analysis != ticket {
      return;
    }
    document.analyzed_revision = Some(revision);
    document.type_hints = analysis.type_hints;
    document.dependencies = analysis.dependencies;
    if let Some(names) = analysis.names {
      document.names = names;
    }
    let version = document.version;
    self
      .client
      .publish_diagnostics(uri, analysis.diagnostics, Some(version))
      .await;
  }

  /// Schedules analyses of the open documents that import `path`.
  async fn schedule_dependents(&self, path: &PathBuf) {
    let dependents: Vec<Url> = self
      .documents
      .read()
      .await
      .iter()
      .filter(|(_, document)| document.dependencies.contains(path))
      .map(|(uri, _)| uri.clone())
      .collect();
    for uri in dependents {
      self.schedule_analysis(uri);
    }
  }

  async fn schedule_all(&self) {
    let uris: Vec<Url> = self.documents.read().await.keys().cloned().collect();
    for uri in uris {
      self.schedule_analysis(uri);
    }
  }

  async fn source(&self, uri: &Url) -> Option<Arc<SourceText>> {
    Some(self.documents.read().await.get(uri)?.source.clone())
  }

  async fn set_text(&self, uri: Url, text: String, version: i32) {
    let path = {
      let mut documents = self.documents.write().await;
      let document =
        documents
          .entry(uri.clone())
          .or_insert_with(|| OpenDocument {
            path: uri
              .to_file_path()
              .ok()
              .and_then(|path| path.canonicalize().ok()),
            source: Arc::new(SourceText::new(String::new())),
            version,
            revision: 0,
            latest_analysis: 0,
            analyzed_revision: None,
            type_hints: vec![],
            dependencies: HashSet::new(),
            names: vec![],
          });
      document.source = Arc::new(SourceText::new(text));
      document.version = version;
      document.revision += 1;
      document.path.clone()
    };
    self.schedule_analysis(uri);
    if let Some(path) = path {
      self.schedule_dependents(&path).await;
    }
  }
}

fn position_params_offset(
  source: &SourceText,
  params: &TextDocumentPositionParams,
) -> Result<usize> {
  source
    .offset(params.position)
    .ok_or_else(|| Error::invalid_params("position outside the document"))
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
  async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
    Ok(InitializeResult {
      capabilities: ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
          TextDocumentSyncKind::FULL,
        )),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        semantic_tokens_provider: Some(
          SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
              legend: token_legend(),
              range: Some(false),
              full: Some(SemanticTokensFullOptions::Bool(true)),
              ..Default::default()
            },
          ),
        ),
        completion_provider: Some(CompletionOptions {
          trigger_characters: Some(vec!["-".to_string(), "/".to_string()]),
          ..Default::default()
        }),
        document_formatting_provider: Some(OneOf::Left(true)),
        // `executeCommandProvider` is deliberately not advertised: the
        // VS Code client would register the commands itself, conflicting
        // with the extension's own registrations. Requests still work.
        ..Default::default()
      },
      ..Default::default()
    })
  }

  async fn initialized(&self, _: InitializedParams) {
    // Imported files that aren't open are read from disk, so analyses
    // depending on them must hear about changes made outside the editor.
    let watchers = DidChangeWatchedFilesRegistrationOptions {
      watchers: vec![FileSystemWatcher {
        glob_pattern: GlobPattern::String("**/*.easl".to_string()),
        kind: None,
      }],
    };
    let registration = Registration {
      id: "easl-file-watcher".to_string(),
      method: "workspace/didChangeWatchedFiles".to_string(),
      register_options: serde_json::to_value(watchers).ok(),
    };
    if let Err(error) =
      self.client.register_capability(vec![registration]).await
    {
      self
        .client
        .log_message(
          MessageType::WARNING,
          format!("couldn't watch .easl files: {error}"),
        )
        .await;
    }
  }

  async fn shutdown(&self) -> Result<()> {
    Ok(())
  }

  async fn did_open(&self, params: DidOpenTextDocumentParams) {
    let document = params.text_document;
    self
      .set_text(document.uri, document.text, document.version)
      .await;
  }

  async fn did_change(&self, params: DidChangeTextDocumentParams) {
    // With full sync, the last change holds the whole text.
    if let Some(change) = params.content_changes.into_iter().last() {
      self
        .set_text(
          params.text_document.uri,
          change.text,
          params.text_document.version,
        )
        .await;
    }
  }

  async fn did_close(&self, params: DidCloseTextDocumentParams) {
    let uri = params.text_document.uri;
    let closed = self.documents.write().await.remove(&uri);
    self.client.publish_diagnostics(uri, vec![], None).await;
    // Importers now read this file from disk, which may differ from the
    // closed buffer.
    if let Some(path) = closed.and_then(|document| document.path) {
      self.schedule_dependents(&path).await;
    }
  }

  async fn did_change_watched_files(
    &self,
    params: DidChangeWatchedFilesParams,
  ) {
    for change in params.changes {
      if change.typ == FileChangeType::CREATED {
        // A new file can satisfy an import that failed before.
        self.schedule_all().await;
        return;
      }
      let Ok(path) = change.uri.to_file_path() else {
        continue;
      };
      let path = path.canonicalize().unwrap_or(path);
      let open = self
        .documents
        .read()
        .await
        .values()
        .any(|document| document.path.as_ref() == Some(&path));
      // An open document's buffer, not its file, is what analyses see.
      if !open {
        self.schedule_dependents(&path).await;
      }
    }
  }

  async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let position_params = params.text_document_position_params;
    let documents = self.documents.read().await;
    let Some(document) = documents.get(&position_params.text_document.uri)
    else {
      return Ok(None);
    };
    // Types are only shown for the text they were inferred from.
    if document.analyzed_revision != Some(document.revision) {
      return Ok(None);
    }
    let offset = position_params_offset(&document.source, &position_params)?;
    // The innermost expression at the cursor, preferring fully-known types.
    let hint = document
      .type_hints
      .iter()
      .filter(|hint| hint.span.contains(&offset))
      .min_by_key(|hint| (hint.span.len(), !hint.fully_known));
    Ok(hint.map(|hint| Hover {
      contents: HoverContents::Markup(MarkupContent {
        kind: MarkupKind::Markdown,
        value: format!("```easl\n{}\n```", hint.description),
      }),
      range: Some(document.source.range(&hint.span)),
    }))
  }

  async fn semantic_tokens_full(
    &self,
    params: SemanticTokensParams,
  ) -> Result<Option<SemanticTokensResult>> {
    let Some(source) = self.source(&params.text_document.uri).await else {
      return Ok(None);
    };
    let tokens = tokio::task::spawn_blocking(move || semantic_tokens(&source))
      .await
      .map_err(|_| Error::internal_error())?;
    Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
      result_id: None,
      data: tokens,
    })))
  }

  async fn formatting(
    &self,
    params: DocumentFormattingParams,
  ) -> Result<Option<Vec<TextEdit>>> {
    let Some(source) = self.source(&params.text_document.uri).await else {
      return Ok(None);
    };
    let formatted = tokio::task::spawn_blocking({
      let source = source.clone();
      move || format_document(parse_easl(source.text()))
    })
    .await
    .map_err(|_| Error::internal_error())?;
    // A document that doesn't parse isn't formatted.
    let Ok(formatted) = formatted else {
      return Ok(None);
    };
    if formatted == source.text() {
      return Ok(Some(vec![]));
    }
    Ok(Some(vec![TextEdit {
      range: source.full_range(),
      new_text: formatted,
    }]))
  }

  async fn completion(
    &self,
    params: CompletionParams,
  ) -> Result<Option<CompletionResponse>> {
    let position_params = params.text_document_position;
    let (source, names) = {
      let documents = self.documents.read().await;
      let Some(document) = documents.get(&position_params.text_document.uri)
      else {
        return Ok(None);
      };
      (document.source.clone(), document.names.clone())
    };
    let cursor = position_params_offset(&source, &position_params)?;
    let text = source.text();
    let token_start = text[..cursor]
      .char_indices()
      .rev()
      .find(|(_, c)| c.is_whitespace() || "()[]{};\"".contains(*c))
      .map_or(0, |(i, c)| i + c.len_utf8());
    let replaced = LspRange {
      start: source.position(token_start),
      end: position_params.position,
    };
    Ok(Some(CompletionResponse::Array(completion_items(
      &text[token_start..cursor],
      replaced,
      &names,
      identifiers(text),
    ))))
  }

  /// Structural editing. Each command takes one argument, the selection as
  /// `[start, end]` `TextDocumentPositionParams`.
  async fn execute_command(
    &self,
    params: ExecuteCommandParams,
  ) -> Result<Option<Value>> {
    let argument = params
      .arguments
      .first()
      .ok_or_else(|| Error::invalid_params("expected one argument"))?;
    let (start, end): (TextDocumentPositionParams, TextDocumentPositionParams) =
      serde_json::from_value(argument.clone())
        .map_err(|e| Error::invalid_params(format!("bad argument: {e}")))?;
    let source = self
      .source(&start.text_document.uri)
      .await
      .ok_or_else(|| Error::invalid_params("document not open"))?;
    let selection = position_params_offset(&source, &start)?
      ..position_params_offset(&source, &end)?;
    let document = parse_easl(source.text());
    let position_json = |position: Position| {
      serde_json::json!([position.line, position.character])
    };
    Ok(match params.command.as_str() {
      "easl.expandSelection" => {
        document.expand_selection(&selection).map(|expanded| {
          let range = source.range(&expanded);
          serde_json::json!([
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character
          ])
        })
      }
      "easl.moveCursorToStart" => Some(position_json(
        source.position(document.move_cursor_to_start(&selection)),
      )),
      "easl.moveCursorToEnd" => Some(position_json(
        source.position(document.move_cursor_to_end(&selection)),
      )),
      _ => return Err(Error::method_not_found()),
    })
  }
}
