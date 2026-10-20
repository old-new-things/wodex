//! CommonMark/GFM converted into a flat, bounded native drawing model.
//!
//! The parser has no HTML renderer or I/O features. URLs and image descriptions
//! are text only. Excessive structure falls back to the complete original text,
//! so a streamed or unusual answer remains readable without recursive layout.
use alloc::{
  format,
  string::{String, ToString},
  vec,
  vec::Vec,
};
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

const MAX_INPUT_BYTES: usize = 512 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_BLOCKS: usize = 2048;
const MAX_SPANS: usize = 8192;
const MAX_EVENTS: usize = 65536;
const MAX_TABLE_COLUMNS: usize = 32;
const MAX_TABLE_CELLS: usize = 4096;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
  pub bold: bool,
  pub italic: bool,
  pub strike: bool,
  pub code: bool,
  pub link: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
  pub text: String,
  pub style: Style,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Alignment {
  Left,
  Center,
  Right,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockKind {
  Paragraph(Vec<Span>),
  Heading { level: u8, spans: Vec<Span> },
  Code { language: String, text: String },
  Rule,
  Table { headers: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>>, alignments: Vec<Alignment> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
  pub indent: u8,
  pub quote_depth: u8,
  pub marker: Option<String>,
  pub kind: BlockKind,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Document {
  pub blocks: Vec<Block>,
}

/// Parse the current message snapshot, including an unfinished streamed suffix.
/// CommonMark treats unmatched inline delimiters as text and accepts an open
/// fenced code block through the end of the current snapshot.
pub fn parse(source: &str) -> Document {
  if source.is_empty() {
    return Document::default();
  }
  // Bound first-pass allocation as well as our output model. A very wide table
  // can otherwise synthesize many empty cells before events become available.
  if source.len() > MAX_INPUT_BYTES
    || source.lines().any(|line| {
      line.bytes().filter(|&byte| byte == b'|').take(MAX_TABLE_COLUMNS + 2).count()
        > MAX_TABLE_COLUMNS + 1
        || line
          .bytes()
          .take_while(|byte| matches!(byte, b'>' | b' ' | b'\t'))
          .filter(|&byte| byte == b'>')
          .take(MAX_DEPTH + 1)
          .count()
          > MAX_DEPTH
    })
  {
    return plain(source);
  }
  let options = Options::ENABLE_TABLES
    | Options::ENABLE_STRIKETHROUGH
    | Options::ENABLE_TASKLISTS
    | Options::ENABLE_CJK_FRIENDLY_EMPHASIS;
  let mut builder = Builder::new(source.len());
  for (count, event) in Parser::new_ext(source, options).enumerate() {
    if count >= MAX_EVENTS || !builder.event(event) || !builder.within_limits() {
      return plain(source);
    }
  }
  builder.flush();
  builder.document
}

fn plain(source: &str) -> Document {
  Document {
    blocks: vec![Block {
      indent: 0,
      quote_depth: 0,
      marker: None,
      kind: BlockKind::Paragraph(vec![Span { text: source.into(), style: Style::default() }]),
    }],
  }
}

struct Link {
  destination: String,
  label: String,
  image: bool,
  autolink: bool,
}

struct Table {
  block: Block,
  row: Vec<Vec<Span>>,
  cell: Vec<Span>,
  cells: usize,
}

struct Builder {
  document: Document,
  pending: Option<Block>,
  table: Option<Table>,
  lists: Vec<Option<u64>>,
  items: Vec<Option<String>>,
  links: Vec<Link>,
  quote_depth: usize,
  depth: usize,
  bold: usize,
  italic: usize,
  strike: usize,
  spans: usize,
  bytes: usize,
  max_bytes: usize,
}

impl Builder {
  fn new(source_bytes: usize) -> Self {
    Self {
      document: Document::default(),
      pending: None,
      table: None,
      lists: Vec::new(),
      items: Vec::new(),
      links: Vec::new(),
      quote_depth: 0,
      depth: 0,
      bold: 0,
      italic: 0,
      strike: 0,
      spans: 0,
      bytes: 0,
      max_bytes: source_bytes.saturating_mul(4).saturating_add(4096),
    }
  }

  fn within_limits(&self) -> bool {
    self.depth <= MAX_DEPTH
      && self.document.blocks.len() < MAX_BLOCKS
      && self.spans <= MAX_SPANS
      && self.bytes <= self.max_bytes
      && self.table.as_ref().is_none_or(|table| table.cells <= MAX_TABLE_CELLS)
  }

  fn style(&self) -> Style {
    Style {
      bold: self.bold != 0,
      italic: self.italic != 0,
      strike: self.strike != 0,
      code: false,
      link: !self.links.is_empty(),
    }
  }

  fn block(&mut self, kind: BlockKind) -> Block {
    let marker = self.items.last_mut().and_then(Option::take);
    // A later paragraph/code block in an item hangs beneath its text rather
    // than returning to the list marker's outer edge.
    let continuation = usize::from(marker.is_none() && !self.lists.is_empty());
    Block {
      indent: (self.lists.len().saturating_sub(1) + continuation).min(MAX_DEPTH) as u8,
      quote_depth: self.quote_depth.min(MAX_DEPTH) as u8,
      marker,
      kind,
    }
  }

  fn flush(&mut self) {
    if let Some(block) = self.pending.take() {
      self.document.blocks.push(block);
    }
  }

  fn begin(&mut self, kind: BlockKind) {
    self.flush();
    let block = self.block(kind);
    self.pending = Some(block);
  }

  fn text(&mut self, text: &str, style: Style) {
    if text.is_empty() {
      return;
    }
    self.bytes = self.bytes.saturating_add(text.len());
    for link in &mut self.links {
      link.label.push_str(text);
    }
    if let Some(Block { kind: BlockKind::Code { text: code, .. }, .. }) = &mut self.pending {
      code.push_str(text);
      return;
    }
    if self.table.is_none() && self.pending.is_none() {
      self.begin(BlockKind::Paragraph(Vec::new()));
    }
    let spans = if let Some(table) = &mut self.table {
      &mut table.cell
    } else {
      match &mut self.pending.as_mut().expect("paragraph created above").kind {
        BlockKind::Paragraph(spans) | BlockKind::Heading { spans, .. } => spans,
        _ => return,
      }
    };
    if let Some(last) = spans.last_mut().filter(|last| last.style == style) {
      last.text.push_str(text);
    } else {
      spans.push(Span { text: text.into(), style });
      self.spans += 1;
    }
  }

  fn event(&mut self, event: Event<'_>) -> bool {
    match event {
      Event::Start(tag) => {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
          return false;
        }
        match tag {
          Tag::Paragraph | Tag::HtmlBlock => self.begin(BlockKind::Paragraph(Vec::new())),
          Tag::Heading { level, .. } => {
            self.begin(BlockKind::Heading { level: level as u8, spans: Vec::new() });
          }
          Tag::CodeBlock(kind) => {
            let language = match kind {
              CodeBlockKind::Fenced(info) => {
                info.split_whitespace().next().unwrap_or("").to_string()
              }
              CodeBlockKind::Indented => String::new(),
            };
            self.begin(BlockKind::Code { language, text: String::new() });
          }
          Tag::BlockQuote(_) => {
            self.flush();
            self.quote_depth += 1;
          }
          Tag::List(start) => {
            if self.items.last().is_some_and(Option::is_some) {
              self.begin(BlockKind::Paragraph(Vec::new()));
            }
            self.flush();
            self.lists.push(start);
          }
          Tag::Item => {
            self.flush();
            let marker = match self.lists.last_mut() {
              Some(Some(number)) => {
                let marker = format!("{number}.");
                *number = number.saturating_add(1);
                marker
              }
              _ => "•".into(),
            };
            self.items.push(Some(marker));
          }
          Tag::Strong => self.bold += 1,
          Tag::Emphasis => self.italic += 1,
          Tag::Strikethrough => self.strike += 1,
          Tag::Link { dest_url, link_type, .. } => {
            self.links.push(Link {
              destination: dest_url.into_string(),
              label: String::new(),
              image: false,
              autolink: matches!(link_type, LinkType::Autolink | LinkType::Email),
            });
          }
          Tag::Image { dest_url, .. } => {
            self.text("[Image: ", self.style());
            self.links.push(Link {
              destination: dest_url.into_string(),
              label: String::new(),
              image: true,
              autolink: false,
            });
          }
          Tag::Table(columns) => {
            if columns.len() > MAX_TABLE_COLUMNS {
              return false;
            }
            self.flush();
            let alignments = columns
              .into_iter()
              .map(|alignment| match alignment {
                pulldown_cmark::Alignment::Center => Alignment::Center,
                pulldown_cmark::Alignment::Right => Alignment::Right,
                _ => Alignment::Left,
              })
              .collect();
            let block =
              self.block(BlockKind::Table { headers: Vec::new(), rows: Vec::new(), alignments });
            self.table = Some(Table { block, row: Vec::new(), cell: Vec::new(), cells: 0 });
          }
          Tag::TableHead | Tag::TableRow | Tag::TableCell => {}
          // These extensions are deliberately disabled. Keeping the fallback
          // here also makes future parser option changes preserve source text.
          _ => return false,
        }
      }
      Event::End(tag) => {
        self.depth = self.depth.saturating_sub(1);
        match tag {
          TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock => {
            self.flush();
          }
          TagEnd::BlockQuote(_) => {
            self.flush();
            self.quote_depth = self.quote_depth.saturating_sub(1);
          }
          TagEnd::List(_) => {
            self.flush();
            self.lists.pop();
          }
          TagEnd::Item => {
            if self.items.last().is_some_and(Option::is_some) {
              self.begin(BlockKind::Paragraph(Vec::new()));
            }
            self.flush();
            self.items.pop();
          }
          TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
          TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
          TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
          TagEnd::Link | TagEnd::Image => {
            if let Some(link) = self.links.pop() {
              let style = Style { link: true, ..self.style() };
              if link.image {
                if link.label.is_empty() {
                  self.text("image", style);
                }
                self.text("]", style);
              }
              if !link.destination.is_empty()
                && (link.image || (!link.autolink && link.label.trim() != link.destination))
              {
                self.text(&format!(" ({})", link.destination), style);
              }
            }
          }
          TagEnd::TableCell => {
            if let Some(table) = &mut self.table {
              table.row.push(core::mem::take(&mut table.cell));
              table.cells += 1;
            }
          }
          TagEnd::TableHead | TagEnd::TableRow => {
            if let Some(table) = &mut self.table {
              if let BlockKind::Table { headers, rows, .. } = &mut table.block.kind {
                let row = core::mem::take(&mut table.row);
                if tag == TagEnd::TableHead {
                  *headers = row;
                } else {
                  rows.push(row);
                }
              }
            }
          }
          TagEnd::Table => {
            if let Some(table) = self.table.take() {
              self.document.blocks.push(table.block);
            }
          }
          _ => return false,
        }
      }
      Event::Text(text) => self.text(&text, self.style()),
      Event::Code(text) => self.text(&text, Style { code: true, ..self.style() }),
      Event::Html(text) | Event::InlineHtml(text) => {
        self.text(&text, Style { code: true, ..self.style() });
      }
      Event::SoftBreak | Event::HardBreak => self.text("\n", self.style()),
      Event::Rule => {
        self.begin(BlockKind::Rule);
        self.flush();
      }
      Event::TaskListMarker(checked) => {
        let checkbox = if checked { "[x]" } else { "[ ]" };
        if let Some(block) = &mut self.pending {
          block.marker = Some(format!("{} {checkbox}", block.marker.as_deref().unwrap_or("•")));
        } else if let Some(marker) = self.items.last_mut() {
          *marker = Some(format!("{} {checkbox}", marker.as_deref().unwrap_or("•")));
        }
      }
      _ => return false,
    }
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn paragraph(block: &Block) -> &[Span] {
    match &block.kind {
      BlockKind::Paragraph(spans) => spans,
      _ => panic!("expected paragraph"),
    }
  }

  fn text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
  }

  #[test]
  fn headings_inline_styles_entities_and_cjk_are_preserved() {
    let document = parse("# 中文 &amp; Markdown\n\n**粗体** *斜体* ~~删除~~ `let x = 1;`");
    assert!(matches!(&document.blocks[0].kind,
      BlockKind::Heading { level: 1, spans } if text(spans) == "中文 & Markdown"));
    let spans = paragraph(&document.blocks[1]);
    assert!(spans.iter().any(|span| span.text == "粗体" && span.style.bold));
    assert!(spans.iter().any(|span| span.text == "斜体" && span.style.italic));
    assert!(spans.iter().any(|span| span.text == "删除" && span.style.strike));
    assert!(spans.iter().any(|span| span.text == "let x = 1;" && span.style.code));
    let cjk = parse("中文**「强调」**测试");
    assert!(paragraph(&cjk.blocks[0])
      .iter()
      .any(|span| span.text == "「强调」" && span.style.bold));
  }

  #[test]
  fn nested_inline_styles_restore_the_outer_style() {
    let document = parse("**outer *inner* outer** plain");
    let spans = paragraph(&document.blocks[0]);
    assert!(spans.iter().any(|span| span.text == "inner" && span.style.bold && span.style.italic));
    assert!(spans
      .iter()
      .any(|span| span.text == " outer" && span.style.bold && !span.style.italic));
    assert_eq!(spans.last().unwrap().style, Style::default());
  }

  #[test]
  fn ordered_nested_and_task_lists_keep_markers_and_continuations() {
    let document = parse("3. three\n   - [x] done\n   - [ ] pending\n4. four\n\n   continued");
    assert_eq!(
      document.blocks.iter().map(|b| b.marker.as_deref()).collect::<Vec<_>>(),
      vec![Some("3."), Some("• [x]"), Some("• [ ]"), Some("4."), None]
    );
    assert_eq!(document.blocks.iter().map(|b| b.indent).collect::<Vec<_>>(), vec![0, 1, 1, 0, 1]);
    assert_eq!(text(paragraph(&document.blocks[4])), "continued");
    let empty_parent = parse("-\n  - child");
    assert_eq!(empty_parent.blocks.len(), 2);
    assert!(paragraph(&empty_parent.blocks[0]).is_empty());
    assert_eq!(empty_parent.blocks[0].marker.as_deref(), Some("•"));
    assert_eq!(empty_parent.blocks[1].indent, 1);
  }

  #[test]
  fn quotes_rules_and_multiline_text_keep_their_structure() {
    let document = parse("> quoted\n> line\n>\n> > nested\n\n---\n\n你好\n世界  \nagain");
    assert_eq!(document.blocks[0].quote_depth, 1);
    assert_eq!(text(paragraph(&document.blocks[0])), "quoted\nline");
    assert_eq!(document.blocks[1].quote_depth, 2);
    assert!(matches!(document.blocks[2].kind, BlockKind::Rule));
    assert_eq!(text(paragraph(&document.blocks[3])), "你好\n世界\nagain");
  }

  #[test]
  fn code_is_literal_and_unclosed_fences_render_during_streaming() {
    let source = "```rust\n  let 中文 = \"**literal**\";\n\n";
    let open = parse(source);
    let closed = parse(&format!("{source}```\n\nafter"));
    assert_eq!(open.blocks[0], closed.blocks[0]);
    assert!(matches!(&open.blocks[0].kind, BlockKind::Code { language, text }
      if language == "rust" && text == "  let 中文 = \"**literal**\";\n\n"));
    assert_eq!(text(paragraph(&closed.blocks[1])), "after");
  }

  #[test]
  fn incomplete_inline_syntax_and_escapes_remain_readable() {
    for source in ["**half", "[link](https://example.", "`half", "![图像"] {
      assert_eq!(text(paragraph(&parse(source).blocks[0])), source);
    }
    assert_eq!(
      text(paragraph(&parse(r"中文 \*literal\* foo_bar_baz").blocks[0])),
      "中文 *literal* foo_bar_baz"
    );
  }

  #[test]
  fn links_references_images_and_html_are_only_readable_text() {
    let document = parse("[site][s] <https://example.org> ![图](https://example.org/a.png)\n\n[s]: https://example.org\n\n<script>alert(1)</script>");
    assert_eq!(
      text(paragraph(&document.blocks[0])),
      "site (https://example.org) https://example.org [Image: 图] (https://example.org/a.png)"
    );
    assert_eq!(text(paragraph(&document.blocks[1])), "<script>alert(1)</script>\n");
    assert_eq!(
      text(paragraph(&parse("[run](javascript:alert(1))").blocks[0])),
      "run (javascript:alert(1))"
    );
  }

  #[test]
  fn table_cells_keep_inline_styles_alignment_and_empty_values() {
    let document = parse("| Name | Value |\n| :--- | ---: |\n| **中文** | `42` |\n| empty | |");
    let BlockKind::Table { headers, rows, alignments } = &document.blocks[0].kind else {
      panic!("expected table");
    };
    assert_eq!(text(&headers[0]), "Name");
    assert_eq!(alignments, &[Alignment::Left, Alignment::Right]);
    assert_eq!(rows.len(), 2);
    assert!(rows[0][0][0].style.bold);
    assert!(rows[0][1][0].style.code);
    assert!(rows[1][1].is_empty());
  }

  #[test]
  fn excessive_structure_falls_back_without_losing_any_source_text() {
    let sources = [
      format!("{}deep", "> ".repeat(MAX_DEPTH + 1)),
      "*a* ".repeat(MAX_SPANS),
      "a\n\n".repeat(MAX_BLOCKS + 1),
      format!("|{}|\n|{}|", "x|".repeat(64), "---|".repeat(64)),
      "中".repeat(MAX_INPUT_BYTES / 3 + 1),
    ];
    for source in sources {
      let document = parse(&source);
      assert_eq!(document.blocks.len(), 1);
      assert_eq!(text(paragraph(&document.blocks[0])), source);
    }
  }

  #[test]
  fn delimiter_heavy_input_and_repeated_reference_expansion_are_bounded() {
    let delimiters = "[".repeat(128 * 1024);
    assert_eq!(text(paragraph(&parse(&delimiters).blocks[0])), delimiters);
    let source =
      format!("{}\n\n[s]: https://example.org/{}", "[s] ".repeat(4096), "x".repeat(4096));
    let document = parse(&source);
    assert_eq!(document.blocks.len(), 1);
    assert_eq!(text(paragraph(&document.blocks[0])), source);
  }
}
