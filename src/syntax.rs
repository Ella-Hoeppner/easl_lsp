//! Purely syntactic features: semantic tokens (keywords, numbers, strings,
//! comments, and depth-colored brackets) and the identifiers of a document.

use easl::parse::{Encloser, Operator, parse_easl};
use tower_lsp::lsp_types::{
  SemanticToken, SemanticTokenType, SemanticTokensLegend,
};

use crate::source::SourceText;

pub const KEYWORDS: &[&str] = &[
  "defn", "def", "var", "override", "struct", "enum", "import", "mod", "use",
  "let", "fn", "if", "when", "match", "do", "for", "while", "break",
  "continue", "return", "discard",
];

/// Token types, in legend order. The VS Code extension refers to the
/// bracket types by name (`encloserD0`..`encloserD6`) to color them.
const COMMENT: u32 = 0;
const KEYWORD: u32 = 1;
const NUMBER: u32 = 2;
const OPERATOR: u32 = 3;
const FIRST_ENCLOSER: u32 = 4;
const ENCLOSER_TYPES: [&str; 7] = [
  "encloserD0",
  "encloserD1",
  "encloserD2",
  "encloserD3",
  "encloserD4",
  "encloserD5",
  "encloserD6",
];
const ENCLOSER_COLORS: usize = ENCLOSER_TYPES.len();
const STRING: u32 = FIRST_ENCLOSER + ENCLOSER_COLORS as u32;

pub fn token_legend() -> SemanticTokensLegend {
  let mut token_types = vec![
    SemanticTokenType::COMMENT,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
  ];
  token_types.extend(ENCLOSER_TYPES.map(SemanticTokenType::new));
  token_types.push(SemanticTokenType::STRING);
  SemanticTokensLegend {
    token_types,
    token_modifiers: vec![],
  }
}

/// Whether `leaf` is a number literal: `1`, `-2.5`, `3u`, `4.f`, ...
pub fn is_number(leaf: &str) -> bool {
  let digits = leaf.strip_prefix('-').unwrap_or(leaf);
  let digits = digits.strip_suffix(['i', 'u', 'f']).unwrap_or(digits);
  digits.chars().any(|c| c.is_ascii_digit())
    && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    && digits.matches('.').count() <= 1
}

/// What surrounds a part of the tree being walked.
#[derive(Clone, Copy, PartialEq)]
enum Context {
  Code,
  Comment,
  String,
}

pub fn semantic_tokens(source: &SourceText) -> Vec<SemanticToken> {
  let document = parse_easl(source.text());
  let spans = document.gather_annotations(
    (Context::Code, 0usize),
    &|leaf: &str, (context, _)| match context {
      Context::Comment => Some(COMMENT),
      Context::String => Some(STRING),
      Context::Code if KEYWORDS.contains(&leaf) => Some(KEYWORD),
      Context::Code if is_number(leaf) => Some(NUMBER),
      Context::Code => None,
    },
    &|encloser: &Encloser, (context, depth)| match (context, encloser) {
      (Context::Comment, _) => ((Context::Comment, *depth), Some(COMMENT)),
      (_, Encloser::LineComment | Encloser::BlockComment) => {
        ((Context::Comment, *depth), Some(COMMENT))
      }
      (_, Encloser::Quote) => ((Context::String, *depth), Some(STRING)),
      (context, _) => (
        (*context, depth + 1),
        Some(FIRST_ENCLOSER + (depth % ENCLOSER_COLORS) as u32),
      ),
    },
    &|operator: &Operator, (context, depth)| match (context, operator) {
      (_, Operator::ExpressionComment) => ((Context::Comment, *depth), None),
      (Context::Comment, _) => ((Context::Comment, *depth), Some(COMMENT)),
      (context, _) => ((*context, *depth), Some(OPERATOR)),
    },
  );
  let mut pieces: Vec<(u32, u32, u32, u32)> = spans
    .iter()
    .flat_map(|(span, token_type)| {
      source
        .line_pieces(span)
        .map(move |(line, column, length)| (line, column, length, *token_type))
    })
    .collect();
  pieces.sort_unstable_by_key(|&(line, column, _, _)| (line, column));
  let mut tokens = Vec::with_capacity(pieces.len());
  let (mut previous_line, mut previous_column) = (0, 0);
  for (line, column, length, token_type) in pieces {
    tokens.push(SemanticToken {
      delta_line: line - previous_line,
      delta_start: if line == previous_line {
        column - previous_column
      } else {
        column
      },
      length,
      token_type,
      token_modifiers_bitset: 0,
    });
    (previous_line, previous_column) = (line, column);
  }
  tokens
}

/// The identifiers written in `text`, outside comments and strings.
pub fn identifiers(text: &str) -> Vec<String> {
  parse_easl(text)
    .gather_annotations(
      Context::Code,
      &|leaf: &str, context| {
        (*context == Context::Code && !is_number(leaf))
          .then(|| leaf.to_string())
      },
      &|encloser: &Encloser, context| {
        let context = match encloser {
          Encloser::LineComment | Encloser::BlockComment => Context::Comment,
          Encloser::Quote => Context::String,
          _ => *context,
        };
        (context, None)
      },
      &|operator: &Operator, context| {
        let context = match operator {
          Operator::ExpressionComment => Context::Comment,
          _ => *context,
        };
        (context, None)
      },
    )
    .into_iter()
    .map(|(_, identifier)| identifier)
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn numbers() {
    for number in ["1", "-2.5", "3u", "4.f", "5.", "0i"] {
      assert!(is_number(number), "{number}");
    }
    for word in ["-", "f", "u", "a1", "1.2.3", "x.y", "-x"] {
      assert!(!is_number(word), "{word}");
    }
  }

  #[test]
  fn identifiers_skip_comments_and_strings() {
    let mut found = identifiers(
      "(defn f [x: f32] ; comment words\n  #_(skipped) (print \"text\" x))",
    );
    found.sort();
    found.dedup();
    assert_eq!(found, vec!["defn", "f", "f32", "print", "x"]);
  }
}
