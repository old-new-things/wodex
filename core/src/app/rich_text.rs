//! Cached Markdown layout. Only visible lines create native textures.
use super::canvas::{Canvas, Color, FontStyle, Rect, ACCENT, BACKGROUND, BORDER, ELEVATED, MUTED};
use super::markdown::{self, Alignment, BlockKind, Span, Style};
use alloc::{rc::Rc, string::String, vec::Vec};
use unicode_width::UnicodeWidthChar;

const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 256;
const MAX_LAYOUT_LINES: usize = 8192;
const MAX_LAYOUT_RUNS: usize = 32768;
const MAX_LAYOUT_IMAGES: usize = 128;

#[derive(Default)]
pub struct RichTextCache {
  entries: Vec<Entry>,
  bytes: usize,
  clock: u64,
}

struct Entry {
  source: String,
  size: i32,
  width: i32,
  layout: Rc<Layout>,
  bytes: usize,
  used: u64,
}

impl RichTextCache {
  pub fn prepare(
    &mut self,
    canvas: &mut Canvas,
    source: &str,
    size: i32,
    width: i32,
  ) -> Rc<Layout> {
    self.clock = self.clock.saturating_add(1);
    if let Some(entry) = self
      .entries
      .iter_mut()
      .find(|entry| entry.size == size && entry.width == width && entry.source == source)
    {
      entry.used = self.clock;
      return Rc::clone(&entry.layout);
    }
    let layout = Rc::new(Layout::build(canvas, source, size, width));
    let source = String::from(source);
    let bytes = source.capacity() + layout.bytes() + core::mem::size_of::<Entry>();
    if bytes <= CACHE_BYTES {
      while self.bytes.saturating_add(bytes) > CACHE_BYTES || self.entries.len() >= CACHE_ENTRIES {
        let oldest = self.entries.iter().enumerate().min_by_key(|(_, entry)| entry.used);
        let Some((index, _)) = oldest else { break };
        self.bytes -= self.entries.swap_remove(index).bytes;
      }
      self.bytes += bytes;
      self.entries.push(Entry {
        source,
        size,
        width,
        layout: Rc::clone(&layout),
        bytes,
        used: self.clock,
      });
    }
    layout
  }
}

struct Run {
  text: String,
  x: i32,
  width: i32,
  size: i32,
  style: Style,
  // Zero for proportional text; otherwise the width of a monospace cell.
  cell: i32,
}

struct Line {
  y: i32,
  height: i32,
  runs: Vec<Run>,
}

struct Decoration {
  area: Rect,
  color: Color,
}

struct ImageCard {
  area: Rect,
  url: String,
  alt: String,
}

pub struct Layout {
  pub height: i32,
  width: i32,
  max_line_height: i32,
  run_count: usize,
  overflow: bool,
  plain: Option<(String, i32)>,
  lines: Vec<Line>,
  decorations: Vec<Decoration>,
  images: Vec<ImageCard>,
}

impl Layout {
  fn build(canvas: &mut Canvas, source: &str, size: i32, width: i32) -> Self {
    let width = width.max(1);
    if source.lines().take(MAX_LAYOUT_LINES + 1).count() > MAX_LAYOUT_LINES {
      return Self::plain(canvas, source, size, width);
    }
    let mut result = Self {
      height: 0,
      width,
      max_line_height: 0,
      run_count: 0,
      overflow: false,
      plain: None,
      lines: Vec::new(),
      decorations: Vec::new(),
      images: Vec::new(),
    };
    let document = markdown::parse(source);
    for block in document.blocks {
      let top = result.height;
      // Bound indentation on the handheld display even for deeply nested input.
      let quotes = i32::from(block.quote_depth.min(4));
      let indent = (i32::from(block.indent.min(6)) * 22 + quotes * 16).min(width / 3);
      let marker_width = block.marker.as_deref().map_or(0, |marker| {
        canvas.styled_text_width(marker, size, FontStyle::Normal).saturating_add(10).min(width / 3)
      });
      let x = indent.saturating_add(marker_width);
      let available = (width - x).max(1);
      if let Some(marker) = block.marker {
        result.push_line(Line {
          y: top,
          height: canvas.line_height(size, FontStyle::Normal),
          runs: alloc::vec![Run {
            width: marker_width,
            text: marker,
            x: indent,
            size,
            style: Style::default(),
            cell: 0,
          }],
        });
      }
      let height = match block.kind {
        BlockKind::Paragraph(spans) => result.flow(canvas, &spans, x, top, available, size),
        BlockKind::Heading { level, spans } => {
          let heading_size = size
            + match level {
              1 => 10,
              2 => 7,
              3 => 4,
              _ => 2,
            };
          let spans: Vec<_> = spans
            .into_iter()
            .map(|span| Span { style: Style { bold: true, ..span.style }, ..span })
            .collect();
          result.flow(canvas, &spans, x, top, available, heading_size)
        }
        BlockKind::Code { language, text } => {
          let code_size = (size - 3).max(12);
          let label_height = if language.is_empty() { 0 } else { 24 };
          let content_top = top + 10 + label_height;
          if !language.is_empty() {
            let label = super::canvas::preview(&language, 32);
            result.flow(
              canvas,
              &[Span { text: label, style: Style { bold: true, ..Style::default() }, image: None }],
              x + 12,
              top + 7,
              (available - 24).max(1),
              13,
            );
          }
          let content_height =
            result.code(canvas, &text, x + 12, content_top, (available - 24).max(1), code_size);
          let height = content_height + 20 + label_height;
          result.decorate(Rect { x, y: top, w: available, h: height }, BACKGROUND);
          height
        }
        BlockKind::Rule => {
          result.decorate(Rect { x, y: top + 9, w: available, h: 1 }, BORDER);
          20
        }
        BlockKind::Table { headers, rows, alignments } => {
          result.table(canvas, &headers, &rows, &alignments, x, top, available, size)
        }
      };
      if result.overflow {
        return Self::plain(canvas, source, size, width);
      }
      for quote in 0..quotes {
        result.decorate(Rect { x: quote * 16, y: top, w: 3, h: height }, BORDER);
      }
      result.height = result.height.saturating_add(height).saturating_add(10);
    }
    result.height =
      result.height.saturating_sub(10).max(canvas.line_height(size, FontStyle::Normal));
    // Table cells and list markers append independent lines; sort for viewport lookup.
    result.lines.sort_by_key(|line| line.y);
    result.max_line_height = result.lines.iter().map(|line| line.height).max().unwrap_or(0);
    result
  }

  fn plain(canvas: &mut Canvas, source: &str, size: i32, width: i32) -> Self {
    Self {
      height: canvas.paragraph(source, 0, 0, size, MUTED, width, false),
      width,
      max_line_height: 0,
      run_count: 0,
      overflow: false,
      plain: Some((source.into(), size)),
      lines: Vec::new(),
      decorations: Vec::new(),
      images: Vec::new(),
    }
  }

  fn bytes(&self) -> usize {
    core::mem::size_of::<Self>()
      + self.plain.as_ref().map_or(0, |(source, _)| source.capacity())
      + self.lines.capacity() * core::mem::size_of::<Line>()
      + self.decorations.capacity() * core::mem::size_of::<Decoration>()
      + self.images.capacity() * core::mem::size_of::<ImageCard>()
      + self.images.iter().map(|image| image.url.capacity() + image.alt.capacity()).sum::<usize>()
      + self
        .lines
        .iter()
        .map(|line| {
          line.runs.capacity() * core::mem::size_of::<Run>()
            + line.runs.iter().map(|run| run.text.capacity()).sum::<usize>()
        })
        .sum::<usize>()
  }

  fn decorate(&mut self, area: Rect, color: Color) {
    self.decorations.push(Decoration { area, color });
  }

  fn push_line(&mut self, line: Line) {
    if self.overflow
      || self.lines.len() >= MAX_LAYOUT_LINES
      || self.run_count.saturating_add(line.runs.len()) > MAX_LAYOUT_RUNS
    {
      self.overflow = true;
      return;
    }
    self.run_count += line.runs.len();
    self.lines.push(line);
  }

  #[allow(clippy::too_many_arguments)]
  fn flow(
    &mut self,
    canvas: &mut Canvas,
    spans: &[Span],
    x: i32,
    y: i32,
    width: i32,
    size: i32,
  ) -> i32 {
    if self.overflow {
      return 0;
    }
    let normal_height = canvas.line_height(size, FontStyle::Normal);
    let mut line = Line { y, height: normal_height, runs: Vec::new() };
    let mut used = 0;
    let mut has_image = false;
    for span in spans {
      if let Some(image) = &span.image {
        if self.images.len() >= MAX_LAYOUT_IMAGES {
          self.overflow = true;
          return 0;
        }
        if !line.runs.is_empty() {
          let next = line.y.saturating_add(line.height);
          self.push_line(line);
          line = Line { y: next, height: normal_height, runs: Vec::new() };
        }
        let height = width.saturating_mul(3).saturating_div(4).clamp(72, 240) + 38;
        self.images.push(ImageCard {
          area: Rect { x, y: line.y.saturating_add(4), w: width, h: height },
          url: image.url.clone(),
          alt: super::canvas::preview(&image.alt, 256),
        });
        line.y = line.y.saturating_add(height).saturating_add(12);
        used = 0;
        has_image = true;
        continue;
      }
      let style = span.style;
      let font = font_style(style);
      let run_size = if style.code { (size - 2).max(12) } else { size };
      let padding = if style.code { 6 } else { 0 };
      let cell = if style.code { canvas.monospace_cell_width(run_size) } else { 0 };
      let normalized =
        (style.code && span.text.contains('\t')).then(|| span.text.replace('\t', " "));
      let mut rest = normalized.as_deref().unwrap_or(&span.text);
      while !rest.is_empty() {
        if self.overflow
          || self.lines.len() >= MAX_LAYOUT_LINES
          || self.run_count >= MAX_LAYOUT_RUNS
        {
          self.overflow = true;
          return 0;
        }
        if rest.starts_with('\n') {
          let next = line.y.saturating_add(line.height);
          self.push_line(line);
          line = Line { y: next, height: normal_height, runs: Vec::new() };
          used = 0;
          rest = &rest[1..];
          continue;
        }
        let mut end = rest.len().min(512);
        while !rest.is_char_boundary(end) {
          end -= 1;
        }
        if let Some(newline) = rest[..end].find('\n') {
          end = newline;
        }
        let candidate = &rest[..end];
        let available = (width - used - padding).max(1);
        let mut length = if cell > 0 {
          code_prefix(candidate, available / cell)
        } else {
          canvas.measure_prefix(candidate, run_size, font, available)
        };
        let leading = candidate.len() - candidate.trim_start_matches(' ').len();
        // A style boundary is not a word boundary: move an ordinary word intact
        // when it fits a fresh line. CJK text remains breakable between glyphs.
        if used > 0
          && length < candidate.len()
          && length > leading
          && candidate[leading..]
            .chars()
            .next()
            .is_some_and(|ch| ch.width() == Some(1) && !ch.is_whitespace())
          && !candidate[leading..length].contains(char::is_whitespace)
          && !candidate[length..].starts_with(char::is_whitespace)
        {
          let word_end = leading
            + candidate[leading..].find(char::is_whitespace).unwrap_or(candidate.len() - leading);
          let word_width = if cell > 0 {
            code_width(&candidate[leading..word_end], cell)
          } else {
            canvas.styled_text_width(&candidate[leading..word_end], run_size, font)
          };
          if word_width + padding <= width {
            let next = line.y.saturating_add(line.height);
            self.push_line(line);
            line = Line { y: next, height: normal_height, runs: Vec::new() };
            used = 0;
            if !style.code {
              rest = rest.trim_start_matches(' ');
            }
            continue;
          }
        }
        if length < candidate.len() {
          if let Some(space) = candidate[..length].rfind(' ') {
            if space > 0 {
              length = space;
            }
          }
        }
        let text = &candidate[..length];
        let measured = if cell > 0 {
          code_width(text, cell)
        } else {
          canvas.styled_text_width(text, run_size, font)
        }
        .saturating_add(padding);
        if measured > width - used && used > 0 {
          let next = line.y.saturating_add(line.height);
          self.push_line(line);
          line = Line { y: next, height: normal_height, runs: Vec::new() };
          used = 0;
          // Preserve inline-code spaces; ordinary line-leading wrap spaces are dispensable.
          if !style.code {
            rest = rest.trim_start_matches(' ');
          }
          continue;
        }
        line.height = line.height.max(canvas.line_height(run_size, font));
        line.runs.push(Run {
          text: text.into(),
          x: x + used,
          width: measured,
          size: run_size,
          style,
          cell,
        });
        used = used.saturating_add(measured);
        rest = &rest[length..];
        if length < candidate.len() {
          let next = line.y.saturating_add(line.height);
          self.push_line(line);
          line = Line { y: next, height: normal_height, runs: Vec::new() };
          used = 0;
          if !style.code {
            rest = rest.trim_start_matches(' ');
          }
        }
      }
    }
    let height = line.y.saturating_sub(y).saturating_add(if has_image && line.runs.is_empty() {
      0
    } else {
      line.height
    });
    if !has_image || !line.runs.is_empty() {
      self.push_line(line);
    }
    height
  }

  fn code(
    &mut self,
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    size: i32,
  ) -> i32 {
    if self.overflow {
      return 0;
    }
    let cell = canvas.monospace_cell_width(size);
    let columns = (width / cell).max(1) as usize;
    let line_height = canvas.line_height(size, FontStyle::Normal);
    let mut line_y = y;
    let mut line = String::new();
    let mut column = 0;
    let flush = |layout: &mut Self, text: &mut String, line_y: &mut i32, column: &mut usize| {
      layout.push_line(Line {
        y: *line_y,
        height: line_height,
        runs: alloc::vec![Run {
          text: core::mem::take(text),
          x,
          width,
          size,
          style: Style::default(),
          cell,
        }],
      });
      *line_y = line_y.saturating_add(line_height);
      *column = 0;
    };
    // A final fence newline terminates the last code line without adding another.
    let text = text.strip_suffix('\n').unwrap_or(text);
    for character in text.chars() {
      if self.overflow || self.lines.len() >= MAX_LAYOUT_LINES {
        self.overflow = true;
        return 0;
      }
      if character == '\n' {
        flush(self, &mut line, &mut line_y, &mut column);
        continue;
      }
      if character == '\t' {
        let spaces = 4 - column % 4;
        for _ in 0..spaces {
          if column >= columns {
            flush(self, &mut line, &mut line_y, &mut column);
          }
          line.push(' ');
          column += 1;
        }
        continue;
      }
      let cells = character.width().unwrap_or(0);
      if cells > 0 && column > 0 && column.saturating_add(cells) > columns {
        flush(self, &mut line, &mut line_y, &mut column);
      }
      // Ignore control codes that have no visible representation, except combining marks.
      if !character.is_control() {
        line.push(character);
        column += cells;
      }
    }
    flush(self, &mut line, &mut line_y, &mut column);
    line_y.saturating_sub(y)
  }

  #[allow(clippy::too_many_arguments)]
  fn table(
    &mut self,
    canvas: &mut Canvas,
    headers: &[Vec<Span>],
    rows: &[Vec<Vec<Span>>],
    alignments: &[Alignment],
    x: i32,
    y: i32,
    width: i32,
    size: i32,
  ) -> i32 {
    let columns = headers.len().max(rows.iter().map(Vec::len).max().unwrap_or(0)).max(1);
    let table_size = (size - 2).max(12);
    let mut top = y;
    if columns > (width / 160).max(1) as usize {
      // On a narrow screen, retain every value using labeled rows instead of clipping columns.
      let labels: Vec<_> = headers.iter().map(|header| short_label(header, 96)).collect();
      // Preserve a long header once. Repeating an unbounded header for every
      // value could otherwise multiply a small source into a huge layout.
      let long_headers = headers.iter().any(|header| {
        header.iter().any(|span| span.image.is_some())
          || header.iter().map(|span| span.text.len()).sum::<usize>() > 96
      });
      if long_headers {
        for header in headers {
          top += self.flow(canvas, header, x + 10, top, (width - 20).max(1), table_size) + 6;
        }
      }
      for row in rows {
        if self.overflow {
          return 0;
        }
        let row_top = top;
        for (column, cell) in row.iter().enumerate() {
          let mut content = labels.get(column).cloned().unwrap_or_default();
          for span in &mut content {
            span.style.bold = true;
          }
          if !content.is_empty() {
            content.push(Span { text: ": ".into(), style: Style::default(), image: None });
          }
          content.extend_from_slice(cell);
          top += self.flow(canvas, &content, x + 10, top + 6, (width - 20).max(1), table_size) + 6;
        }
        self.decorate(Rect { x, y: row_top, w: width, h: top - row_top + 6 }, BACKGROUND);
        top += 14;
      }
      if rows.is_empty() && !long_headers {
        for header in headers {
          top += self.flow(canvas, header, x + 10, top, (width - 20).max(1), table_size);
        }
      }
      return (top - y).max(1);
    }
    let cell_width = (width / columns as i32).max(1);
    for (row_index, row) in
      core::iter::once(headers).chain(rows.iter().map(Vec::as_slice)).enumerate()
    {
      if self.overflow {
        return 0;
      }
      let mut row_height = 0;
      for column in 0..columns {
        let mut spans = row.get(column).cloned().unwrap_or_default();
        if row_index == 0 {
          for span in &mut spans {
            span.style.bold = true;
          }
        }
        let first_line = self.lines.len();
        let left = x + column as i32 * cell_width + 8;
        let inner_width = (cell_width - 16).max(1);
        row_height =
          row_height.max(self.flow(canvas, &spans, left, top + 7, inner_width, table_size) + 14);
        for line in &mut self.lines[first_line..] {
          let used = line.runs.last().map_or(0, |run| run.x + run.width - left);
          let extra = (inner_width - used).max(0);
          let offset = match alignments.get(column).unwrap_or(&Alignment::Left) {
            Alignment::Left => 0,
            Alignment::Center => extra / 2,
            Alignment::Right => extra,
          };
          for run in &mut line.runs {
            run.x += offset;
          }
        }
      }
      self.decorate(
        Rect { x, y: top, w: width, h: row_height },
        if row_index == 0 { ELEVATED } else { BACKGROUND },
      );
      self.decorate(Rect { x, y: top + row_height - 1, w: width, h: 1 }, BORDER);
      for column in 1..columns {
        self.decorate(
          Rect { x: x + column as i32 * cell_width, y: top, w: 1, h: row_height },
          BORDER,
        );
      }
      top = top.saturating_add(row_height);
    }
    top.saturating_sub(y)
  }

  pub fn draw(&self, canvas: &mut Canvas, x: i32, y: i32, color: Color) {
    let outer = canvas.clip_area();
    if outer.w <= 0 || outer.h <= 0 {
      return;
    }
    let left = outer.x.max(x);
    let right = outer.x.saturating_add(outer.w).min(x.saturating_add(self.width));
    canvas.clip(Rect { x: left, w: (right - left).max(0), ..outer });
    if let Some((source, size)) = &self.plain {
      canvas.paragraph(source, x, y, *size, color, self.width, true);
      canvas.clip(outer);
      return;
    }
    for decoration in &self.decorations {
      canvas.rect(
        Rect {
          x: x.saturating_add(decoration.area.x),
          y: y.saturating_add(decoration.area.y),
          ..decoration.area
        },
        decoration.color,
        0,
      );
    }
    let clip = canvas.clip_area();
    let bottom = i64::from(clip.y) + i64::from(clip.h);
    let first = self.lines.partition_point(|line| {
      i64::from(y) + i64::from(line.y) + i64::from(self.max_line_height) < i64::from(clip.y)
    });
    for line in &self.lines[first..] {
      let line_y = i64::from(y) + i64::from(line.y);
      if line_y >= bottom {
        break;
      }
      if line_y + i64::from(line.height) < i64::from(clip.y) {
        continue;
      }
      let line_y = line_y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
      let baseline = line
        .runs
        .iter()
        .map(|run| canvas.font_ascent(run.size, font_style(run.style)))
        .max()
        .unwrap_or(0);
      for run in &line.runs {
        let run_x = x.saturating_add(run.x);
        let run_y =
          line_y.saturating_add(baseline - canvas.font_ascent(run.size, font_style(run.style)));
        if run.style.code {
          canvas.rect(
            Rect { x: run_x, y: line_y, w: run.width, h: line.height - 2 },
            BACKGROUND,
            3,
          );
        }
        if run.cell > 0 {
          draw_monospace(canvas, run, run_x + if run.style.code { 3 } else { 0 }, run_y, color);
          continue;
        }
        let run_color = if run.style.link { ACCENT } else { color };
        canvas.styled_text(
          &run.text,
          run_x + if run.style.code { 3 } else { 0 },
          run_y,
          run.size,
          run_color,
          0,
          font_style(run.style),
        );
        if run.style.strike {
          canvas.rect(
            Rect { x: run_x, y: line_y + line.height / 2, w: run.width, h: 1 },
            run_color,
            0,
          );
        }
        if run.style.link {
          canvas.rect(Rect { x: run_x, y: line_y + line.height - 4, w: run.width, h: 1 }, MUTED, 0);
        }
      }
    }
    for image in &self.images {
      canvas.draw_image(
        &image.url,
        &image.alt,
        Rect { x: x.saturating_add(image.area.x), y: y.saturating_add(image.area.y), ..image.area },
      );
    }
    canvas.clip(outer);
  }
}

fn font_style(style: Style) -> FontStyle {
  match (style.bold, style.italic) {
    (false, false) => FontStyle::Normal,
    (true, false) => FontStyle::Bold,
    (false, true) => FontStyle::Italic,
    (true, true) => FontStyle::BoldItalic,
  }
}

fn short_label(spans: &[Span], limit: usize) -> Vec<Span> {
  let mut remaining = limit;
  let mut result = Vec::new();
  for span in spans {
    // Stacked tables repeat labels per row; an image belongs in the header once.
    let text = span.image.as_ref().map_or(span.text.as_str(), |image| image.alt.as_str());
    if text.len() <= remaining {
      result.push(Span { text: text.into(), style: span.style, image: None });
      remaining -= text.len();
    } else {
      result.push(Span {
        text: super::canvas::preview(text, remaining),
        style: span.style,
        image: None,
      });
      break;
    }
  }
  result
}

fn draw_monospace(canvas: &mut Canvas, run: &Run, x: i32, y: i32, color: Color) {
  let mut cluster_start = 0;
  let mut column = 0;
  let mut cluster_cells = 0;
  for (offset, character) in run.text.char_indices() {
    let cells = character.width().unwrap_or(0);
    if cells > 0 && offset > cluster_start {
      let cluster = &run.text[cluster_start..offset];
      if !cluster.chars().all(char::is_whitespace) {
        draw_code_cluster(canvas, cluster, x + column * run.cell, y, run.size, color);
      }
      column += cluster_cells;
      cluster_start = offset;
    }
    if cells > 0 {
      cluster_cells = cells as i32;
    }
  }
  let cluster = &run.text[cluster_start..];
  if !cluster.is_empty() && !cluster.chars().all(char::is_whitespace) {
    draw_code_cluster(canvas, cluster, x + column * run.cell, y, run.size, color);
  }
}

fn draw_code_cluster(canvas: &mut Canvas, cluster: &str, x: i32, y: i32, size: i32, color: Color) {
  let font = if cluster.starts_with(|ch: char| ch.is_ascii()) {
    FontStyle::Monospace
  } else {
    FontStyle::Normal
  };
  let offset = canvas.font_ascent(size, FontStyle::Normal) - canvas.font_ascent(size, font);
  canvas.styled_text(cluster, x, y.saturating_add(offset), size, color, 0, font);
}

fn code_width(text: &str, cell: i32) -> i32 {
  text.chars().map(|ch| ch.width().unwrap_or(0) as i32).sum::<i32>().saturating_mul(cell)
}

fn code_prefix(text: &str, columns: i32) -> usize {
  let mut used = 0;
  let mut end = 0;
  for (offset, character) in text.char_indices() {
    let cells = character.width().unwrap_or(0) as i32;
    if used + cells > columns && end > 0 {
      break;
    }
    used += cells;
    end = offset + character.len_utf8();
  }
  end
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::app::canvas::{test_backend::canvas, INK};
  use alloc::format;

  #[test]
  fn inline_images_keep_text_order_and_only_visible_cards_request_pixels() {
    let (_backend, mut canvas) = canvas();
    let source = "before ![A cat 中文](https://example.test/cat.png) after\n\n\
      ![Second](https://example.test/second.png)";
    let layout = Layout::build(&mut canvas, source, 20, 300);
    assert_eq!(layout.images.len(), 2);
    assert!(canvas.take_image_requests().is_empty());
    let before = layout
      .lines
      .iter()
      .find(|line| line.runs.iter().any(|run| run.text.contains("before")))
      .unwrap();
    let after = layout
      .lines
      .iter()
      .find(|line| line.runs.iter().any(|run| run.text.contains("after")))
      .unwrap();
    assert!(before.y + before.height <= layout.images[0].area.y);
    assert!(after.y >= layout.images[0].area.y + layout.images[0].area.h);
    assert!(layout.images[1].area.y > after.y);
    canvas.clip(Rect { x: 0, y: 0, w: 300, h: 100 });
    canvas.begin_image_frame();
    layout.draw(&mut canvas, 0, 0, INK);
    assert_eq!(canvas.take_image_requests(), ["https://example.test/cat.png"]);
    canvas.clip(Rect { x: 0, y: layout.images[1].area.y, w: 300, h: 100 });
    canvas.begin_image_frame();
    layout.draw(&mut canvas, 0, 0, INK);
    assert_eq!(canvas.take_image_requests(), ["https://example.test/second.png"]);
    assert_eq!(layout.images[0].alt, "A cat 中文");
  }

  #[test]
  fn images_fit_table_cells_and_stacked_headers_are_rendered_once() {
    let (_backend, mut canvas) = canvas();
    let source = "| ![Header](https://example.test/header.png) | Other |\n\
      | --- | --- |\n\
      | ![Body](https://example.test/body.png) | words |\n\
      | next | value |";
    let wide = Layout::build(&mut canvas, source, 20, 500);
    assert_eq!(wide.images.len(), 2);
    assert!(wide.images.iter().all(|image| image.area.w == 234));
    assert!(wide.images[1].area.y >= wide.images[0].area.y + wide.images[0].area.h);
    let narrow = Layout::build(&mut canvas, source, 20, 220);
    assert_eq!(narrow.images.len(), 2);
    assert_eq!(narrow.images.iter().filter(|image| image.alt == "Header").count(), 1);
    assert!(narrow.images.iter().all(|image| image.area.w <= 200));
    assert!(narrow.height < 1500);
  }

  #[test]
  fn image_layout_limits_and_disabled_loading_remain_bounded() {
    let (_backend, mut canvas) = canvas();
    let source = "![cat](https://example.test/cat.png)\n\n".repeat(MAX_LAYOUT_IMAGES);
    let layout = Layout::build(&mut canvas, &source, 20, 1);
    assert_eq!(layout.images.len(), MAX_LAYOUT_IMAGES);
    assert!(layout.height < MAX_LAYOUT_IMAGES as i32 * 150);
    assert!(layout.bytes() < 128 * 1024);
    canvas.set_image_loading(false);
    canvas.begin_image_frame();
    layout.draw(&mut canvas, 0, 0, INK);
    assert!(canvas.take_image_requests().is_empty());
    let too_many =
      Layout::build(&mut canvas, &(source + "![extra](https://example.test/x.png)"), 20, 300);
    assert!(too_many.images.is_empty());
  }

  #[test]
  fn styles_and_cjk_wrap_inside_the_available_width() {
    let (backend, mut canvas) = canvas();
    let text = "# Heading\n\n中文 **bold words** and *italic* ~~old~~ `code`\n\n- list\n> quote";
    let layout = Layout::build(&mut canvas, text, 20, 110);
    assert!(layout.lines.iter().flat_map(|line| &line.runs).any(|run| run.style.bold));
    assert!(layout.lines.iter().flat_map(|line| &line.runs).any(|run| run.style.italic));
    assert!(layout.lines.iter().flat_map(|line| &line.runs).any(|run| run.style.strike));
    for line in &layout.lines {
      for run in &line.runs {
        assert!(run.x >= 0 && run.x + run.width <= 110, "{}", run.text);
      }
    }
    layout.draw(&mut canvas, 0, 0, INK);
    assert!(!backend.drawn_text.iter().any(|text| text.contains("**") || text.contains("~~")));
    assert!(canvas.take_error().is_none());
  }

  #[test]
  fn code_preserves_indentation_and_combining_marks_when_wrapping() {
    let (_backend, mut canvas) = canvas();
    let cell = canvas.monospace_cell_width(17);
    let layout =
      Layout::build(&mut canvas, "```rust\n\t中e\u{301}x\n    next\n```", 20, 8 * cell + 24);
    let code: Vec<_> = layout
      .lines
      .iter()
      .flat_map(|line| &line.runs)
      .filter(|run| run.cell > 0)
      .map(|run| run.text.as_str())
      .collect();
    assert_eq!(code, ["    中e\u{301}x", "    next"]);
    let narrow = Layout::build(&mut canvas, "```\n中文abc\n```", 20, 4 * cell + 24);
    let code: Vec<_> = narrow
      .lines
      .iter()
      .flat_map(|line| &line.runs)
      .filter(|run| run.cell > 0)
      .map(|run| run.text.as_str())
      .collect();
    assert_eq!(code, ["中文", "abc"]);
  }

  #[test]
  fn mixed_code_fonts_share_a_baseline_and_inline_tabs_remain_visible_spaces() {
    let (backend, mut canvas) = canvas();
    let layout = Layout::build(&mut canvas, "```\na中\n```", 20, 200);
    layout.draw(&mut canvas, 0, 0, INK);
    assert_eq!(backend.drawn_text, ["a", "中"]);
    assert_eq!(backend.copied_rects[0].y, backend.copied_rects[1].y + 4);
    let inline = Layout::build(&mut canvas, "`a\tb`", 20, 200);
    assert_eq!(inline.lines[0].runs[0].text, "a b");
    assert!(inline.lines[0].runs[0].cell > 0);
  }

  #[test]
  fn emphasis_boundaries_do_not_split_words_that_fit_a_new_line() {
    let (_backend, mut canvas) = canvas();
    let layout = Layout::build(&mut canvas, "Hello **world**", 20, 80);
    let text: Vec<String> = layout
      .lines
      .iter()
      .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect())
      .collect();
    assert_eq!(text, ["Hello ", "world"]);
    let layout = Layout::build(&mut canvas, "**Hello** world", 20, 80);
    let text: Vec<String> = layout
      .lines
      .iter()
      .map(|line| line.runs.iter().map(|run| run.text.as_str()).collect())
      .collect();
    assert_eq!(text, ["Hello", "world"]);
  }

  #[test]
  fn tables_preserve_all_cells_and_stack_wide_rows() {
    let (_backend, mut canvas) = canvas();
    let grid =
      Layout::build(&mut canvas, "| Name | Value |\n| :--- | ---: |\n| 中文 | **42** |", 20, 400);
    assert!(grid
      .lines
      .iter()
      .flat_map(|line| &line.runs)
      .any(|run| run.text == "42" && run.style.bold));
    assert!(grid.lines.iter().flat_map(|line| &line.runs).all(|run| run.x + run.width <= 400));
    let stacked =
      Layout::build(&mut canvas, "| A | B | C |\n|---|---|---|\n| one | two | three |", 20, 240);
    let text: String =
      stacked.lines.iter().flat_map(|line| &line.runs).map(|run| run.text.as_str()).collect();
    assert_eq!(text, "A: oneB: twoC: three");
    assert!(stacked.height > grid.height);
  }

  #[test]
  fn far_scrolled_markdown_draws_only_visible_tail_and_restores_clip() {
    let (mut backend, mut canvas) = canvas();
    let text = format!("{}**最后一行**", "中文\n".repeat(60_000));
    let layout = Layout::build(&mut canvas, &text, 20, 100);
    assert!(layout.plain.is_some());
    assert!(layout.lines.is_empty());
    let clip = Rect { x: 10, y: 100, w: 80, h: 52 };
    canvas.clip(clip);
    layout.draw(&mut canvas, 0, 126 - layout.height, INK);
    assert!(backend.drawn_text.iter().any(|text| text.contains("最后一行")));
    assert!(backend.drawn_text.len() <= 4);
    assert_eq!(canvas.clip_area(), clip);
    backend.drawn_text.clear();
    layout.draw(&mut canvas, 0, i32::MIN, INK);
    assert!(backend.drawn_text.is_empty());
  }

  #[test]
  fn oversized_layouts_use_compact_plain_text_and_repeated_headers_are_bounded() {
    let (_backend, mut canvas) = canvas();
    let mut cache = RichTextCache::default();
    let source = "a\n".repeat(256 * 1024);
    let layout = cache.prepare(&mut canvas, &source, 20, 800);
    assert!(layout.plain.is_some() && layout.lines.is_empty());
    assert!(Rc::ptr_eq(&layout, &cache.prepare(&mut canvas, &source, 20, 800)));
    let narrow = Layout::build(&mut canvas, &"a".repeat(32_000), 20, 1);
    assert!(narrow.plain.is_some());
    let table = format!(
      "| {} | B | C |\n|---|---|---|\n{}",
      "Header".repeat(1500),
      "| 1 | 2 | 3 |\n".repeat(200)
    );
    let layout = Layout::build(&mut canvas, &table, 20, 400);
    assert!(layout.plain.is_none());
    let displayed: usize =
      layout.lines.iter().flat_map(|line| &line.runs).map(|run| run.text.len()).sum();
    assert!(displayed < 100_000, "Repeated headers expanded to {displayed} bytes");
  }

  #[test]
  fn streaming_changes_replace_layout_without_mutating_the_source_and_cache_stays_bounded() {
    let (_backend, mut canvas) = canvas();
    let mut cache = RichTextCache::default();
    let partial = String::from("**answer**\n\n```rust\nlet n = 1;");
    let first = cache.prepare(&mut canvas, &partial, 20, 300);
    assert!(Rc::ptr_eq(&first, &cache.prepare(&mut canvas, &partial, 20, 300)));
    let complete = format!("{partial}\n```\n\nDone.");
    let second = cache.prepare(&mut canvas, &complete, 20, 300);
    assert!(!Rc::ptr_eq(&first, &second));
    assert!(second.height > first.height);
    assert!(second.lines.iter().flat_map(|line| &line.runs).any(|run| run.text == "Done."));
    assert_eq!(partial, "**answer**\n\n```rust\nlet n = 1;");
    for index in 0..300 {
      cache.prepare(&mut canvas, &format!("**stream {index}**"), 20, 300);
    }
    assert!(cache.entries.len() <= CACHE_ENTRIES);
    assert!(cache.bytes <= CACHE_BYTES);
  }
}
