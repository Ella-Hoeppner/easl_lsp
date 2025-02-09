use std::{collections::HashMap, sync::RwLock};

use easl::{
  compiler::{
    builtins::built_in_macros,
    program::{EaslDocument, Program},
    types::TypeState,
  },
  format::format_document,
  parse::{easl_syntax_graph, Encloser, Operator},
};
use regex::Regex;
use serde_json::Value;
use sse::{str_tagged::StringTaggedSyntaxGraph, Parser};
use tower_lsp::{
  jsonrpc::{Error, Result},
  lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    ExecuteCommandOptions, ExecuteCommandParams, InitializeParams,
    InitializeResult, InitializedParams, MessageType, Position, Range,
    ServerCapabilities, TextDocumentPositionParams, TextDocumentSyncCapability,
    TextDocumentSyncKind,
  },
  Client, LanguageServer,
};

#[derive(Clone)]
enum SyntaxColorMode {
  Commented,
  Keyword,
  Number,
  Operator,
  Encloser(usize),
}

impl SyntaxColorMode {
  fn encode(self) -> usize {
    match self {
      SyntaxColorMode::Commented => 0,
      SyntaxColorMode::Keyword => 1,
      SyntaxColorMode::Number => 2,
      SyntaxColorMode::Operator => 3,
      SyntaxColorMode::Encloser(depth) => 4 + (depth.rem_euclid(7)),
    }
  }
}

#[derive(Debug)]
pub struct Backend {
  pub client: Client,
  documents: RwLock<HashMap<String, String>>,
}

impl Backend {
  pub fn new(client: Client) -> Self {
    Self {
      client,
      documents: Default::default(),
    }
  }
}

impl Backend {
  fn selection_command(
    &self,
    params: ExecuteCommandParams,
    name: &str,
    f: impl Fn(EaslDocument, usize, usize) -> Result<Option<Value>>,
  ) -> Result<Option<Value>> {
    if params.arguments.len() == 1 {
      if let Some((selection_start_params, selection_end_params)) =
        serde_json::from_value::<(
          TextDocumentPositionParams,
          TextDocumentPositionParams,
        )>(params.arguments[0].clone())
        .ok()
      {
        match self.documents.read() {
          Ok(docs) => {
            let uri = selection_start_params.text_document.uri.to_string();
            let text = docs
              .get(&uri)
              .expect(&format!("didn't have data for document {}", uri));
            let document: EaslDocument =
              Parser::new(easl_syntax_graph(), text).try_into().unwrap();
            let document_start_index = document
              .row_and_col_to_index(
                selection_start_params.position.line as usize,
                selection_start_params.position.character as usize,
              )
              .expect("invalid row and col");
            let document_end_index = document
              .row_and_col_to_index(
                selection_end_params.position.line as usize,
                selection_end_params.position.character as usize,
              )
              .expect("invalid row and col");
            f(document, document_start_index, document_end_index)
          }
          Err(e) => {
            panic!("{name} failed to read document: {e:?}")
          }
        }
      } else {
        Err(Error::invalid_params(format!(
          "Invalid parameters: {}",
          params.arguments[0]
        )))
      }
    } else {
      Err(Error::invalid_params(format!(
        "Invalid number of arguments {} to {}",
        params.arguments.len(),
        name
      )))
    }
  }
}

fn sexp_graph<'g>() -> StringTaggedSyntaxGraph<'g> {
  StringTaggedSyntaxGraph::contextless_from_descriptions(
    vec![
      ' '.to_string(),
      '\n'.to_string(),
      '\t'.to_string(),
      '\r'.to_string(),
    ],
    Some('\\'.to_string()),
    vec![("", "(", ")"), ("bracket", "[", "]")],
    vec![
      ("quote", "'", 0, 1),
      ("meta", "^", 0, 2),
      ("question", "?", 1, 0),
      ("colon", ":", 1, 1),
    ],
  )
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
  async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
    Ok(InitializeResult {
      capabilities: ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(
          TextDocumentSyncKind::FULL,
        )),
        execute_command_provider: Some(ExecuteCommandOptions {
          commands: vec![
            "expandSelection".to_string(),
            "moveCursorToStart".to_string(),
            "moveCursorToEnd".to_string(),
            "formatDocument".to_string(),
            "colorDocument".to_string(),
            "getTypeInfo".to_string(),
          ],
          work_done_progress_options: Default::default(),
        }),
        ..Default::default()
      },
      ..Default::default()
    })
  }

  async fn initialized(&self, _: InitializedParams) {
    self
      .client
      .log_message(MessageType::INFO, "server initialized!")
      .await;
  }

  async fn shutdown(&self) -> Result<()> {
    Ok(())
  }

  async fn execute_command(
    &self,
    params: ExecuteCommandParams,
  ) -> Result<Option<Value>> {
    let mut extra_messages = vec![];
    self
      .client
      .log_message(
        MessageType::INFO,
        format!("executing {}", params.command.as_str()),
      )
      .await;
    let params_clone = params.clone();
    let return_value = match params.command.as_str() {
      "expandSelection" => self.selection_command(
        params,
        "expandSelection",
        |document, start_index, end_index| {
          Ok(document.expand_selection(&(start_index..end_index)).map(
            |new_selection| {
              let (start_row, start_col) =
                document.index_to_row_and_col(new_selection.start).unwrap();
              let (end_row, end_col) =
                document.index_to_row_and_col(new_selection.end).unwrap();
              serde_json::to_value([start_row, start_col, end_row, end_col])
                .unwrap()
            },
          ))
        },
      ),
      "moveCursorToStart" => self.selection_command(
        params,
        "moveCursorToStart",
        |document, start_index, end_index| {
          let (start_row, start_col) = document
            .index_to_row_and_col(
              document.move_cursor_to_start(&(start_index..end_index)),
            )
            .unwrap();
          Ok(Some(serde_json::to_value([start_row, start_col]).unwrap()))
        },
      ),
      "moveCursorToEnd" => self.selection_command(
        params,
        "moveCursorToStart",
        |document, start_index, end_index| {
          let (start_row, start_col) = document
            .index_to_row_and_col(
              document.move_cursor_to_end(&(start_index..end_index)),
            )
            .unwrap();
          Ok(Some(serde_json::to_value([start_row, start_col]).unwrap()))
        },
      ),
      "formatDocument" => {
        let uri = params.arguments[0].as_str().unwrap().to_string();
        match self.documents.read() {
          Ok(docs) => {
            let text = docs
              .get(&uri)
              .expect(&format!("didn't have data for document {}", uri));
            match TryInto::<EaslDocument>::try_into(Parser::new(
              easl_syntax_graph(),
              text,
            )) {
              Ok(document) => Ok(Some(
                serde_json::to_value(format_document(document)).unwrap(),
              )),
              Err(_) => Ok(None),
            }
          }
          Err(e) => {
            panic!("formatDocument failed to read document: {e:?}")
          }
        }
      }
      "colorDocument" => {
        let uri = params.arguments[0].as_str().unwrap().to_string();
        match self.documents.read() {
          Ok(docs) => {
            let text = docs
              .get(&uri)
              .expect(&format!("didn't have data for document {}", uri));
            match TryInto::<EaslDocument>::try_into(Parser::new(
              easl_syntax_graph(),
              text,
            )) {
              Ok(document) => Ok(Some(
                document
                  .gather_annotations(
                    (false, 0),
                    &|leaf: &str, (commented, depth)| {
                      if *commented {
                        Some(SyntaxColorMode::Commented)
                      } else {
                        ["let", "defn", "def", "var"]
                          .contains(&leaf)
                          .then(|| SyntaxColorMode::Keyword)
                          .or_else(|| {
                            Regex::new(r"^\d+(\.\d+)?[iuf]?$")
                              .unwrap()
                              .is_match(leaf)
                              .then(|| SyntaxColorMode::Number)
                          })
                      }
                    },
                    &|_: &Encloser, (commented, depth)| {
                      (
                        (*commented, depth + 1),
                        if *commented {
                          Some(SyntaxColorMode::Commented)
                        } else {
                          Some(SyntaxColorMode::Encloser(*depth))
                        },
                      )
                    },
                    &|operator: &Operator, (commented, depth)| {
                      if *operator == Operator::ExpressionComment {
                        ((true, *depth), None)
                      } else {
                        (
                          (*commented, *depth),
                          if *commented {
                            Some(SyntaxColorMode::Commented)
                          } else {
                            Some(SyntaxColorMode::Operator)
                          },
                        )
                      }
                    },
                  )
                  .into_iter()
                  .filter_map(|(span, annotation)| {
                    if let Ok((line, col)) =
                      document.index_to_row_and_col(span.start)
                    {
                      Some([
                        line,
                        col,
                        span.end - span.start,
                        annotation.encode(),
                        0,
                      ])
                    } else {
                      None
                    }
                  })
                  .collect(),
              )),
              Err(_) => Ok(None),
            }
          }
          Err(e) => {
            panic!("formatDocument failed to read document: {e:?}")
          }
        }
      }
      "getTypeInfo" => {
        let uri = params.arguments[0].as_str().unwrap().to_string();
        let row = params.arguments[1].as_u64().unwrap() as usize;
        let col = params.arguments[2].as_u64().unwrap() as usize;
        match self.documents.read() {
          Ok(docs) => {
            let text = docs
              .get(&uri)
              .expect(&format!("didn't have data for document {}", uri));
            match TryInto::<EaslDocument>::try_into(Parser::new(
              easl_syntax_graph(),
              text,
            )) {
              Ok(document) => {
                let char_index =
                  document.row_and_col_to_index(row, col).unwrap();
                let (mut program, _) =
                  Program::from_easl_document(&document, built_in_macros());
                if let Err(err) = program.validate_raw_program() {
                  extra_messages.push(format!("{err:#?}"));
                }
                if let Some((_, best_annotation)) =
                  program.gather_type_annotations().into_iter().fold(
                    None,
                    |best: Option<(usize, TypeState)>,
                     (source_trace, typestate)| {
                      if let Some(span_length) = source_trace
                        .all_document_positions()
                        .into_iter()
                        .filter_map(|pos| {
                          pos.span.contains(&char_index).then(|| pos.span.len())
                        })
                        .min()
                      {
                        if let Some((best_length, ref best_typestate)) = best {
                          if span_length < best_length
                            || (!best_typestate.check_is_fully_known()
                              && typestate.check_is_fully_known())
                          {
                            Some((span_length, typestate))
                          } else {
                            best
                          }
                        } else {
                          Some((span_length, typestate))
                        }
                      } else {
                        best
                      }
                    },
                  )
                {
                  if let TypeState::Known(t) = best_annotation {
                    Ok(Some(t.to_string().into()))
                  } else {
                    Ok(Some(format!("not Known: {:?}", best_annotation).into()))
                  }
                } else {
                  Ok(None)
                }
              }
              Err(_) => Ok(None),
            }
          }
          Err(e) => {
            panic!("formatDocument failed to read document: {e:?}")
          }
        }
      }
      _ => Err(Error::method_not_found()),
    };
    self
      .client
      .log_message(
        MessageType::INFO,
        format!("finished executing {}", params_clone.command.as_str()),
      )
      .await;
    for extra_message in extra_messages {
      self
        .client
        .log_message(MessageType::INFO, format!("{}", extra_message))
        .await;
    }
    return_value
  }

  async fn did_open(&self, params: DidOpenTextDocumentParams) {
    let uri = params.text_document.uri.to_string();
    let text = params.text_document.text;
    match self.documents.write() {
      Ok(mut docs) => docs.insert(uri.clone(), text),
      Err(e) => panic!("did_open failed: {e:?}"),
    };
    self
      .client
      .log_message(MessageType::INFO, format!("opened {uri}"))
      .await;
  }

  async fn did_change(&self, params: DidChangeTextDocumentParams) {
    let uri = params.text_document.uri.to_string();
    let changes = params.content_changes;
    for change in changes {
      let text = match self.documents.write() {
        Ok(mut docs) => {
          docs.insert(uri.clone(), change.text.clone());
          change.text.as_str()
        }
        Err(e) => panic!("did_change failed: {e:?}"),
      };

      let error_messages: Option<
        Vec<((usize, usize), (usize, usize), String)>,
      > = match TryInto::<EaslDocument>::try_into(Parser::new(
        easl_syntax_graph(),
        text,
      )) {
        Ok(document) => {
          let (mut program, mut errors) =
            Program::from_easl_document(&document, built_in_macros());
          if let Err(err) = program.validate_raw_program() {
            errors.push(err);
          }
          Some(
            errors
              .into_iter()
              .map(|err| {
                err
                  .source_trace
                  .all_document_positions()
                  .into_iter()
                  .map(|pos| {
                    (
                      document.index_to_row_and_col(pos.span.start).unwrap(),
                      document.index_to_row_and_col(pos.span.end).unwrap(),
                      err.to_string(),
                    )
                  })
                  .collect::<Vec<_>>()
              })
              .flatten()
              .collect(),
          )
        }
        Err(_) => None,
      };

      self
        .client
        .publish_diagnostics(
          params.text_document.uri.clone(),
          error_messages
            .unwrap_or(vec![])
            .into_iter()
            .map(|((start_row, start_col), (end_row, end_col), message)| {
              Diagnostic {
                range: Range {
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
                message: message,
                source: Some("sse".to_string()),
                ..Default::default()
              }
            })
            .collect(),
          None,
        )
        .await;
    }
  }

  async fn did_close(&self, params: DidCloseTextDocumentParams) {
    let uri = params.text_document.uri.to_string();
    match self.documents.write() {
      Ok(mut docs) => docs.remove(&uri),
      Err(e) => panic!("did_close failed: {e:?}"),
    };
  }
}
