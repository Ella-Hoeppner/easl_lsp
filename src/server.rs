use std::{collections::HashMap, ops::Range, sync::Arc};

use easl::{
  compiler::{
    builtins::built_in_macros,
    program::Program,
    types::{TypeDescription, TypeState, TypeStateDescription},
  },
  format::format_document,
  parse::{parse_easl, parse_easl_without_comments, Encloser, Operator},
};
use regex::Regex;
use serde_json::Value;
use tokio::sync::RwLock;
use tower_lsp::{
  jsonrpc::{Error, Result},
  lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentFormattingParams, ExecuteCommandParams,
    Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, MarkupContent,
    MarkupKind, MessageType, OneOf, Position, Range as LspRange, SemanticToken,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions,
    SemanticTokensLegend, SemanticTokensOptions, SemanticTokensParams,
    SemanticTokensResult, SemanticTokensServerCapabilities, ServerCapabilities,
    TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url,
  },
  Client, LanguageServer,
};

// --- Semantic token type indices ---
// These must match the legend order declared in `initialize`.
const TOK_COMMENT: u32 = 0;
const TOK_KEYWORD: u32 = 1;
const TOK_NUMBER: u32 = 2;
const TOK_OPERATOR: u32 = 3;
// Encloser depths 0-6 at indices 4-10
fn encloser_token_type(depth: usize) -> u32 {
  4 + (depth.rem_euclid(7)) as u32
}

// --- Per-document cached state ---
struct DocumentState {
  text: String,
  /// Type annotations gathered after the last successful compilation.
  /// Each entry is (source_trace, is_fully_known, type_description_string).
  type_annotations: Vec<(easl::compiler::error::SourceTrace, bool, String)>,
}

// --- Backend ---
pub struct Backend {
  pub client: Client,
  documents: Arc<RwLock<HashMap<String, DocumentState>>>,
}

impl Backend {
  pub fn new(client: Client) -> Self {
    Self {
      client,
      documents: Arc::new(RwLock::new(HashMap::new())),
    }
  }

  async fn update_document(&self, uri: Url, text: String) {
    let uri_str = uri.to_string();

    // Step 1: store the new text immediately, before any async work.
    // Formatting and structural editing read only the text, so they must
    // never see a stale version. We update it here before spawning the
    // compilation so that any concurrent request already sees the new text.
    {
      let mut docs = self.documents.write().await;
      let entry = docs.entry(uri_str.clone()).or_insert_with(|| DocumentState {
        text: String::new(),
        type_annotations: vec![],
      });
      entry.text = text.clone();
    }

    // Step 2: compile asynchronously (this is the slow part).
    let text_for_compile = text;
    let (type_annotations, diagnostics) =
      tokio::task::spawn_blocking(move || compile_text(&text_for_compile))
        .await
        .unwrap_or_else(|_| (vec![], vec![]));

    // Step 3: write back the type annotations (text is already current).
    {
      let mut docs = self.documents.write().await;
      if let Some(state) = docs.get_mut(&uri_str) {
        state.type_annotations = type_annotations;
      }
    }

    self.client.publish_diagnostics(uri, diagnostics, None).await;
  }
}

/// Compile `text`, returning (type_annotations, diagnostics).
/// Runs synchronously – call via `spawn_blocking`.
fn compile_text(
  text: &str,
) -> (
  Vec<(easl::compiler::error::SourceTrace, bool, String)>,
  Vec<Diagnostic>,
) {
  let document = parse_easl_without_comments(text);

  // Report parse errors immediately.
  if !document.parsing_failures.is_empty() {
    let diagnostics = document
      .parsing_failures
      .iter()
      .filter_map(|e| {
        let (start_row, start_col) =
          document.index_to_row_and_col(e.pos.start).ok()?;
        let (end_row, end_col) =
          document.index_to_row_and_col(e.pos.end).ok()?;
        Some(Diagnostic {
          range: LspRange {
            start: Position {
              line: start_row as u32,
              character: start_col as u32,
            },
            end: Position {
              line: end_row as u32,
              character: end_col as u32,
            },
          },
          severity: Some(DiagnosticSeverity::ERROR),
          message: format!("{}", e.kind),
          source: Some("easl".to_string()),
          ..Default::default()
        })
      })
      .collect();
    return (vec![], diagnostics);
  }

  // Full compilation.
  let (mut program, mut errors) =
    Program::from_easl_document(&document, built_in_macros());
  for err in program.validate_raw_program().into_iter() {
    errors.log(err);
  }

  // Gather type annotations (only meaningful after compilation).
  let type_annotations = program
    .gather_type_annotations()
    .into_iter()
    .map(|(source_trace, typestate)| {
      let is_known = typestate.check_is_fully_known();
      let description = match typestate {
        TypeState::Known(t) => TypeDescription::from(t).to_string(),
        other => format!("?? {}", TypeStateDescription::from(other)),
      };
      (source_trace, is_known, description)
    })
    .collect();

  // Convert compiler errors to LSP diagnostics.
  let diagnostics = errors
    .into_iter()
    .flat_map(|err| {
      let message = err.kind.to_string();
      err
        .source_trace
        .into_document_positions_iter()
        .filter_map(|pos| {
          let (start_row, start_col) =
            document.index_to_row_and_col(pos.span.start).ok()?;
          let (end_row, end_col) =
            document.index_to_row_and_col(pos.span.end).ok()?;
          Some(Diagnostic {
            range: LspRange {
              start: Position {
                line: start_row as u32,
                character: start_col as u32,
              },
              end: Position {
                line: end_row as u32,
                character: end_col as u32,
              },
            },
            severity: Some(DiagnosticSeverity::ERROR),
            message: message.clone(),
            source: Some("easl".to_string()),
            ..Default::default()
          })
        })
        .collect::<Vec<_>>()
    })
    .collect();

  (type_annotations, diagnostics)
}

/// Build LSP semantic tokens from the source text.
/// Reparsed (with comments) to get full coloring.
fn build_semantic_tokens(text: &str) -> Vec<SemanticToken> {
  let document = parse_easl(text);

  // Regex for number literals (integers and floats with optional type suffix).
  let number_re = Regex::new(r"^\d+(\.\d+)?[iuf]?$").unwrap();

  // Gather colored spans via the fsexp annotation walker.
  let mut raw: Vec<(u32, u32, u32, u32)> = document
    .gather_annotations(
      (false, 0usize), // walk_state: (is_commented, depth)
      &|leaf: &str, (commented, _depth)| -> Option<u32> {
        if *commented {
          return Some(TOK_COMMENT);
        }
        if [
          "let", "defn", "def", "var", "override", "if", "when", "match",
          "enum", "struct", "fn", "return", "break", "loop", "for", "in",
        ]
        .contains(&leaf)
        {
          return Some(TOK_KEYWORD);
        }
        if number_re.is_match(leaf) {
          return Some(TOK_NUMBER);
        }
        None
      },
      &|encloser: &Encloser, (commented, depth)| {
        let new_state = (*commented, depth + 1);
        let annotation = if *commented {
          Some(TOK_COMMENT)
        } else {
          Some(encloser_token_type(*depth))
        };
        // For comment enclosers, mark subtree as commented.
        let new_state = match encloser {
          Encloser::LineComment | Encloser::BlockComment => (true, *depth),
          _ => new_state,
        };
        (new_state, annotation)
      },
      &|operator: &Operator, (commented, depth)| {
        if *operator == Operator::ExpressionComment {
          // Everything under #_ is commented out.
          ((true, *depth), None)
        } else {
          let annotation = if *commented {
            Some(TOK_COMMENT)
          } else {
            Some(TOK_OPERATOR)
          };
          ((*commented, *depth), annotation)
        }
      },
    )
    .into_iter()
    .filter_map(|(span, token_type): (Range<usize>, u32)| {
      let (line, col) = document.index_to_row_and_col(span.start).ok()?;
      let length = (span.end - span.start) as u32;
      if length == 0 {
        return None;
      }
      Some((line as u32, col as u32, length, token_type))
    })
    .collect();

  // Sort by line then column (should already be ordered, but be safe).
  raw.sort_unstable_by_key(|&(line, col, _, _)| (line, col));

  // Encode as LSP delta format.
  let mut tokens = Vec::with_capacity(raw.len());
  let mut prev_line = 0u32;
  let mut prev_start = 0u32;
  for (line, col, length, token_type) in raw {
    let delta_line = line - prev_line;
    let delta_start = if delta_line == 0 {
      col - prev_start
    } else {
      col
    };
    tokens.push(SemanticToken {
      delta_line,
      delta_start,
      length,
      token_type,
      token_modifiers_bitset: 0,
    });
    prev_line = line;
    prev_start = col;
  }
  tokens
}

/// The semantic token legend declared in `initialize`.
fn token_legend() -> SemanticTokensLegend {
  SemanticTokensLegend {
    token_types: vec![
      SemanticTokenType::COMMENT,           // 0
      SemanticTokenType::KEYWORD,           // 1
      SemanticTokenType::NUMBER,            // 2
      SemanticTokenType::OPERATOR,          // 3
      SemanticTokenType::new("encloserD0"), // 4
      SemanticTokenType::new("encloserD1"), // 5
      SemanticTokenType::new("encloserD2"), // 6
      SemanticTokenType::new("encloserD3"), // 7
      SemanticTokenType::new("encloserD4"), // 8
      SemanticTokenType::new("encloserD5"), // 9
      SemanticTokenType::new("encloserD6"), // 10
    ],
    token_modifiers: vec![],
  }
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
              work_done_progress_options: Default::default(),
              legend: token_legend(),
              range: Some(false),
              full: Some(SemanticTokensFullOptions::Bool(true)),
            },
          ),
        ),
        document_formatting_provider: Some(OneOf::Left(true)),
        // Don't advertise commands via executeCommandProvider — that causes
        // vscode-languageclient to auto-register them as VS Code commands,
        // which conflicts with the manual registrations in the extension.
        // The execute_command handler still works without this advertisement.
        ..Default::default()
      },
      ..Default::default()
    })
  }

  async fn initialized(&self, _: InitializedParams) {
    self
      .client
      .log_message(MessageType::INFO, "Easl LSP initialized")
      .await;
  }

  async fn shutdown(&self) -> Result<()> {
    Ok(())
  }

  // ---- Document lifecycle ----

  async fn did_open(&self, params: DidOpenTextDocumentParams) {
    self
      .update_document(params.text_document.uri, params.text_document.text)
      .await;
  }

  async fn did_change(&self, params: DidChangeTextDocumentParams) {
    // With FULL sync there is always exactly one change containing the full text.
    if let Some(change) = params.content_changes.into_iter().last() {
      self
        .update_document(params.text_document.uri, change.text)
        .await;
    }
  }

  async fn did_close(&self, params: DidCloseTextDocumentParams) {
    let uri = params.text_document.uri.to_string();
    self.documents.write().await.remove(&uri);
  }

  // ---- Hover (type info) ----

  async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
    let uri = params
      .text_document_position_params
      .text_document
      .uri
      .to_string();
    let line = params.text_document_position_params.position.line as usize;
    let col = params.text_document_position_params.position.character as usize;

    let docs = self.documents.read().await;
    let state = match docs.get(&uri) {
      Some(s) => s,
      None => return Ok(None),
    };

    // Re-parse (fast) to convert position to byte index.
    let document = parse_easl(&state.text);
    let char_index = match document.row_and_col_to_index(line, col) {
      Ok(i) => i,
      Err(_) => return Ok(None),
    };

    // Find the tightest spanning annotation, preferring fully-known types.
    let best = state
      .type_annotations
      .iter()
      .filter_map(|(source_trace, is_known, description)| {
        let span_len = source_trace
          .primary_position
          .as_ref()
          .filter(|pos| pos.span.contains(&char_index))
          .map(|pos| pos.span.len())?;
        Some((span_len, *is_known, description.as_str()))
      })
      .fold(
        None::<(usize, bool, &str)>,
        |best, (span_len, is_known, desc)| match best {
          None => Some((span_len, is_known, desc)),
          Some((best_len, best_known, _)) => {
            if span_len < best_len || (!best_known && is_known) {
              Some((span_len, is_known, desc))
            } else {
              best
            }
          }
        },
      );

    if let Some((_, _, description)) = best {
      Ok(Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
          kind: MarkupKind::Markdown,
          value: format!("```\n{description}\n```"),
        }),
        range: None,
      }))
    } else {
      Ok(None)
    }
  }

  // ---- Semantic tokens (syntax coloring) ----

  async fn semantic_tokens_full(
    &self,
    params: SemanticTokensParams,
  ) -> Result<Option<SemanticTokensResult>> {
    let uri = params.text_document.uri.to_string();
    let docs = self.documents.read().await;
    let text = match docs.get(&uri) {
      Some(s) => s.text.clone(),
      None => return Ok(None),
    };
    drop(docs);

    let tokens =
      tokio::task::spawn_blocking(move || build_semantic_tokens(&text))
        .await
        .map_err(|_| Error::internal_error())?;

    Ok(Some(SemanticTokensResult::Tokens(SemanticTokens {
      result_id: None,
      data: tokens,
    })))
  }

  // ---- Formatting ----

  async fn formatting(
    &self,
    params: DocumentFormattingParams,
  ) -> Result<Option<Vec<TextEdit>>> {
    let uri = params.text_document.uri.to_string();
    let docs = self.documents.read().await;
    let text = match docs.get(&uri) {
      Some(s) => s.text.clone(),
      None => return Ok(None),
    };
    drop(docs);

    // Compute end position before moving text into the closure.
    let last_line = text.lines().count().saturating_sub(1) as u32;
    let last_col = text.lines().last().map(|l| l.len() as u32).unwrap_or(0);

    let formatted =
      tokio::task::spawn_blocking(move || format_document(parse_easl(&text)))
        .await
        .map_err(|_| Error::internal_error())?;

    Ok(Some(vec![TextEdit {
      range: LspRange {
        start: Position {
          line: 0,
          character: 0,
        },
        end: Position {
          line: last_line,
          character: last_col,
        },
      },
      new_text: formatted,
    }]))
  }

  // ---- Custom commands (structural editing) ----

  async fn execute_command(
    &self,
    params: ExecuteCommandParams,
  ) -> Result<Option<Value>> {
    let command = params.command.as_str();

    // All structural-editing commands take a single argument: a JSON array
    // [startPositionParams, endPositionParams] where each is a
    // TextDocumentPositionParams.
    let (start_params, end_params) = extract_selection_args(&params)?;

    let uri = start_params.text_document.uri.to_string();
    let docs = self.documents.read().await;
    let text = match docs.get(&uri) {
      Some(s) => s.text.clone(),
      None => {
        return Err(Error::invalid_params(format!("document not open: {uri}")))
      }
    };
    drop(docs);

    let document = parse_easl(&text);

    let start_index = document
      .row_and_col_to_index(
        start_params.position.line as usize,
        start_params.position.character as usize,
      )
      .map_err(|_| Error::invalid_params("invalid start position"))?;
    let end_index = document
      .row_and_col_to_index(
        end_params.position.line as usize,
        end_params.position.character as usize,
      )
      .map_err(|_| Error::invalid_params("invalid end position"))?;

    match command {
      "easl.expandSelection" => {
        let new_selection =
          document.expand_selection(&(start_index..end_index));
        if let Some(sel) = new_selection {
          let (start_row, start_col) =
            document.index_to_row_and_col(sel.start).unwrap_or((0, 0));
          let (end_row, end_col) =
            document.index_to_row_and_col(sel.end).unwrap_or((0, 0));
          Ok(Some(
            serde_json::to_value([start_row, start_col, end_row, end_col])
              .unwrap(),
          ))
        } else {
          Ok(None)
        }
      }
      "easl.moveCursorToStart" => {
        let new_index =
          document.move_cursor_to_start(&(start_index..end_index));
        let (row, col) =
          document.index_to_row_and_col(new_index).unwrap_or((0, 0));
        Ok(Some(serde_json::to_value([row, col]).unwrap()))
      }
      "easl.moveCursorToEnd" => {
        let new_index = document.move_cursor_to_end(&(start_index..end_index));
        let (row, col) =
          document.index_to_row_and_col(new_index).unwrap_or((0, 0));
        Ok(Some(serde_json::to_value([row, col]).unwrap()))
      }
      _ => Err(Error::method_not_found()),
    }
  }
}

/// Extract (start, end) TextDocumentPositionParams from execute_command args.
fn extract_selection_args(
  params: &ExecuteCommandParams,
) -> Result<(TextDocumentPositionParams, TextDocumentPositionParams)> {
  let arg = params
    .arguments
    .first()
    .ok_or_else(|| Error::invalid_params("expected one argument"))?;
  serde_json::from_value::<(
    TextDocumentPositionParams,
    TextDocumentPositionParams,
  )>(arg.clone())
  .map_err(|e| Error::invalid_params(format!("bad argument: {e}")))
}
