use super::host::{Key, CTRL, GUI, SHIFT};
use super::view::{EditorView, Setting};
use alloc::string::String;

const MAX_EDIT_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EditTarget {
  Draft,
  Setting(Setting),
}

pub struct Editor {
  pub title: String,
  pub value: String,
  pub composition: String,
  pub cursor: usize,
  pub secret: bool,
  pub select_all: bool,
  pub target: EditTarget,
}

pub enum EditResult {
  Continue,
  Commit,
  Cancel,
  Paste,
}

impl Editor {
  pub fn new(title: &str, value: String, secret: bool, target: EditTarget) -> Self {
    let cursor = value.len();
    Self {
      title: title.into(),
      value,
      composition: String::new(),
      cursor,
      secret,
      select_all: false,
      target,
    }
  }

  pub fn view(&self) -> EditorView<'_> {
    EditorView {
      title: &self.title,
      value: &self.value,
      composition: &self.composition,
      cursor: self.cursor,
      secret: self.secret,
      select_all: self.select_all,
    }
  }

  pub fn insert(&mut self, text: &str) {
    let retained = if self.select_all { 0 } else { self.value.len() };
    // Refuse the entire edit before changing selection or text. An oversized
    // clipboard must never erase a selected draft.
    if retained.saturating_add(text.len()) > MAX_EDIT_BYTES || text.contains('\0') {
      return;
    }
    if self.select_all {
      self.clear();
    }
    self.value.insert_str(self.cursor, text);
    self.cursor += text.len();
    self.composition.clear();
  }

  pub fn key(&mut self, key: Key, modifiers: u32) -> EditResult {
    let shortcut = modifiers & (CTRL | GUI) != 0;
    match key {
      Key::Escape => return EditResult::Cancel,
      Key::Enter if self.composition.is_empty() => {
        if modifiers & SHIFT != 0 {
          if !self.secret {
            self.insert("\n");
          }
        } else {
          return EditResult::Commit;
        }
      }
      Key::Backspace => {
        if self.select_all {
          self.clear();
        } else if self.cursor > 0 {
          let previous = self.previous();
          self.value.drain(previous..self.cursor);
          self.cursor = previous;
        }
      }
      Key::Delete => {
        if self.select_all {
          self.clear();
        } else if self.cursor < self.value.len() {
          let next = self.next();
          self.value.drain(self.cursor..next);
        }
      }
      Key::Left => {
        self.cursor = self.previous();
        self.select_all = false;
      }
      Key::Right => {
        self.cursor = self.next();
        self.select_all = false;
      }
      Key::Home => {
        self.cursor = 0;
        self.select_all = false;
      }
      Key::End => {
        self.cursor = self.value.len();
        self.select_all = false;
      }
      Key::A if shortcut => self.select_all = true,
      Key::V if shortcut => return EditResult::Paste,
      Key::U if modifiers & CTRL != 0 => self.clear(),
      _ => {}
    }
    EditResult::Continue
  }

  fn clear(&mut self) {
    self.value.clear();
    self.cursor = 0;
    self.select_all = false;
    self.composition.clear();
  }

  fn previous(&self) -> usize {
    self.value[..self.cursor].char_indices().next_back().map_or(0, |(index, _)| index)
  }

  fn next(&self) -> usize {
    self.value[self.cursor..]
      .chars()
      .next()
      .map_or(self.cursor, |character| self.cursor + character.len_utf8())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn cursor_and_deletion_preserve_multibyte_boundaries() {
    let mut editor = Editor::new("Message", "a中𠮷b".into(), false, EditTarget::Draft);
    editor.key(Key::Left, 0);
    editor.key(Key::Backspace, 0);
    assert_eq!(editor.value, "a中b");
    editor.key(Key::Left, 0);
    editor.key(Key::Delete, 0);
    assert_eq!(editor.value, "ab");
    editor.insert("中文");
    assert_eq!(editor.value, "a中文b");
    assert_eq!(editor.cursor, "a中文".len());
  }

  #[test]
  fn oversized_or_nul_paste_preserves_the_selected_draft() {
    let mut editor = Editor::new("Message", "keep me".into(), false, EditTarget::Draft);
    editor.key(Key::A, CTRL);
    editor.insert(&"x".repeat(MAX_EDIT_BYTES + 1));
    editor.insert("bad\0text");
    assert_eq!(editor.value, "keep me");
    assert!(editor.select_all);
    editor.insert("新内容");
    assert_eq!(editor.value, "新内容");
    assert!(!editor.select_all);
  }

  #[test]
  fn composition_does_not_submit_and_secret_fields_have_no_newlines() {
    let mut editor = Editor::new("Key", "key".into(), true, EditTarget::Draft);
    editor.composition = "拼音".into();
    assert!(matches!(editor.key(Key::Enter, 0), EditResult::Continue));
    editor.composition.clear();
    editor.key(Key::Enter, SHIFT);
    assert_eq!(editor.value, "key");
    assert!(matches!(editor.key(Key::Enter, 0), EditResult::Commit));
  }
}
