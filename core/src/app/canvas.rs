use alloc::collections::BTreeMap;
use alloc::ffi::CString;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::{c_char, c_void, CStr};
use core::ops::Range;

const CACHE_BYTES: usize = 16 * 1024 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
  pub x: i32,
  pub y: i32,
  pub w: i32,
  pub h: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Color {
  pub r: u8,
  pub g: u8,
  pub b: u8,
  pub a: u8,
}

impl Color {
  pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
    Self { r, g, b, a: 255 }
  }
}

pub const BACKGROUND: Color = Color::rgb(18, 22, 28);
pub const PANEL: Color = Color::rgb(25, 30, 37);
pub const ELEVATED: Color = Color::rgb(34, 41, 49);
pub const INK: Color = Color::rgb(237, 241, 238);
pub const MUTED: Color = Color::rgb(143, 155, 164);
pub const ACCENT: Color = Color::rgb(187, 239, 129);
pub const DARK_INK: Color = Color::rgb(27, 40, 25);
pub const BORDER: Color = Color::rgb(49, 59, 66);
pub const DANGER: Color = Color::rgb(255, 167, 158);

/// Synchronous drawing primitives supplied by SDL/SDL_ttf on the native host.
///
/// The owner of this table must keep its renderer and platform alive until the
/// canvas is dropped. Every callback must accept its documented handles and
/// must neither retain borrowed text nor unwind across this boundary.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RenderApi {
  pub renderer: *mut c_void,
  pub platform: *const c_void,
  pub font_open: unsafe extern "C" fn(*const c_void, i32) -> *mut c_void,
  pub font_close: unsafe extern "C" fn(*mut c_void),
  pub font_line_skip: unsafe extern "C" fn(*mut c_void) -> i32,
  pub text_measure: unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut i32) -> i32,
  pub text_size: unsafe extern "C" fn(*mut c_void, *const c_char, *mut i32, *mut i32) -> i32,
  pub texture_create: unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *const c_char,
    Color,
    i32,
    *mut i32,
    *mut i32,
  ) -> *mut c_void,
  pub texture_destroy: unsafe extern "C" fn(*mut c_void),
  pub fill_rect: unsafe extern "C" fn(*mut c_void, Rect, Color) -> i32,
  pub outline_rect: unsafe extern "C" fn(*mut c_void, Rect, Color) -> i32,
  pub draw_line: unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32, Color) -> i32,
  pub copy_texture: unsafe extern "C" fn(*mut c_void, *mut c_void, Rect) -> i32,
  pub set_clip: unsafe extern "C" fn(*mut c_void, *const Rect) -> i32,
  pub error: unsafe extern "C" fn() -> *const c_char,
}

struct Font {
  handle: *mut c_void,
  close: unsafe extern "C" fn(*mut c_void),
}

impl Drop for Font {
  fn drop(&mut self) {
    unsafe { (self.close)(self.handle) };
  }
}

struct Texture {
  handle: *mut c_void,
  destroy: unsafe extern "C" fn(*mut c_void),
  width: i32,
  height: i32,
  bytes: usize,
  used: u64,
}

impl Drop for Texture {
  fn drop(&mut self) {
    unsafe { (self.destroy)(self.handle) };
  }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct TextureKey {
  size: i32,
  width: i32,
  color: Color,
  text: String,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct LayoutKey {
  size: i32,
  width: i32,
  text: String,
}

struct Layout {
  lines: Vec<Range<usize>>,
  line_height: i32,
}

impl Layout {
  fn bytes(&self, key: &LayoutKey) -> usize {
    key.text.capacity()
      + self.lines.capacity() * core::mem::size_of::<Range<usize>>()
      + core::mem::size_of::<Self>()
      + core::mem::size_of::<LayoutKey>()
  }

  fn height(&self) -> i32 {
    self.line_height.saturating_mul(i32::try_from(self.lines.len()).unwrap_or(i32::MAX))
  }
}

/// Rust owns font/texture lifetimes, bounded caches, UTF-8 layout, and drawing policy.
pub struct Canvas {
  api: RenderApi,
  // Textures are released before fonts, including when construction fails.
  textures: BTreeMap<TextureKey, Texture>,
  fonts: BTreeMap<i32, Font>,
  layouts: BTreeMap<LayoutKey, Layout>,
  texture_bytes: usize,
  texture_clock: u64,
  layout_bytes: usize,
  clip: Rect,
  error: Option<String>,
}

impl Canvas {
  /// # Safety
  /// Callbacks must satisfy RenderApi's contract. The renderer, platform, and
  /// callback code must remain valid until this canvas and all its handles drop.
  pub unsafe fn new(api: &RenderApi) -> Result<Self, String> {
    let mut result = Self {
      api: *api,
      textures: BTreeMap::new(),
      fonts: BTreeMap::new(),
      layouts: BTreeMap::new(),
      texture_bytes: 0,
      texture_clock: 0,
      layout_bytes: 0,
      clip: Rect { x: 0, y: 0, w: 1280, h: 720 },
      error: None,
    };
    result.font(20)?;
    Ok(result)
  }

  pub fn take_error(&mut self) -> Option<String> {
    self.error.take()
  }

  fn native_error(&self, context: &str) -> String {
    let error = unsafe { (self.api.error)() };
    if error.is_null() {
      return context.to_string();
    }
    let detail = unsafe { CStr::from_ptr(error) }.to_string_lossy();
    if detail.is_empty() {
      context.to_string()
    } else {
      format!("{context}: {detail}")
    }
  }

  fn record(&mut self, error: String) {
    if self.error.is_none() {
      self.error = Some(error);
    }
  }

  fn checked(&mut self, status: i32, context: &str) {
    if status != 0 {
      self.record(self.native_error(context));
    }
  }

  fn font(&mut self, size: i32) -> Result<*mut c_void, String> {
    if let Some(font) = self.fonts.get(&size) {
      return Ok(font.handle);
    }
    if !(1..=512).contains(&size) {
      return Err("Font size must be between 1 and 512 points.".to_string());
    }
    let handle = unsafe { (self.api.font_open)(self.api.platform, size) };
    if handle.is_null() {
      return Err(self.native_error("Cannot load font"));
    }
    self.fonts.insert(size, Font { handle, close: self.api.font_close });
    Ok(handle)
  }

  fn texture(&mut self, key: &TextureKey) -> Result<Texture, String> {
    let value = CString::new(key.text.as_str())
      .map_err(|_| "Text contains an unsupported NUL character.".to_string())?;
    let font = self.font(key.size)?;
    let (mut width, mut height) = (0, 0);
    let handle = unsafe {
      (self.api.texture_create)(
        self.api.renderer,
        font,
        value.as_ptr(),
        key.color,
        key.width,
        &mut width,
        &mut height,
      )
    };
    if handle.is_null() {
      return Err(self.native_error("Cannot render text"));
    }
    let texture = Texture {
      handle,
      destroy: self.api.texture_destroy,
      width,
      height,
      bytes: (width.max(0) as usize).saturating_mul(height.max(0) as usize).saturating_mul(4),
      used: self.texture_clock,
    };
    if width <= 0 || height <= 0 {
      return Err("The font renderer returned invalid text dimensions.".to_string());
    }
    Ok(texture)
  }

  pub fn text(&mut self, value: &str, x: i32, y: i32, size: i32, color: Color, width: i32) -> i32 {
    if value.is_empty() {
      return size.saturating_add(6);
    }
    self.texture_clock = self.texture_clock.saturating_add(1);
    let key = TextureKey { size, width, color, text: value.to_string() };
    let mut entry = if let Some(mut entry) = self.textures.remove(&key) {
      self.texture_bytes -= entry.bytes;
      entry.used = self.texture_clock;
      entry
    } else {
      match self.texture(&key) {
        Ok(entry) => entry,
        Err(error) => {
          self.record(error);
          return size.saturating_add(6);
        }
      }
    };
    let destination = Rect { x, y, w: entry.width, h: entry.height };
    let status = unsafe { (self.api.copy_texture)(self.api.renderer, entry.handle, destination) };
    self.checked(status, "Cannot draw text");
    let height = entry.height;
    // A single unusually large texture is transient; cached pixels remain bounded.
    if entry.bytes <= CACHE_BYTES {
      while self.texture_bytes + entry.bytes > CACHE_BYTES {
        let oldest =
          self.textures.iter().min_by_key(|(_, texture)| texture.used).map(|(key, _)| TextureKey {
            size: key.size,
            width: key.width,
            color: key.color,
            text: key.text.clone(),
          });
        let Some(oldest) = oldest else { break };
        if let Some(removed) = self.textures.remove(&oldest) {
          self.texture_bytes -= removed.bytes;
        }
      }
      entry.used = self.texture_clock;
      self.texture_bytes += entry.bytes;
      self.textures.insert(key, entry);
    }
    height
  }

  fn build_layout(&mut self, key: &LayoutKey) -> Result<Layout, String> {
    let font = self.font(key.size)?;
    let line_height = unsafe { (self.api.font_line_skip)(font) }.saturating_add(4).max(1);
    let mut result = Layout { lines: Vec::new(), line_height };
    let mut offset = 0;
    while offset < key.text.len() {
      if key.text.as_bytes()[offset] == b'\n' {
        result.lines.push(offset..offset);
        offset += 1;
        continue;
      }
      // Measure bounded strings without producing surfaces for offscreen history.
      let mut candidate_end = (offset + 512).min(key.text.len());
      while !key.text.is_char_boundary(candidate_end) {
        candidate_end -= 1;
      }
      if let Some(newline) = key.text[offset..candidate_end].find('\n') {
        candidate_end = offset + newline;
      }
      let candidate = &key.text[offset..candidate_end];
      let value = CString::new(candidate)
        .map_err(|_| "Text contains an unsupported NUL character.".to_string())?;
      let mut count = 0;
      if unsafe { (self.api.text_measure)(font, value.as_ptr(), key.width.max(1), &mut count) } != 0
      {
        return Err(self.native_error("Cannot measure text"));
      }
      let mut length = candidate
        .char_indices()
        .nth(count.max(1) as usize)
        .map_or(candidate.len(), |(index, _)| index);
      if length < candidate.len() {
        if let Some(space) = candidate[..length].rfind(' ') {
          if space > 0 {
            length = space;
          }
        }
      }
      result.lines.push(offset..offset + length);
      offset += length;
      if offset < key.text.len() && matches!(key.text.as_bytes()[offset], b' ' | b'\n') {
        offset += 1;
      }
    }
    if result.lines.is_empty() {
      result.lines.push(0..0);
    }
    Ok(result)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn paragraph(
    &mut self,
    value: &str,
    x: i32,
    y: i32,
    size: i32,
    color: Color,
    width: i32,
    draw: bool,
  ) -> i32 {
    let key = LayoutKey { size, width, text: value.to_string() };
    // Temporarily own the layout so drawing may update the texture cache without
    // cloning every line of a long paragraph.
    let layout = if let Some(layout) = self.layouts.remove(&key) {
      self.layout_bytes -= layout.bytes(&key);
      layout
    } else {
      match self.build_layout(&key) {
        Ok(layout) => layout,
        Err(error) => {
          self.record(error);
          return size.saturating_add(6);
        }
      }
    };
    if draw && self.clip.w > 0 && self.clip.h > 0 {
      let line_height = i64::from(layout.line_height);
      let clip_top = i64::from(self.clip.y);
      let clip_bottom = i64::from(self.clip.y.saturating_add(self.clip.h));
      // Include a line whose bottom touches the clip's top, matching the
      // existing visibility rule. Skip history above and below the viewport.
      let first = (clip_top - i64::from(y) - 1).div_euclid(line_height).max(0);
      let end = (clip_bottom - i64::from(y) + line_height - 1).div_euclid(line_height).max(0);
      let first = usize::try_from(first).unwrap_or(usize::MAX).min(layout.lines.len());
      let end = usize::try_from(end).unwrap_or(usize::MAX).min(layout.lines.len());
      let mut line_y = i64::from(y) + first as i64 * line_height;
      for range in &layout.lines[first..end.max(first)] {
        if !range.is_empty() {
          let draw_y = line_y.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
          self.text(&key.text[range.clone()], x, draw_y, size, color, 0);
        }
        line_y += line_height;
      }
    }
    let height = layout.height();
    let bytes = layout.bytes(&key);
    if bytes <= CACHE_BYTES {
      if self.layout_bytes + bytes > CACHE_BYTES {
        self.layouts.clear();
        self.layout_bytes = 0;
      }
      self.layout_bytes += bytes;
      self.layouts.insert(key, layout);
    }
    height
  }

  pub fn rect(&mut self, area: Rect, color: Color, radius: i32) {
    if area.w <= 0 || area.h <= 0 || self.clip.w <= 0 || self.clip.h <= 0 {
      return;
    }
    let area_right = i64::from(area.x) + i64::from(area.w);
    let area_bottom = i64::from(area.y) + i64::from(area.h);
    let clip_right = i64::from(self.clip.x) + i64::from(self.clip.w);
    let clip_bottom = i64::from(self.clip.y) + i64::from(self.clip.h);
    if area_right <= i64::from(self.clip.x)
      || i64::from(area.x) >= clip_right
      || area_bottom <= i64::from(self.clip.y)
      || i64::from(area.y) >= clip_bottom
    {
      return;
    }
    let radius = radius.max(0).min(area.w / 2).min(area.h / 2);
    if radius == 0 {
      let status = unsafe { (self.api.fill_rect)(self.api.renderer, area, color) };
      self.checked(status, "Cannot fill rectangle");
      return;
    }
    let middle = Rect { x: area.x.saturating_add(radius), w: area.w - radius * 2, ..area };
    let status = unsafe { (self.api.fill_rect)(self.api.renderer, middle, color) };
    self.checked(status, "Cannot fill rounded rectangle");
    let first = (i64::from(self.clip.y) - i64::from(area.y)).max(0) as i32;
    let end = (clip_bottom - i64::from(area.y)).min(i64::from(area.h)) as i32;
    for dy in first..end {
      let distance = if dy < radius {
        radius - dy
      } else if dy >= area.h - radius {
        dy - (area.h - radius - 1)
      } else {
        0
      };
      let inset = if distance == 0 {
        0
      } else {
        radius - integer_sqrt((i64::from(radius).pow(2) - i64::from(distance).pow(2)) as u64) as i32
      };
      let status = unsafe {
        (self.api.draw_line)(
          self.api.renderer,
          area.x.saturating_add(inset),
          area.y.saturating_add(dy),
          (area_right - i64::from(inset) - 1).clamp(i64::from(i32::MIN), i64::from(i32::MAX))
            as i32,
          area.y.saturating_add(dy),
          color,
        )
      };
      self.checked(status, "Cannot draw rounded rectangle");
    }
  }

  pub fn outline(&mut self, area: Rect, color: Color) {
    let status = unsafe { (self.api.outline_rect)(self.api.renderer, area, color) };
    self.checked(status, "Cannot outline rectangle");
  }

  pub fn clip(&mut self, area: Rect) {
    self.clip = area;
    let status = unsafe { (self.api.set_clip)(self.api.renderer, &area) };
    self.checked(status, "Cannot clip drawing");
  }

  pub fn unclip(&mut self) {
    self.clip = Rect { x: 0, y: 0, w: 1280, h: 720 };
    let status = unsafe { (self.api.set_clip)(self.api.renderer, core::ptr::null()) };
    self.checked(status, "Cannot clear drawing clip");
  }

  pub fn text_width(&mut self, value: &str, size: i32) -> i32 {
    let result = (|| {
      let font = self.font(size)?;
      let text = CString::new(value)
        .map_err(|_| "Text contains an unsupported NUL character.".to_string())?;
      let (mut width, mut height) = (0, 0);
      if unsafe { (self.api.text_size)(font, text.as_ptr(), &mut width, &mut height) } != 0 {
        return Err(self.native_error("Cannot measure text"));
      }
      Ok(width)
    })();
    match result {
      Ok(width) => width,
      Err(error) => {
        self.record(error);
        0
      }
    }
  }
}

fn integer_sqrt(value: u64) -> u64 {
  let (mut low, mut high) = (0u64, value.min(u32::MAX as u64) + 1);
  while low + 1 < high {
    let middle = low + (high - low) / 2;
    if middle <= value / middle {
      low = middle
    } else {
      high = middle
    }
  }
  low
}

pub fn preview(value: &str, max_bytes: usize) -> String {
  if value.len() <= max_bytes {
    return value.to_string();
  }
  let mut end = max_bytes;
  while !value.is_char_boundary(end) {
    end -= 1;
  }
  format!("{}…", &value[..end])
}

#[cfg(test)]
pub(crate) mod test_backend {
  use super::*;
  use alloc::boxed::Box;

  #[derive(Default)]
  pub struct Backend {
    fonts_opened: usize,
    fonts_closed: usize,
    textures_created: usize,
    textures_destroyed: usize,
    copies: usize,
    fills: usize,
    lines: Vec<Rect>,
    measured: Vec<String>,
    pub drawn_text: Vec<String>,
    fail_measure: bool,
    texture_bytes: usize,
  }

  struct FakeFont {
    backend: *mut Backend,
    size: i32,
  }

  struct FakeTexture {
    backend: *mut Backend,
  }

  unsafe extern "C" fn open(context: *const c_void, size: i32) -> *mut c_void {
    let backend = context as *mut Backend;
    (*backend).fonts_opened += 1;
    Box::into_raw(Box::new(FakeFont { backend, size })).cast()
  }

  unsafe extern "C" fn close(font: *mut c_void) {
    let font = Box::from_raw(font.cast::<FakeFont>());
    (*font.backend).fonts_closed += 1;
  }

  unsafe extern "C" fn skip(font: *mut c_void) -> i32 {
    (*font.cast::<FakeFont>()).size + 2
  }

  unsafe extern "C" fn measure(
    font: *mut c_void,
    value: *const c_char,
    width: i32,
    count: *mut i32,
  ) -> i32 {
    let font = &*font.cast::<FakeFont>();
    let text = CStr::from_ptr(value).to_str().unwrap();
    let backend = &mut *font.backend;
    backend.measured.push(text.to_string());
    if backend.fail_measure {
      return -1;
    }
    *count = (text.chars().count() as i32).min(width / 10);
    0
  }

  unsafe extern "C" fn size(
    _font: *mut c_void,
    value: *const c_char,
    width: *mut i32,
    height: *mut i32,
  ) -> i32 {
    *width = CStr::from_ptr(value).to_str().unwrap().chars().count() as i32 * 10;
    *height = 20;
    0
  }

  unsafe extern "C" fn texture(
    renderer: *mut c_void,
    _font: *mut c_void,
    value: *const c_char,
    _color: Color,
    _wrap: i32,
    width: *mut i32,
    height: *mut i32,
  ) -> *mut c_void {
    let backend = &mut *renderer.cast::<Backend>();
    backend.textures_created += 1;
    backend.drawn_text.push(CStr::from_ptr(value).to_string_lossy().into_owned());
    *height = 20;
    *width = if backend.texture_bytes == 0 {
      CStr::from_ptr(value).to_str().unwrap().chars().count() as i32 * 10
    } else {
      (backend.texture_bytes / 80) as i32
    };
    Box::into_raw(Box::new(FakeTexture { backend })).cast()
  }

  unsafe extern "C" fn destroy(texture: *mut c_void) {
    let texture = Box::from_raw(texture.cast::<FakeTexture>());
    (*texture.backend).textures_destroyed += 1;
  }

  unsafe extern "C" fn fill(renderer: *mut c_void, _area: Rect, _color: Color) -> i32 {
    (*renderer.cast::<Backend>()).fills += 1;
    0
  }

  unsafe extern "C" fn line(
    renderer: *mut c_void,
    x1: i32,
    y1: i32,
    x2: i32,
    _y2: i32,
    _color: Color,
  ) -> i32 {
    (*renderer.cast::<Backend>()).lines.push(Rect { x: x1, y: y1, w: x2 - x1 + 1, h: 1 });
    0
  }

  unsafe extern "C" fn copy(renderer: *mut c_void, _texture: *mut c_void, _area: Rect) -> i32 {
    (*renderer.cast::<Backend>()).copies += 1;
    0
  }

  unsafe extern "C" fn clip(_renderer: *mut c_void, _area: *const Rect) -> i32 {
    0
  }

  unsafe extern "C" fn error() -> *const c_char {
    c"mock render failure".as_ptr()
  }

  fn api(backend: &mut Backend) -> RenderApi {
    let context = (backend as *mut Backend).cast();
    RenderApi {
      renderer: context,
      platform: context,
      font_open: open,
      font_close: close,
      font_line_skip: skip,
      text_measure: measure,
      text_size: size,
      texture_create: texture,
      texture_destroy: destroy,
      fill_rect: fill,
      outline_rect: fill,
      draw_line: line,
      copy_texture: copy,
      set_clip: clip,
      error,
    }
  }

  pub fn canvas() -> (Box<Backend>, Canvas) {
    let mut backend = Box::<Backend>::default();
    let canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    (backend, canvas)
  }

  #[test]
  fn unicode_layout_is_bounded_and_only_visible_lines_create_textures() {
    let mut backend = Backend::default();
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    let value = "中文𠮷abc\n".repeat(100);
    let height = canvas.paragraph(&value, 0, 0, 20, INK, 30, false);
    assert_eq!(height, 200 * 26);
    assert_eq!(backend.textures_created, 0);
    assert!(backend.measured.iter().all(|candidate| candidate.len() <= 512));
    canvas.clip(Rect { x: 0, y: 0, w: 30, h: 50 });
    assert_eq!(canvas.paragraph(&value, 0, 0, 20, INK, 30, true), height);
    assert_eq!(backend.copies, 2);
    assert!(canvas.take_error().is_none());
  }

  #[test]
  fn wrapping_respects_unicode_boundaries_even_when_a_glyph_exceeds_width() {
    let mut backend = Backend::default();
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    assert_eq!(canvas.paragraph("中𠮷a", 0, 0, 20, INK, 1, false), 3 * 26);
    assert_eq!(canvas.paragraph("one two", 0, 0, 20, INK, 50, false), 2 * 26);
    assert_eq!(canvas.paragraph("\n\n", 0, 0, 20, INK, 50, false), 2 * 26);
    assert_eq!(preview("中𠮷a", 4), "中…");
    assert_eq!(preview("abc", 3), "abc");
    assert_eq!(preview("中文", 0), "…");
  }

  #[test]
  fn cache_distinguishes_alpha_and_releases_every_resource() {
    let mut backend = Backend::default();
    {
      let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
      canvas.text("中", 0, 0, 20, INK, 0);
      canvas.text("中", 10, 0, 20, INK, 0);
      assert_eq!(backend.textures_created, 1);
      canvas.text("中", 0, 0, 20, Color { a: 128, ..INK }, 0);
      assert_eq!(backend.textures_created, 2);
      canvas.text("文", 0, 0, 21, INK, 0);
      assert_eq!(backend.fonts_opened, 2);
    }
    assert_eq!(backend.fonts_closed, backend.fonts_opened);
    assert_eq!(backend.textures_destroyed, backend.textures_created);
  }

  #[test]
  fn texture_lru_and_oversized_transient_texture_preserve_memory_bound() {
    let mut backend = Backend { texture_bytes: 10 * 1024 * 1024, ..Backend::default() };
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    canvas.text("first", 0, 0, 20, INK, 0);
    canvas.text("second", 0, 0, 20, INK, 0);
    assert_eq!(backend.textures_destroyed, 1);
    assert_eq!(canvas.textures.len(), 1);
    assert!(canvas.texture_bytes <= CACHE_BYTES);
    backend.texture_bytes = 20 * 1024 * 1024;
    canvas.text("transient", 0, 0, 20, INK, 0);
    assert_eq!(backend.textures_destroyed, 2);
    assert_eq!(canvas.textures.len(), 1);
    assert!(canvas.texture_bytes <= CACHE_BYTES);
  }

  #[test]
  fn errors_are_reported_without_poisoning_font_resources() {
    let mut backend = Backend { fail_measure: true, ..Backend::default() };
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    canvas.paragraph("中文", 0, 0, 20, INK, 100, false);
    assert_eq!(canvas.take_error().as_deref(), Some("Cannot measure text: mock render failure"));
    canvas.text_width("bad\0text", 20);
    assert!(canvas.take_error().unwrap().contains("NUL"));
    assert_eq!(canvas.fonts.len(), 1);
  }

  #[test]
  fn rounded_rectangles_use_integer_geometry_without_std_math() {
    let mut backend = Backend::default();
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    canvas.rect(Rect { x: 4, y: 5, w: 20, h: 12 }, INK, 5);
    assert_eq!(backend.lines.len(), 12);
    assert_eq!(backend.lines.first().unwrap().w, 10);
    assert_eq!(backend.lines.first().unwrap().w, backend.lines.last().unwrap().w);
    assert_eq!(integer_sqrt(0), 0);
    assert_eq!(integer_sqrt(24), 4);
    assert_eq!(integer_sqrt(25), 5);
    assert_eq!(integer_sqrt(u64::MAX), u32::MAX as u64);
  }

  #[test]
  fn giant_message_cards_draw_only_visible_rows_and_skip_offscreen_geometry() {
    let mut backend = Backend::default();
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    canvas.clip(Rect { x: 0, y: 20, w: 100, h: 50 });
    canvas.rect(Rect { x: 0, y: -10_000_000, w: 100, h: 20_000_000 }, PANEL, 10);
    assert_eq!(backend.lines.len(), 50);
    assert_eq!(backend.lines.first().unwrap().y, 20);
    assert_eq!(backend.lines.last().unwrap().y, 69);
    assert_eq!(backend.fills, 1);

    canvas.rect(Rect { x: 0, y: 10_000_000, w: 100, h: 1000 }, PANEL, 10);
    canvas.rect(Rect { x: 100, y: 20, w: 100, h: 50 }, PANEL, 10);
    canvas.rect(Rect { x: -100, y: 20, w: 100, h: 50 }, PANEL, 0);
    assert_eq!(backend.lines.len(), 50);
    assert_eq!(backend.fills, 1);

    canvas.clip(Rect { x: i32::MAX - 20, y: i32::MAX - 20, w: 20, h: 20 });
    canvas.rect(Rect { x: i32::MAX - 10, y: i32::MAX - 10, w: 20, h: 20 }, PANEL, 5);
    assert_eq!(backend.lines.len(), 60);
    assert!(canvas.take_error().is_none());
  }

  #[test]
  fn far_scrolled_paragraph_renders_tail_and_retains_top_boundary_line() {
    let mut backend = Backend::default();
    let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
    let mut value = "before\n".repeat(60_000);
    value.push_str("TAIL 中文");
    let height = canvas.paragraph(&value, 0, 0, 20, INK, 100, false);
    assert_eq!(height, 60_001 * 26);
    assert_eq!(backend.textures_created, 0);
    canvas.clip(Rect { x: 0, y: 0, w: 100, h: 26 });
    assert_eq!(canvas.paragraph(&value, 0, -60_000 * 26, 20, INK, 100, true), height);
    assert_eq!(backend.copies, 2);
    assert_eq!(backend.drawn_text, ["before", "TAIL 中文"]);

    canvas.clip(Rect { x: 0, y: i32::MAX - 30, w: 100, h: 30 });
    canvas.paragraph(&value, 0, i32::MIN, 20, INK, 100, true);
    assert_eq!(backend.copies, 2);
    assert!(canvas.take_error().is_none());
  }
}

// Native integration tests invoke the same Canvas used by production views.
#[cfg(not(test))]
mod ffi {
  #![allow(clippy::missing_safety_doc)]
  use super::*;
  use alloc::boxed::Box;

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_new(api: *const RenderApi) -> *mut Canvas {
    let Some(api) = api.as_ref() else { return core::ptr::null_mut() };
    Canvas::new(api).map_or(core::ptr::null_mut(), |canvas| Box::into_raw(Box::new(canvas)))
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_free(canvas: *mut Canvas) {
    if !canvas.is_null() {
      drop(Box::from_raw(canvas));
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_paragraph(
    canvas: *mut Canvas,
    value: *const c_char,
    x: i32,
    y: i32,
    size: i32,
    color: Color,
    width: i32,
    draw: bool,
  ) -> i32 {
    let (Some(canvas), false) = (canvas.as_mut(), value.is_null()) else { return 0 };
    match CStr::from_ptr(value).to_str() {
      Ok(text) => canvas.paragraph(text, x, y, size, color, width, draw),
      Err(_) => {
        canvas.record("Text is not valid UTF-8.".to_string());
        0
      }
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_text_width(
    canvas: *mut Canvas,
    value: *const c_char,
    size: i32,
  ) -> i32 {
    let (Some(canvas), false) = (canvas.as_mut(), value.is_null()) else { return 0 };
    match CStr::from_ptr(value).to_str() {
      Ok(text) => canvas.text_width(text, size),
      Err(_) => {
        canvas.record("Text is not valid UTF-8.".to_string());
        0
      }
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_clip(canvas: *mut Canvas, area: Rect) {
    if let Some(canvas) = canvas.as_mut() {
      canvas.clip(area);
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_unclip(canvas: *mut Canvas) {
    if let Some(canvas) = canvas.as_mut() {
      canvas.unclip();
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_ok(canvas: *const Canvas) -> bool {
    canvas.as_ref().is_some_and(|canvas| canvas.error.is_none())
  }
}
