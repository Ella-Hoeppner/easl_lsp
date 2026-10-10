//! Structural edits on the syntax tree.

use std::ops::Range;

use easl::parse::{EaslTree, Encloser};
use fsexp::syntax::EncloserOrOperator;

/// Moving a form past its neighbor: replace `replaced` with `new_text`, then
/// select `selection` (in the edited text).
#[derive(Debug, PartialEq)]
pub struct FormMove {
  pub replaced: Range<usize>,
  pub new_text: String,
  pub selection: Range<usize>,
}

fn is_comment(tree: &EaslTree) -> bool {
  matches!(
    tree,
    EaslTree::Inner(
      (
        _,
        EncloserOrOperator::Encloser(
          Encloser::LineComment | Encloser::BlockComment
        )
      ),
      _
    )
  )
}

fn contains(span: &Range<usize>, selection: &Range<usize>) -> bool {
  span.start <= selection.start && selection.end <= span.end
}

/// Swaps the form containing `selection` with its next (`forward`) or
/// previous sibling, skipping comments. The form moved is the innermost one
/// containing the selection whose parent is a bracketed form (or the
/// document): a form under an operator (`@cpu (defn ...)`, `x: f32`) moves
/// with its operator. The selection keeps its place within the form.
pub fn move_form(
  text: &str,
  trees: &[EaslTree],
  selection: &Range<usize>,
  forward: bool,
) -> Option<FormMove> {
  // The innermost (siblings, index) whose siblings are a bracketed form's
  // children or the document's top-level forms.
  let mut found: Option<(&[EaslTree], usize)> = None;
  let mut siblings = trees;
  let mut in_bracketed_list = true;
  loop {
    let Some(index) = siblings
      .iter()
      .position(|tree| contains(&tree.position().span, selection))
    else {
      break;
    };
    if in_bracketed_list {
      found = Some((siblings, index));
    }
    match &siblings[index] {
      EaslTree::Inner((position, kind), children)
        if position.span != *selection =>
      {
        in_bracketed_list = matches!(kind, EncloserOrOperator::Encloser(_));
        siblings = children;
      }
      _ => break,
    }
  }
  let (siblings, index) = found?;
  let neighbor = if forward {
    (index + 1..siblings.len()).find(|&i| !is_comment(&siblings[i]))?
  } else {
    (0..index).rev().find(|&i| !is_comment(&siblings[i]))?
  };
  let moved = siblings[index].position().span.clone();
  let other = siblings[neighbor].position().span.clone();
  let (first, second) = if forward {
    (&moved, &other)
  } else {
    (&other, &moved)
  };
  let new_text = format!(
    "{}{}{}",
    &text[second.clone()],
    &text[first.end..second.start],
    &text[first.clone()]
  );
  let moved_start = if forward {
    first.start + (second.end - first.end)
  } else {
    first.start
  };
  let shift = |offset: usize| offset - moved.start + moved_start;
  Some(FormMove {
    replaced: first.start..second.end,
    new_text,
    selection: shift(selection.start)..shift(selection.end),
  })
}

#[cfg(test)]
mod tests {
  use easl::parse::parse_easl;

  use super::*;

  /// `text` after moving the form at `cursor` (marked by `|`), with the
  /// cursor's new place marked.
  fn moved(text: &str, forward: bool) -> Option<String> {
    let cursor = text.find('|').unwrap();
    let text = text.replace('|', "");
    let document = parse_easl(&text);
    let form_move =
      move_form(&text, &document.syntax_trees, &(cursor..cursor), forward)?;
    let mut result = text.clone();
    result.replace_range(form_move.replaced, &form_move.new_text);
    result.insert(form_move.selection.start, '|');
    Some(result)
  }

  #[test]
  fn moves_within_the_enclosing_form() {
    assert_eq!(moved("(f a| b c)", true).as_deref(), Some("(f b a| c)"));
    assert_eq!(moved("(f a b| c)", false).as_deref(), Some("(f b| a c)"));
    assert_eq!(
      moved("(f a |(g x) c)", true).as_deref(),
      Some("(f a c |(g x))")
    );
    assert_eq!(moved("(f a b c|)", true), None);
    assert_eq!(moved("(f a (g |x) c)", true), None);
  }

  #[test]
  fn moves_top_level_forms() {
    assert_eq!(
      moved("|(def a: f32 1.)\n\n(def b: f32 2.)", true).as_deref(),
      Some("(def b: f32 2.)\n\n|(def a: f32 1.)")
    );
  }

  #[test]
  fn operator_forms_move_whole() {
    assert_eq!(
      moved("(defn g [])\n@cpu\n|(defn f [])", false).as_deref(),
      Some("@cpu\n|(defn f [])\n(defn g [])")
    );
    assert_eq!(
      moved("[x: |f32 y: u32]", true).as_deref(),
      Some("[y: u32 x: |f32]")
    );
  }

  #[test]
  fn comments_are_skipped() {
    assert_eq!(
      moved("(f a| ; note\n b)", true).as_deref(),
      Some("(f b ; note\n a|)")
    );
  }
}
