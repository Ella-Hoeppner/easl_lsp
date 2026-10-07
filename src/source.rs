//! Source text with conversions between byte offsets (what the compiler's
//! spans use) and LSP positions (zero-based lines, UTF-16 columns).

use std::ops::Range;

use tower_lsp::lsp_types::{Position, Range as LspRange};

pub struct SourceText {
  text: String,
  /// Byte offset of the start of each line.
  line_starts: Vec<usize>,
}

impl SourceText {
  pub fn new(text: String) -> Self {
    let line_starts = std::iter::once(0)
      .chain(text.match_indices('\n').map(|(i, _)| i + 1))
      .collect();
    Self { text, line_starts }
  }

  pub fn text(&self) -> &str {
    &self.text
  }

  /// The line containing byte `offset`, and that line's start offset.
  fn line_of(&self, offset: usize) -> (usize, usize) {
    let line = self.line_starts.partition_point(|&start| start <= offset) - 1;
    (line, self.line_starts[line])
  }

  /// The position of byte `offset`, clamped to the text and rounded down to
  /// a character boundary.
  pub fn position(&self, offset: usize) -> Position {
    let mut offset = offset.min(self.text.len());
    while !self.text.is_char_boundary(offset) {
      offset -= 1;
    }
    let (line, line_start) = self.line_of(offset);
    Position {
      line: line as u32,
      character: self.text[line_start..offset].encode_utf16().count() as u32,
    }
  }

  pub fn range(&self, span: &Range<usize>) -> LspRange {
    LspRange {
      start: self.position(span.start),
      end: self.position(span.end),
    }
  }

  /// The byte offset of `position`, or `None` past the end of its line.
  pub fn offset(&self, position: Position) -> Option<usize> {
    let line_start = *self.line_starts.get(position.line as usize)?;
    let line_end = self
      .line_starts
      .get(position.line as usize + 1)
      .map_or(self.text.len(), |next| next - 1);
    let mut column = 0;
    for (i, c) in self.text[line_start..line_end].char_indices() {
      if column >= position.character as usize {
        return Some(line_start + i);
      }
      column += c.len_utf16();
    }
    (column >= position.character as usize).then_some(line_end)
  }

  /// The range covering the whole text.
  pub fn full_range(&self) -> LspRange {
    self.range(&(0..self.text.len()))
  }

  /// `span` split at line breaks into single-line pieces, each as (line,
  /// start column, length) in UTF-16 units. Empty pieces are left out.
  pub fn line_pieces(
    &self,
    span: &Range<usize>,
  ) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
    let span = span.clone();
    let (first_line, _) = self.line_of(span.start.min(self.text.len()));
    let (last_line, _) = self.line_of(span.end.min(self.text.len()));
    (first_line..=last_line).filter_map(move |line| {
      let line_start = self.line_starts[line];
      let line_end = self
        .line_starts
        .get(line + 1)
        .map_or(self.text.len(), |next| next - 1);
      let start = span.start.max(line_start);
      let end = span.end.min(line_end);
      (start < end).then(|| {
        let start_position = self.position(start);
        let length = self.text[start..end].encode_utf16().count() as u32;
        (start_position.line, start_position.character, length)
      })
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn positions_count_utf16_units() {
    let source = SourceText::new("(a\n é 𝄞 b)".to_string());
    let b = source.text().find('b').unwrap();
    assert_eq!(
      source.position(b),
      Position {
        line: 1,
        character: 6
      }
    );
    assert_eq!(
      source.offset(Position {
        line: 1,
        character: 6
      }),
      Some(b)
    );
    assert_eq!(
      source.offset(Position {
        line: 0,
        character: 2
      }),
      Some(2)
    );
    assert_eq!(
      source.offset(Position {
        line: 0,
        character: 3
      }),
      None
    );
  }

  #[test]
  fn spans_split_at_line_breaks() {
    let source = SourceText::new("#|ab\ncd|#".to_string());
    let pieces: Vec<_> = source.line_pieces(&(0..9)).collect();
    assert_eq!(pieces, vec![(0, 0, 4), (1, 0, 4)]);
  }
}
