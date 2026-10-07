//! Completion candidates: the names a document can refer to, words written
//! in it, and the language's builtins and keywords.
//!
//! Every candidate is returned for every request; the editor does the
//! (fuzzy) filtering against the word being typed.

use std::{collections::HashMap, sync::LazyLock};

use easl::compiler::{
  builtins::built_in_macros, modules::NameKind, program::Program,
  types::PRIMITIVE_TYPE_NAMES,
};
use tower_lsp::lsp_types::{
  CompletionItem, CompletionItemKind, CompletionTextEdit, Range, TextEdit,
};

use crate::syntax::KEYWORDS;

/// Where a candidate comes from, in the order they're listed when the editor
/// ranks them equally.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
  /// A definition the document can refer to (its own or imported).
  Program,
  /// A word written in the document (locals, mostly).
  Document,
  Builtin,
}

#[derive(Clone)]
struct Candidate {
  kind: CompletionItemKind,
  source: Source,
}

fn name_kind_item_kind(kind: NameKind) -> CompletionItemKind {
  match kind {
    NameKind::Function => CompletionItemKind::FUNCTION,
    NameKind::Variable => CompletionItemKind::VARIABLE,
    NameKind::Struct => CompletionItemKind::STRUCT,
    NameKind::Enum => CompletionItemKind::ENUM,
    NameKind::Variant => CompletionItemKind::ENUM_MEMBER,
    NameKind::Module => CompletionItemKind::MODULE,
  }
}

/// The builtins, keywords, and macros, by name.
static BUILTINS: LazyLock<Vec<(String, CompletionItemKind)>> =
  LazyLock::new(|| {
    let program = Program::default();
    let mut builtins: HashMap<String, CompletionItemKind> = HashMap::new();
    // Inserted from least to most specific kind: a builtin struct's
    // constructor is listed as the struct.
    for name in program.abstract_functions.keys() {
      builtins.insert(name.to_string(), CompletionItemKind::FUNCTION);
    }
    for name in PRIMITIVE_TYPE_NAMES {
      builtins.insert(name.to_string(), CompletionItemKind::STRUCT);
    }
    for s in program.typedefs.structs.iter() {
      builtins.insert(s.name.0.to_string(), CompletionItemKind::STRUCT);
    }
    for (alias, _) in program.typedefs.type_aliases.iter() {
      builtins.insert(alias.to_string(), CompletionItemKind::STRUCT);
    }
    for e in program.typedefs.enums.iter() {
      builtins.insert(e.name.0.to_string(), CompletionItemKind::ENUM);
      for variant in e.variants.iter() {
        builtins
          .insert(variant.name.to_string(), CompletionItemKind::ENUM_MEMBER);
      }
    }
    for name in ["true", "false"] {
      builtins.insert(name.to_string(), CompletionItemKind::CONSTANT);
    }
    for name in built_in_macros()
      .iter()
      .flat_map(|m| m.reserved_names.iter())
      .map(|name| name.to_string())
      .chain(KEYWORDS.iter().map(|keyword| keyword.to_string()))
    {
      builtins.insert(name, CompletionItemKind::KEYWORD);
    }
    // Compiler-internal names (`$fnbox-make`, ...) can't be written, and
    // operators (`+`, `->`, ...) aren't worth completing.
    builtins.retain(|name, _| {
      !name.starts_with('$') && name.chars().any(char::is_alphanumeric)
    });
    builtins.into_iter().collect()
  });

/// The completion items for a cursor inside the word `typed`, which the
/// chosen item replaces (`replaced`).
pub fn completion_items(
  typed: &str,
  replaced: Range,
  program_names: &[(String, NameKind)],
  document_words: Vec<String>,
) -> Vec<CompletionItem> {
  let mut candidates: HashMap<String, Candidate> = HashMap::new();
  let mut add = |name: String, kind, source| {
    let candidate = Candidate { kind, source };
    candidates
      .entry(name)
      .and_modify(|existing| {
        if source < existing.source {
          *existing = candidate.clone();
        }
      })
      .or_insert(candidate);
  };
  for (name, kind) in BUILTINS.iter() {
    add(name.clone(), *kind, Source::Builtin);
  }
  // The word being typed is in the document too; it's only a candidate if
  // it's also written elsewhere.
  let typed_count = document_words.iter().filter(|w| *w == typed).count();
  for word in document_words {
    if word != typed || typed_count > 1 {
      add(word, CompletionItemKind::VARIABLE, Source::Document);
    }
  }
  for (name, kind) in program_names {
    add(name.clone(), name_kind_item_kind(*kind), Source::Program);
  }
  candidates
    .into_iter()
    .map(|(name, candidate)| CompletionItem {
      label: name.clone(),
      kind: Some(candidate.kind),
      sort_text: Some(format!("{}{name}", candidate.source as u8)),
      text_edit: Some(CompletionTextEdit::Edit(TextEdit {
        range: replaced,
        new_text: name,
      })),
      ..Default::default()
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn kind_of(
    items: &[CompletionItem],
    label: &str,
  ) -> Option<CompletionItemKind> {
    items.iter().find(|item| item.label == label)?.kind
  }

  #[test]
  fn candidates_from_every_source() {
    let items = completion_items(
      "ge",
      Range::default(),
      &[("geo/Point".to_string(), NameKind::Struct)],
      vec!["ge".to_string(), "my-local".to_string()],
    );
    assert_eq!(
      kind_of(&items, "geo/Point"),
      Some(CompletionItemKind::STRUCT)
    );
    assert_eq!(
      kind_of(&items, "my-local"),
      Some(CompletionItemKind::VARIABLE)
    );
    assert_eq!(kind_of(&items, "sin"), Some(CompletionItemKind::FUNCTION));
    assert_eq!(kind_of(&items, "vec4f"), Some(CompletionItemKind::STRUCT));
    assert_eq!(
      kind_of(&items, "Some"),
      Some(CompletionItemKind::ENUM_MEMBER)
    );
    assert_eq!(kind_of(&items, "when"), Some(CompletionItemKind::KEYWORD));
    assert_eq!(kind_of(&items, "defn"), Some(CompletionItemKind::KEYWORD));
    assert_eq!(kind_of(&items, "ge"), None);
    assert!(items.iter().all(|item| !item.label.starts_with('$')));
    assert_eq!(kind_of(&items, "+"), None);
  }

  #[test]
  fn program_names_outrank_words_and_builtins() {
    let items = completion_items(
      "",
      Range::default(),
      &[("sin".to_string(), NameKind::Variable)],
      vec!["sin".to_string()],
    );
    assert_eq!(kind_of(&items, "sin"), Some(CompletionItemKind::VARIABLE));
    let sort_text = |label: &str| {
      items
        .iter()
        .find(|item| item.label == label)
        .unwrap()
        .sort_text
        .clone()
        .unwrap()
    };
    assert!(sort_text("sin") < sort_text("cos"));
  }
}
