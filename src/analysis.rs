//! Compiling a document, with everything it imports, into what the editor
//! shows: diagnostics, inferred types for hover, and the names in scope.
//!
//! Each document is analyzed as the main file of its own program. A file
//! never sees the files importing it, so this is exactly what the compiler
//! says about that file wherever it's used.

use std::{
  collections::{HashMap, HashSet, VecDeque},
  fs,
  ops::Range,
  panic::{AssertUnwindSafe, catch_unwind},
  path::{Path, PathBuf},
};

use easl::{
  compiler::{
    builtins::built_in_macros,
    error::{CompileError, ErrorLog},
    modules::NameKind,
    program::{CompilerTarget, Program},
    types::{TypeDescription, TypeState, TypeStateDescription},
  },
  parse::{
    EaslMultiDocument, load_and_parse_easl_multidocument_with_lookup_function,
    parse_easl_without_comments,
  },
};
use fsexp::document::DocumentPosition;
use tower_lsp::lsp_types::{
  Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, Location,
  Range as LspRange, Url,
};

use crate::source::SourceText;

pub struct AnalysisInput {
  pub uri: Url,
  /// The document's canonical path, when it's a file on disk. Without one,
  /// the document is compiled alone and its imports can't be resolved.
  pub path: Option<PathBuf>,
  pub text: String,
  /// The text of every open document, by canonical path. These take the
  /// place of the files on disk, so unsaved edits are seen everywhere.
  pub open_documents: HashMap<PathBuf, String>,
}

/// The inferred type of an expression of the analyzed document.
pub struct TypeHint {
  pub span: Range<usize>,
  pub fully_known: bool,
  pub description: String,
}

pub struct Analysis {
  pub diagnostics: Vec<Diagnostic>,
  pub type_hints: Vec<TypeHint>,
  /// The canonical paths of every file the document imports, directly or
  /// not: a change to any of them changes this analysis.
  pub dependencies: HashSet<PathBuf>,
  /// The names the document can refer to, when name resolution got far
  /// enough to know them.
  pub names: Option<Vec<(String, NameKind)>>,
}

pub fn analyze(input: &AnalysisInput) -> Analysis {
  catch_unwind(AssertUnwindSafe(|| analyze_unguarded(input))).unwrap_or_else(
    |panic| {
      let message = panic
        .downcast_ref::<&str>()
        .map(|message| message.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown cause".to_string());
      Analysis {
        diagnostics: vec![error_diagnostic(
          LspRange::default(),
          format!("internal compiler error: {message}"),
          vec![],
        )],
        type_hints: vec![],
        dependencies: HashSet::new(),
        names: None,
      }
    },
  )
}

/// The outcome of loading a program's files.
enum Loaded {
  /// Some file failed to parse.
  ParseFailures(EaslMultiDocument),
  /// Loading failed for another reason (e.g. an import that can't be found).
  LoadErrors(EaslMultiDocument, ErrorLog),
  Loaded(EaslMultiDocument),
}

fn load(input: &AnalysisInput) -> Loaded {
  if let Some(path) = &input.path {
    let loaded =
      load_and_parse_easl_multidocument_with_lookup_function(path, |path| {
        match input.open_documents.get(path) {
          Some(text) => Ok(text.clone()),
          None => fs::read_to_string(path),
        }
      });
    // An IO error here means the document itself couldn't be read from
    // disk (e.g. it was deleted while open): fall back to compiling the
    // text alone.
    if let Ok(loaded) = loaded {
      return match loaded {
        Err(documents) => Loaded::ParseFailures(documents),
        Ok(Err((documents, errors))) => Loaded::LoadErrors(documents, errors),
        Ok(Ok(documents)) => Loaded::Loaded(documents),
      };
    }
  }
  let document = parse_easl_without_comments(&input.text);
  let parse_failed = !document.parsing_failures.is_empty();
  let documents = EaslMultiDocument::from_singular_document(
    document,
    String::new(),
    input.text.clone(),
  );
  if parse_failed {
    Loaded::ParseFailures(documents)
  } else {
    Loaded::Loaded(documents)
  }
}

fn analyze_unguarded(input: &AnalysisInput) -> Analysis {
  let mut type_hints = vec![];
  let mut names = None;
  let (documents, errors) = match load(input) {
    Loaded::ParseFailures(documents) => (documents, ErrorLog::new()),
    Loaded::LoadErrors(documents, errors) => (documents, errors),
    Loaded::Loaded(documents) => {
      let (mut program, mut errors) =
        Program::from_easl_documents(&documents, built_in_macros());
      names = Some(
        program
          .main_file_names
          .iter()
          .map(|(name, kind)| (name.to_string(), *kind))
          .collect(),
      );
      if errors.is_empty() {
        errors = program.validate_raw_program(CompilerTarget::WGSL);
        type_hints = gather_type_hints(&program);
      }
      (documents, errors)
    }
  };
  let files = Files::new(input, &documents);
  let mut diagnostics: Vec<Diagnostic> = errors
    .into_iter()
    .map(|error| files.compile_error_diagnostic(error))
    .collect();
  for (index, (document, _, _)) in documents.sources.iter().enumerate() {
    for failure in document.parsing_failures.iter() {
      diagnostics.push(files.diagnostic(
        format!("{}", failure.kind),
        Some((index, failure.pos.clone())),
        vec![],
      ));
    }
  }
  diagnostics.sort_by_key(|diagnostic| {
    (
      diagnostic.range.start.line,
      diagnostic.range.start.character,
      diagnostic.message.clone(),
    )
  });
  diagnostics.dedup();
  Analysis {
    diagnostics,
    type_hints,
    dependencies: documents
      .sources
      .iter()
      .skip(1)
      .map(|(_, path, _)| PathBuf::from(path))
      .collect(),
    names,
  }
}

fn gather_type_hints(program: &Program) -> Vec<TypeHint> {
  program
    .gather_type_annotations()
    .into_iter()
    .filter_map(|(source_trace, typestate)| {
      let position = source_trace.primary_position?;
      (position.path.first() == Some(&0)).then(|| TypeHint {
        span: position.span,
        fully_known: typestate.check_is_fully_known(),
        description: match typestate {
          TypeState::Known(t) => TypeDescription::from(t).to_string(),
          other => format!("?? {}", TypeStateDescription::from(other)),
        },
      })
    })
    .collect()
}

fn error_diagnostic(
  range: LspRange,
  message: String,
  related_information: Vec<DiagnosticRelatedInformation>,
) -> Diagnostic {
  Diagnostic {
    range,
    severity: Some(DiagnosticSeverity::ERROR),
    source: Some("easl".to_string()),
    message,
    related_information: (!related_information.is_empty())
      .then_some(related_information),
    ..Default::default()
  }
}

/// A span in one of the program's documents, by document index.
type Spot = (usize, Range<usize>);

/// The program's documents, for placing diagnostics. Document 0 is the
/// analyzed document; diagnostics about the others go on the analyzed
/// document's import form that brings them in.
struct Files<'a> {
  documents: &'a EaslMultiDocument,
  sources: Vec<SourceText>,
  uris: Vec<Option<Url>>,
  /// For each imported document, the span of the analyzed document's
  /// import form through which it's loaded.
  imported_via: HashMap<usize, Range<usize>>,
}

impl<'a> Files<'a> {
  fn new(input: &AnalysisInput, documents: &'a EaslMultiDocument) -> Self {
    let sources = documents
      .sources
      .iter()
      .map(|(_, _, text)| SourceText::new(text.clone()))
      .collect();
    let uris = documents
      .sources
      .iter()
      .enumerate()
      .map(|(index, (_, path, _))| {
        if index == 0 {
          Some(input.uri.clone())
        } else {
          Url::from_file_path(path).ok()
        }
      })
      .collect();
    Self {
      documents,
      sources,
      uris,
      imported_via: imported_via(documents),
    }
  }

  fn file_name(&self, index: usize) -> String {
    let path = &self.documents.sources[index].1;
    Path::new(path)
      .file_name()
      .map(|name| name.to_string_lossy().into_owned())
      .unwrap_or_else(|| path.clone())
  }

  fn location(&self, (index, span): &Spot) -> Option<Location> {
    Some(Location {
      uri: self.uris.get(*index)?.clone()?,
      range: self.sources[*index].range(span),
    })
  }

  fn diagnostic(
    &self,
    message: String,
    primary: Option<Spot>,
    secondaries: Vec<Spot>,
  ) -> Diagnostic {
    let mut related_information = vec![];
    let (range, message) = match primary {
      Some((0, span)) => (self.sources[0].range(&span), message),
      Some(spot) => {
        if let Some(location) = self.location(&spot) {
          related_information.push(DiagnosticRelatedInformation {
            location,
            message: message.clone(),
          });
        }
        let range = self
          .imported_via
          .get(&spot.0)
          .map(|span| self.sources[0].range(span))
          .unwrap_or_default();
        (range, format!("in {}: {message}", self.file_name(spot.0)))
      }
      None => (LspRange::default(), message),
    };
    for spot in secondaries {
      if let Some(location) = self.location(&spot) {
        related_information.push(DiagnosticRelatedInformation {
          location,
          message: "related location".to_string(),
        });
      }
    }
    error_diagnostic(range, message, related_information)
  }

  fn compile_error_diagnostic(&self, error: CompileError) -> Diagnostic {
    let spot = |position: DocumentPosition| {
      position.path.first().map(|index| (*index, position.span))
    };
    self.diagnostic(
      error.kind.to_string(),
      error.source_trace.primary_position.and_then(spot),
      error
        .source_trace
        .secondary_positions
        .into_iter()
        .filter_map(spot)
        .collect(),
    )
  }
}

/// For each imported document, the span of the main document's import form
/// through which it's (first) loaded.
fn imported_via(documents: &EaslMultiDocument) -> HashMap<usize, Range<usize>> {
  let mut imports: Vec<(usize, Range<usize>, usize)> = documents
    .imports
    .iter()
    .filter_map(|(position, target)| {
      Some((*position.path.first()?, position.span.clone(), *target))
    })
    .collect();
  imports.sort_by_key(|(from, span, _)| (*from, span.start));
  let mut via: HashMap<usize, Range<usize>> = HashMap::new();
  let mut queue = VecDeque::new();
  for (from, span, target) in imports.iter() {
    if *from == 0 && *target != 0 && !via.contains_key(target) {
      via.insert(*target, span.clone());
      queue.push_back(*target);
    }
  }
  while let Some(document) = queue.pop_front() {
    let span = via[&document].clone();
    for (from, _, target) in imports.iter() {
      if *from == document && *target != 0 && !via.contains_key(target) {
        via.insert(*target, span.clone());
        queue.push_back(*target);
      }
    }
  }
  via
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::{AtomicUsize, Ordering};

  use super::*;

  /// A fresh directory holding `files`, and the canonical path of each.
  fn project(files: &[(&str, &str)]) -> HashMap<String, PathBuf> {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let directory = std::env::temp_dir().join(format!(
      "easl_lsp_test_{}_{}",
      std::process::id(),
      COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&directory).unwrap();
    files
      .iter()
      .map(|(name, text)| {
        let path = directory.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        (name.to_string(), path.canonicalize().unwrap())
      })
      .collect()
  }

  fn analyze_file(
    path: &Path,
    open_documents: HashMap<PathBuf, String>,
  ) -> Analysis {
    let text = open_documents
      .get(path)
      .cloned()
      .unwrap_or_else(|| fs::read_to_string(path).unwrap());
    analyze(&AnalysisInput {
      uri: Url::from_file_path(path).unwrap(),
      path: Some(path.to_path_buf()),
      text,
      open_documents,
    })
  }

  fn messages(analysis: &Analysis) -> Vec<String> {
    analysis
      .diagnostics
      .iter()
      .map(|diagnostic| diagnostic.message.clone())
      .collect()
  }

  const GEOMETRY: &str = "(struct Point
  x: f32
  y: f32)

(enum Shape
  (Circle f32)
  Empty)

(defn area [s: Shape]: f32
  (match s
    (Shape/Circle r) (* 3. r r)
    Shape/Empty 0.))
";

  #[test]
  fn imported_names_resolve() {
    let files = project(&[
      ("geometry.easl", GEOMETRY),
      (
        "main.easl",
        "(import geo \"geometry.easl\")
(import \"geometry.easl\")

(defn f []: f32
  (+ (area (Shape/Circle 1.)) (geo/area geo/Shape/Empty) (.x (Point 1. 2.))))
",
      ),
    ]);
    let analysis = analyze_file(&files["main.easl"], HashMap::new());
    assert_eq!(messages(&analysis), Vec::<String>::new());
    assert_eq!(
      analysis.dependencies,
      HashSet::from([files["geometry.easl"].clone()])
    );
    let names: Vec<String> = analysis
      .names
      .unwrap()
      .into_iter()
      .map(|(name, _)| name)
      .collect();
    for name in ["area", "geo/Point", "geo/Shape/Circle", "Shape", "f"] {
      assert!(names.contains(&name.to_string()), "{name} in {names:?}");
    }
    assert!(!analysis.type_hints.is_empty());
  }

  #[test]
  fn imported_file_errors_land_on_the_import() {
    let files = project(&[
      ("lib.easl", "(defn broken []: f32\n  nope)\n"),
      (
        "main.easl",
        "(import lib \"lib.easl\")\n\n(def x: f32\n     1.)\n",
      ),
    ]);
    let analysis = analyze_file(&files["main.easl"], HashMap::new());
    let diagnostic = analysis
      .diagnostics
      .iter()
      .find(|diagnostic| diagnostic.message.contains("nope"))
      .expect("an error from lib.easl");
    assert!(diagnostic.message.starts_with("in lib.easl: "));
    assert_eq!(diagnostic.range.start.line, 0);
    let related = diagnostic.related_information.as_ref().unwrap();
    assert_eq!(
      related[0].location.uri,
      Url::from_file_path(&files["lib.easl"]).unwrap()
    );
    assert_eq!(related[0].location.range.start.line, 1);
  }

  #[test]
  fn open_documents_override_files() {
    let files = project(&[
      ("lib.easl", "(defn old []: f32\n  1.)\n"),
      (
        "main.easl",
        "(import \"lib.easl\")\n\n(defn f []: f32\n  (new))\n",
      ),
    ]);
    assert!(
      !analyze_file(&files["main.easl"], HashMap::new())
        .diagnostics
        .is_empty()
    );
    let unsaved = HashMap::from([(
      files["lib.easl"].clone(),
      "(defn new []: f32\n  2.)\n".to_string(),
    )]);
    assert_eq!(
      messages(&analyze_file(&files["main.easl"], unsaved)),
      Vec::<String>::new()
    );
  }

  #[test]
  fn missing_and_unparsable_imports() {
    let files = project(&[
      ("bad.easl", "(defn f []: f32\n  (+ 1. 2.)\n"),
      (
        "main.easl",
        "(def x: f32\n     1.)\n\n(import \"missing.easl\")\n",
      ),
      ("other.easl", "(import \"bad.easl\")\n"),
    ]);
    let missing = analyze_file(&files["main.easl"], HashMap::new());
    assert_eq!(missing.diagnostics.len(), 1);
    assert_eq!(missing.diagnostics[0].range.start.line, 3);
    assert!(missing.diagnostics[0].message.contains("missing.easl"));
    let unparsable = analyze_file(&files["other.easl"], HashMap::new());
    assert_eq!(unparsable.diagnostics.len(), 1);
    assert!(
      unparsable.diagnostics[0]
        .message
        .starts_with("in bad.easl: ")
    );
    assert_eq!(unparsable.diagnostics[0].range.start.line, 0);
  }

  #[test]
  fn type_hints_are_from_the_analyzed_document() {
    let files = project(&[
      ("lib.easl", "(defn g [x: u32]: u32\n  (* x 2u))\n"),
      (
        "main.easl",
        "(import \"lib.easl\")\n\n(defn f []: u32\n  (g 1u))\n",
      ),
    ]);
    let analysis = analyze_file(&files["main.easl"], HashMap::new());
    let text = fs::read_to_string(&files["main.easl"]).unwrap();
    assert!(
      analysis
        .type_hints
        .iter()
        .all(|hint| hint.span.end <= text.len())
    );
    let call = text.find("(g 1u)").unwrap();
    assert!(analysis.type_hints.iter().any(|hint| {
      hint.span == (call..call + "(g 1u)".len()) && hint.description == "u32"
    }));
  }
}
