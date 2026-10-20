use crate::backend::images::DecodedImage;
use alloc::collections::BTreeMap;
use alloc::ffi::CString;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::{c_char, c_void, CStr};
use core::ops::Range;

const CACHE_BYTES: usize = 16 * 1024 * 1024;
const FONT_CACHE_LIMIT: usize = 32;
const IMAGE_CACHE_BYTES: usize = 16 * 1024 * 1024;
const IMAGE_CACHE_ENTRIES: usize = 32;
const IMAGE_URL_BYTES: usize = 4096;

#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontStyle {
  #[default]
  Normal = 0,
  Bold = 1,
  Italic = 2,
  BoldItalic = 3,
  Monospace = 4,
}

impl FontStyle {
  fn from_bits(value: u32) -> Option<Self> {
    match value {
      0 => Some(Self::Normal),
      1 => Some(Self::Bold),
      2 => Some(Self::Italic),
      3 => Some(Self::BoldItalic),
      4 => Some(Self::Monospace),
      _ => None,
    }
  }
}

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
  pub font_open: unsafe extern "C" fn(*const c_void, i32, u32) -> *mut c_void,
  pub font_close: unsafe extern "C" fn(*mut c_void),
  pub font_line_skip: unsafe extern "C" fn(*mut c_void) -> i32,
  pub font_ascent: unsafe extern "C" fn(*mut c_void) -> i32,
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
  /// Copies tightly packed RGBA8 pixels into a native texture synchronously.
  pub image_texture_create: unsafe extern "C" fn(*mut c_void, *const u8, u32, u32) -> *mut c_void,
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
  used: u64,
  monospace_cell: Option<i32>,
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
  style: FontStyle,
  width: i32,
  color: Color,
  text: String,
}

enum ImageState {
  Idle,
  Loading,
  Ready(Texture),
  Failed(String),
}

struct ImageEntry {
  state: ImageState,
  used: u64,
}

impl ImageEntry {
  fn bytes(&self) -> usize {
    match &self.state {
      ImageState::Ready(texture) => texture.bytes,
      _ => 0,
    }
  }
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
  // Canvas drop releases textures before the remaining cached font handles.
  textures: BTreeMap<TextureKey, Texture>,
  images: BTreeMap<String, ImageEntry>,
  image_requests: Vec<String>,
  image_bytes: usize,
  image_clock: u64,
  image_loading: bool,
  fonts: BTreeMap<(i32, FontStyle), Font>,
  layouts: BTreeMap<LayoutKey, Layout>,
  texture_bytes: usize,
  texture_clock: u64,
  font_clock: u64,
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
      images: BTreeMap::new(),
      image_requests: Vec::new(),
      image_bytes: 0,
      image_clock: 0,
      image_loading: true,
      fonts: BTreeMap::new(),
      layouts: BTreeMap::new(),
      texture_bytes: 0,
      texture_clock: 0,
      font_clock: 0,
      layout_bytes: 0,
      clip: Rect { x: 0, y: 0, w: 1280, h: 720 },
      error: None,
    };
    result.font(20, FontStyle::Normal)?;
    Ok(result)
  }

  pub fn take_error(&mut self) -> Option<String> {
    self.error.take()
  }

  /// A frame only offers URLs that its actual visible image cards draw.
  pub fn begin_image_frame(&mut self) {
    self.image_requests.clear();
  }

  pub fn set_image_loading(&mut self, enabled: bool) {
    if self.image_loading && !enabled {
      self.clear_images();
    }
    self.image_loading = enabled;
  }

  pub fn take_image_requests(&mut self) -> Vec<String> {
    core::mem::take(&mut self.image_requests)
  }

  pub fn mark_image_loading(&mut self, url: &str) {
    if let Some(entry) = self.images.get_mut(url) {
      if matches!(entry.state, ImageState::Idle) {
        entry.state = ImageState::Loading;
      }
    }
  }

  pub fn clear_images(&mut self) {
    self.images.clear();
    self.image_requests.clear();
    self.image_bytes = 0;
  }

  fn evict_image(&mut self, except: &str) -> bool {
    let oldest = self
      .images
      .iter()
      .filter(|(url, _)| url.as_str() != except)
      .min_by_key(|(_, entry)| entry.used)
      .map(|(url, _)| url.clone());
    let Some(oldest) = oldest else { return false };
    if let Some(entry) = self.images.remove(&oldest) {
      self.image_bytes -= entry.bytes();
    }
    self.image_requests.retain(|url| url != &oldest);
    true
  }

  pub fn set_image_result(&mut self, url: &str, result: Result<DecodedImage, String>) {
    // Cancellation, conversation changes, and eviction must never resurrect an
    // old request or allocate a texture for a result that is no longer wanted.
    if !self.images.get(url).is_some_and(|entry| matches!(entry.state, ImageState::Loading)) {
      return;
    }
    let result = result.and_then(|image| {
      let bytes = u64::from(image.width)
        .checked_mul(u64::from(image.height))
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= IMAGE_CACHE_BYTES as u64 && *bytes == image.rgba.len() as u64);
      let Some(bytes) = bytes else {
        return Err("The image is too large or has invalid pixels.".into());
      };
      if image.width == 0
        || image.height == 0
        || image.width > i32::MAX as u32 / 4
        || image.height > i32::MAX as u32
      {
        return Err("The image has invalid dimensions.".into());
      }
      while self.image_bytes.saturating_add(bytes as usize) > IMAGE_CACHE_BYTES {
        if !self.evict_image(url) {
          return Err("The image cache is full.".into());
        }
      }
      let handle = unsafe {
        (self.api.image_texture_create)(
          self.api.renderer,
          image.rgba.as_ptr(),
          image.width,
          image.height,
        )
      };
      if handle.is_null() {
        return Err("The image could not be displayed.".into());
      }
      Ok(Texture {
        handle,
        destroy: self.api.texture_destroy,
        width: image.width as i32,
        height: image.height as i32,
        bytes: bytes as usize,
        used: self.image_clock,
      })
    });
    let state = match result {
      Ok(texture) => {
        self.image_bytes += texture.bytes;
        ImageState::Ready(texture)
      }
      Err(message) => ImageState::Failed(preview(&message.replace('\0', ""), 120)),
    };
    if let Some(entry) = self.images.get_mut(url) {
      entry.state = state;
    }
  }

  /// Draws a fixed-size image card without fetching or decoding on the UI thread.
  pub fn draw_image(&mut self, url: &str, alt: &str, area: Rect) {
    let Some(clip) = intersection(self.clip, area) else { return };
    let outer = self.clip;
    self.clip(clip);
    self.rect(area, BACKGROUND, 4);
    let frame =
      Rect { x: area.x + 8, y: area.y + 8, w: (area.w - 16).max(1), h: (area.h - 38).max(1) };
    self.image_clock = self.image_clock.saturating_add(1);
    let valid_url = self.image_loading && !url.is_empty() && url.len() <= IMAGE_URL_BYTES;
    if valid_url && !self.images.contains_key(url) {
      while self.images.len() >= IMAGE_CACHE_ENTRIES {
        if !self.evict_image(url) {
          break;
        }
      }
      self
        .images
        .insert(url.into(), ImageEntry { state: ImageState::Idle, used: self.image_clock });
    }
    let mut pixels = None;
    let mut status = String::from(if self.image_loading {
      "Image address unavailable"
    } else {
      "Images disabled"
    });
    if let Some(entry) = self.images.get_mut(url).filter(|_| self.image_loading) {
      entry.used = self.image_clock;
      match &entry.state {
        ImageState::Ready(texture) => {
          pixels = Some((texture.handle, texture.width, texture.height))
        }
        ImageState::Idle => {
          if !self.image_requests.iter().any(|request| request == url) {
            self.image_requests.push(url.into());
          }
          status = "Loading image…".into();
        }
        ImageState::Loading => status = "Loading image…".into(),
        ImageState::Failed(message) => {
          status = if message.is_empty() { "Image unavailable".into() } else { message.clone() };
        }
      }
    }
    if let Some((handle, width, height)) = pixels {
      let destination = fit_image(frame, width, height);
      let status = unsafe { (self.api.copy_texture)(self.api.renderer, handle, destination) };
      self.checked(status, "Cannot draw image");
    } else if let Some(frame_clip) = intersection(clip, frame) {
      self.clip(frame_clip);
      let text_y = frame.y + (frame.h - 22).max(0) / 2;
      self.paragraph(&status, frame.x + 6, text_y, 15, MUTED, (frame.w - 12).max(1), true);
      self.clip(clip);
    }
    let caption = if alt.trim().is_empty() { "Image" } else { alt };
    let caption = preview(&caption.replace(['\n', '\r', '\0'], " "), 256);
    let length = self.measure_prefix(&caption, 14, FontStyle::Normal, frame.w);
    self.text(&caption[..length], frame.x, area.y + area.h - 25, 14, MUTED, 0);
    self.clip(outer);
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

  fn font(&mut self, size: i32, style: FontStyle) -> Result<*mut c_void, String> {
    self.font_clock = self.font_clock.saturating_add(1);
    if let Some(font) = self.fonts.get_mut(&(size, style)) {
      font.used = self.font_clock;
      return Ok(font.handle);
    }
    if !(1..=512).contains(&size) {
      return Err("Font size must be between 1 and 512 points.".to_string());
    }
    let handle = unsafe { (self.api.font_open)(self.api.platform, size, style as u32) };
    if handle.is_null() {
      return Err(self.native_error("Cannot load font"));
    }
    if self.fonts.len() >= FONT_CACHE_LIMIT {
      let oldest = self.fonts.iter().min_by_key(|(_, font)| font.used).map(|(key, _)| *key);
      if let Some(oldest) = oldest {
        self.fonts.remove(&oldest);
      }
    }
    self.fonts.insert(
      (size, style),
      Font { handle, close: self.api.font_close, used: self.font_clock, monospace_cell: None },
    );
    Ok(handle)
  }

  fn texture(&mut self, key: &TextureKey) -> Result<Texture, String> {
    let value = CString::new(key.text.as_str())
      .map_err(|_| "Text contains an unsupported NUL character.".to_string())?;
    let font = self.font(key.size, key.style)?;
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
    self.styled_text(value, x, y, size, color, width, FontStyle::Normal)
  }

  #[allow(clippy::too_many_arguments)]
  pub fn styled_text(
    &mut self,
    value: &str,
    x: i32,
    y: i32,
    size: i32,
    color: Color,
    width: i32,
    style: FontStyle,
  ) -> i32 {
    if value.is_empty() {
      return size.saturating_add(6);
    }
    self.texture_clock = self.texture_clock.saturating_add(1);
    let key = TextureKey { size, style, width, color, text: value.to_string() };
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
            style: key.style,
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
    let font = self.font(key.size, FontStyle::Normal)?;
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

  pub fn clip_area(&self) -> Rect {
    self.clip
  }

  pub fn unclip(&mut self) {
    self.clip = Rect { x: 0, y: 0, w: 1280, h: 720 };
    let status = unsafe { (self.api.set_clip)(self.api.renderer, core::ptr::null()) };
    self.checked(status, "Cannot clear drawing clip");
  }

  pub fn text_width(&mut self, value: &str, size: i32) -> i32 {
    self.styled_text_width(value, size, FontStyle::Normal)
  }

  pub fn styled_text_width(&mut self, value: &str, size: i32, style: FontStyle) -> i32 {
    let result = (|| {
      let font = self.font(size, style)?;
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

  pub fn line_height(&mut self, size: i32, style: FontStyle) -> i32 {
    match self.font(size, style) {
      Ok(font) => unsafe { (self.api.font_line_skip)(font) }.saturating_add(4).max(1),
      Err(error) => {
        self.record(error);
        size.saturating_add(6).max(1)
      }
    }
  }

  pub fn font_ascent(&mut self, size: i32, style: FontStyle) -> i32 {
    match self.font(size, style) {
      Ok(font) => unsafe { (self.api.font_ascent)(font) }.max(1),
      Err(error) => {
        self.record(error);
        size.max(1)
      }
    }
  }

  /// Fixed code cells contain every printable ASCII glyph and reserve two
  /// cells for CJK. Measurements follow the selected bundled or override font.
  pub fn monospace_cell_width(&mut self, size: i32) -> i32 {
    let result = (|| {
      let handle = self.font(size, FontStyle::Monospace)?;
      let key = (size, FontStyle::Monospace);
      if let Some(cell) = self.fonts.get(&key).and_then(|font| font.monospace_cell) {
        return Ok(cell);
      }
      let mut cell = 1;
      for byte in b' '..=b'~' {
        let text = [byte, 0];
        let (mut width, mut height) = (0, 0);
        if unsafe { (self.api.text_size)(handle, text.as_ptr().cast(), &mut width, &mut height) }
          != 0
        {
          return Err(self.native_error("Cannot measure code font"));
        }
        cell = cell.max(width);
      }
      let cjk = self.font(size, FontStyle::Normal)?;
      let (mut width, mut height) = (0, 0);
      if unsafe { (self.api.text_size)(cjk, c"中".as_ptr(), &mut width, &mut height) } != 0 {
        return Err(self.native_error("Cannot measure code font"));
      }
      cell = cell.max(width.saturating_add(1) / 2);
      if let Some(font) = self.fonts.get_mut(&key) {
        font.monospace_cell = Some(cell);
      }
      Ok(cell)
    })();
    match result {
      Ok(cell) => cell,
      Err(error) => {
        self.record(error);
        size.max(1)
      }
    }
  }

  /// Returns a UTF-8 byte boundary fitting the width, measuring at most 512
  /// bytes at a time. A nonempty input always advances by at least one scalar,
  /// even when a single glyph is wider than the available space.
  pub fn measure_prefix(&mut self, value: &str, size: i32, style: FontStyle, width: i32) -> usize {
    let Some(first) = value.chars().next() else { return 0 };
    let result = (|| {
      let font = self.font(size, style)?;
      let mut end = value.len().min(512);
      while !value.is_char_boundary(end) {
        end -= 1;
      }
      let candidate = &value[..end];
      let text = CString::new(candidate)
        .map_err(|_| "Text contains an unsupported NUL character.".to_string())?;
      let mut count = 0;
      if unsafe { (self.api.text_measure)(font, text.as_ptr(), width.max(1), &mut count) } != 0 {
        return Err(self.native_error("Cannot measure text"));
      }
      Ok(candidate.char_indices().nth(count.max(1) as usize).map_or(end, |(index, _)| index))
    })();
    match result {
      Ok(length) => length,
      Err(error) => {
        self.record(error);
        first.len_utf8()
      }
    }
  }
}

fn intersection(left: Rect, right: Rect) -> Option<Rect> {
  if left.w <= 0 || left.h <= 0 || right.w <= 0 || right.h <= 0 {
    return None;
  }
  let x = left.x.max(right.x);
  let y = left.y.max(right.y);
  let end_x = (i64::from(left.x) + i64::from(left.w)).min(i64::from(right.x) + i64::from(right.w));
  let end_y = (i64::from(left.y) + i64::from(left.h)).min(i64::from(right.y) + i64::from(right.h));
  let width = end_x - i64::from(x);
  let height = end_y - i64::from(y);
  (width > 0 && height > 0).then_some(Rect { x, y, w: width as i32, h: height as i32 })
}

fn fit_image(area: Rect, width: i32, height: i32) -> Rect {
  let (width, height) = (i64::from(width.max(1)), i64::from(height.max(1)));
  let (available_width, available_height) = (i64::from(area.w.max(1)), i64::from(area.h.max(1)));
  // Preserve aspect ratio, including very tall/narrow images, using integer
  // arithmetic that is available in the Switch's no_std target.
  let (draw_width, draw_height) = if width * available_height > height * available_width {
    (available_width, (height * available_width / width).max(1))
  } else {
    ((width * available_height / height).max(1), available_height)
  };
  Rect {
    x: area.x.saturating_add((area.w - draw_width as i32) / 2),
    y: area.y.saturating_add((area.h - draw_height as i32) / 2),
    w: draw_width as i32,
    h: draw_height as i32,
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
    pub copied_rects: Vec<Rect>,
    pub image_uploads: Vec<(u32, u32)>,
    fail_image: bool,
    fail_measure: bool,
    texture_bytes: usize,
    wide_ascii: bool,
    wide_cjk: bool,
    size_calls: usize,
  }

  struct FakeFont {
    backend: *mut Backend,
    size: i32,
    style: u32,
  }

  struct FakeTexture {
    backend: *mut Backend,
  }

  unsafe extern "C" fn open(context: *const c_void, size: i32, style: u32) -> *mut c_void {
    let backend = context as *mut Backend;
    (*backend).fonts_opened += 1;
    Box::into_raw(Box::new(FakeFont { backend, size, style })).cast()
  }

  unsafe extern "C" fn close(font: *mut c_void) {
    let font = Box::from_raw(font.cast::<FakeFont>());
    (*font.backend).fonts_closed += 1;
  }

  unsafe extern "C" fn skip(font: *mut c_void) -> i32 {
    (*font.cast::<FakeFont>()).size + 2
  }

  unsafe extern "C" fn ascent(font: *mut c_void) -> i32 {
    let font = &*font.cast::<FakeFont>();
    (font.size - if font.style == FontStyle::Monospace as u32 { 4 } else { 0 }).max(1)
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
    *count = (text.chars().count() as i32).min(width / (10 + font.style as i32));
    0
  }

  unsafe extern "C" fn size(
    font: *mut c_void,
    value: *const c_char,
    width: *mut i32,
    height: *mut i32,
  ) -> i32 {
    let font = &*font.cast::<FakeFont>();
    let backend = &mut *font.backend;
    backend.size_calls += 1;
    let advance = 10 + font.style as i32;
    *width = CStr::from_ptr(value)
      .to_str()
      .unwrap()
      .chars()
      .map(|character| {
        if backend.wide_ascii {
          match character {
            'M' => 10,
            'm' => 13,
            '@' => 14,
            '中' if backend.wide_cjk => 31,
            _ => advance,
          }
        } else {
          advance
        }
      })
      .sum();
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

  unsafe extern "C" fn image_texture(
    renderer: *mut c_void,
    _pixels: *const u8,
    width: u32,
    height: u32,
  ) -> *mut c_void {
    let backend = &mut *renderer.cast::<Backend>();
    if backend.fail_image {
      return core::ptr::null_mut();
    }
    backend.image_uploads.push((width, height));
    backend.textures_created += 1;
    Box::into_raw(Box::new(FakeTexture { backend })).cast()
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

  unsafe extern "C" fn copy(renderer: *mut c_void, _texture: *mut c_void, area: Rect) -> i32 {
    let backend = &mut *renderer.cast::<Backend>();
    backend.copies += 1;
    backend.copied_rects.push(area);
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
      font_ascent: ascent,
      text_measure: measure,
      text_size: size,
      texture_create: texture,
      texture_destroy: destroy,
      image_texture_create: image_texture,
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

  fn image(width: u32, height: u32) -> DecodedImage {
    DecodedImage { width, height, rgba: alloc::vec![255; width as usize * height as usize * 4] }
  }

  #[test]
  fn images_request_only_visible_missing_urls_and_preserve_aspect_ratio() {
    let (mut backend, mut canvas) = canvas();
    canvas.clip(Rect { x: 0, y: 0, w: 300, h: 200 });
    let card = Rect { x: 0, y: 0, w: 200, h: 150 };
    let url = "https://example.test/cat.png";
    canvas.begin_image_frame();
    canvas.draw_image(url, "cat", Rect { y: 200, ..card });
    canvas.draw_image(url, "cat", Rect { x: 300, ..card });
    assert!(canvas.take_image_requests().is_empty());
    assert!(canvas.images.is_empty());
    canvas.draw_image(url, "cat", card);
    canvas.draw_image(url, "same cat", card);
    assert_eq!(canvas.take_image_requests(), [url]);
    canvas.mark_image_loading(url);
    canvas.draw_image(url, "cat", card);
    assert!(canvas.take_image_requests().is_empty());
    canvas.set_image_result(url, Ok(image(4, 2)));
    assert_eq!(backend.image_uploads, [(4, 2)]);
    backend.copied_rects.clear();
    canvas.draw_image(url, "cat", card);
    assert_eq!(backend.copied_rects[0], Rect { x: 8, y: 18, w: 184, h: 92 });
    assert_eq!(canvas.clip_area(), Rect { x: 0, y: 0, w: 300, h: 200 });
    let partial_clip = Rect { x: 100, y: 40, w: 60, h: 30 };
    canvas.clip(partial_clip);
    backend.copied_rects.clear();
    canvas.draw_image(url, "cat", card);
    // Cropping uses the viewport, never a rescaled or stretched destination.
    assert_eq!(backend.copied_rects[0], Rect { x: 8, y: 18, w: 184, h: 92 });
    assert_eq!(canvas.clip_area(), partial_clip);
    assert!(canvas.take_image_requests().is_empty());
    assert!(canvas.take_error().is_none());
  }

  #[test]
  fn disabled_failed_invalid_and_stale_images_do_not_allocate_or_repeat_requests() {
    let (mut backend, mut canvas) = canvas();
    let card = Rect { x: 0, y: 0, w: 200, h: 150 };
    let url = "https://example.test/cat.png";
    canvas.set_image_loading(false);
    canvas.draw_image(url, "cat", card);
    assert!(canvas.take_image_requests().is_empty());
    assert!(canvas.images.is_empty());
    assert!(backend.drawn_text.iter().any(|text| text == "Images disabled"));
    canvas.set_image_loading(true);
    canvas.draw_image(url, "cat", card);
    canvas.begin_image_frame();
    assert!(canvas.take_image_requests().is_empty());
    canvas.mark_image_loading(url);
    canvas
      .set_image_result(url, Ok(DecodedImage { width: 20, height: 20, rgba: alloc::vec![0; 4] }));
    canvas.draw_image(url, "cat", card);
    assert!(canvas.take_image_requests().is_empty());
    assert!(backend.image_uploads.is_empty());
    assert!(matches!(canvas.images[url].state, ImageState::Failed(_)));
    canvas.clear_images();
    canvas.set_image_result(url, Ok(image(2, 2)));
    assert!(backend.image_uploads.is_empty());
    canvas.draw_image(url, "cat", card);
    canvas.mark_image_loading(url);
    backend.fail_image = true;
    canvas.set_image_result(url, Ok(image(2, 2)));
    assert!(matches!(canvas.images[url].state, ImageState::Failed(_)));
    assert!(canvas.take_error().is_none());
    canvas.set_image_loading(false);
    assert!(canvas.images.is_empty());
  }

  #[test]
  fn image_cache_bounds_pixels_metadata_and_releases_native_textures() {
    let (backend, mut canvas) = canvas();
    let card = Rect { x: 0, y: 0, w: 200, h: 150 };
    for name in ["first", "second", "third"] {
      canvas.draw_image(name, name, card);
      canvas.mark_image_loading(name);
      canvas.set_image_result(name, Ok(image(2048, 1024)));
    }
    assert_eq!(canvas.images.len(), 2);
    assert_eq!(canvas.image_bytes, IMAGE_CACHE_BYTES);
    assert!(!canvas.images.contains_key("first"));
    assert_eq!(backend.image_uploads.len(), 3);
    let destroyed = backend.textures_destroyed;
    canvas.clear_images();
    assert_eq!(backend.textures_destroyed, destroyed + 2);
    assert_eq!(canvas.image_bytes, 0);
    for index in 0..40 {
      let name = format!("missing-{index}");
      canvas.draw_image(&name, &name, card);
      canvas.mark_image_loading(&name);
      canvas.set_image_result(&name, Err("Not found".into()));
    }
    assert_eq!(canvas.images.len(), IMAGE_CACHE_ENTRIES);
    assert_eq!(canvas.image_bytes, 0);
    assert!(canvas.take_image_requests().len() <= IMAGE_CACHE_ENTRIES);
    drop(canvas);
    assert_eq!(backend.textures_created, backend.textures_destroyed);
    assert_eq!(backend.fonts_opened, backend.fonts_closed);
  }

  #[test]
  fn image_fit_and_clip_intersections_handle_extreme_aspect_ratios() {
    let frame = Rect { x: 10, y: 20, w: 100, h: 100 };
    assert_eq!(fit_image(frame, 1, 10000), Rect { x: 59, y: 20, w: 1, h: 100 });
    assert_eq!(fit_image(frame, 10000, 1), Rect { x: 10, y: 69, w: 100, h: 1 });
    assert_eq!(intersection(frame, Rect { x: 110, ..frame }), None);
    assert_eq!(intersection(frame, Rect { w: 0, ..frame }), None);
    assert_eq!(
      intersection(frame, Rect { x: 100, y: 50, w: 100, h: 100 }),
      Some(Rect { x: 100, y: 50, w: 10, h: 70 })
    );
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
  fn styles_use_independent_metrics_textures_and_bounded_font_handles() {
    let mut backend = Backend::default();
    {
      let mut canvas = unsafe { Canvas::new(&api(&mut backend)) }.unwrap();
      for (style, width) in [
        (FontStyle::Normal, 20),
        (FontStyle::Bold, 22),
        (FontStyle::Italic, 24),
        (FontStyle::BoldItalic, 26),
      ] {
        assert_eq!(FontStyle::from_bits(style as u32), Some(style));
        assert_eq!(canvas.styled_text_width("中a", 20, style), width);
        canvas.styled_text("中a", 0, 0, 20, INK, 0, style);
        canvas.styled_text("中a", 0, 0, 20, INK, 0, style);
      }
      assert_eq!(FontStyle::from_bits(4), Some(FontStyle::Monospace));
      assert_eq!(FontStyle::from_bits(5), None);
      assert_eq!(backend.fonts_opened, 4);
      assert_eq!(backend.textures_created, 4);
      assert_eq!(canvas.text_width("中a", 20), 20);
      for size in 30..80 {
        canvas.styled_text_width("x", size, FontStyle::Bold);
      }
      assert_eq!(canvas.fonts.len(), FONT_CACHE_LIMIT);
      assert_eq!(backend.fonts_opened - backend.fonts_closed, FONT_CACHE_LIMIT);
      canvas.styled_text("中a", 0, 0, 20, INK, 0, FontStyle::Bold);
      assert_eq!(backend.textures_created, 4);
    }
    assert_eq!(backend.fonts_closed, backend.fonts_opened);
    assert_eq!(backend.textures_destroyed, backend.textures_created);
  }

  #[test]
  fn styled_prefix_measurement_is_bounded_unicode_safe_and_always_advances() {
    let (mut backend, mut canvas) = canvas();
    assert_eq!(canvas.measure_prefix("", 20, FontStyle::Bold, 0), 0);
    assert_eq!(canvas.measure_prefix("中𠮷a", 20, FontStyle::Bold, 1), 3);
    assert_eq!(canvas.measure_prefix("中𠮷a", 20, FontStyle::Bold, 22), 7);
    assert_eq!(canvas.measure_prefix("中𠮷a", 20, FontStyle::Bold, 100), 8);
    assert_eq!(canvas.measure_prefix(&"中".repeat(300), 20, FontStyle::Normal, 9000), 510);
    assert!(backend.measured.iter().all(|value| value.len() <= 512));
    assert_eq!(canvas.line_height(20, FontStyle::BoldItalic), 26);
    backend.fail_measure = true;
    assert_eq!(canvas.measure_prefix("𠮷中", 20, FontStyle::Italic, 100), 4);
    assert!(canvas.take_error().is_some());
  }

  #[test]
  fn monospace_cells_contain_ascii_and_cjk_and_reuse_font_metrics() {
    let (mut backend, mut canvas) = canvas();
    backend.wide_ascii = true;
    assert_eq!(canvas.monospace_cell_width(20), 14);
    assert_eq!(backend.size_calls, 96);
    assert_eq!(canvas.monospace_cell_width(20), 14);
    assert_eq!(backend.size_calls, 96);
    backend.wide_cjk = true;
    assert_eq!(canvas.monospace_cell_width(18), 16);
    assert_eq!(backend.size_calls, 192);
    assert_eq!(canvas.font_ascent(20, FontStyle::Normal), 20);
    assert_eq!(canvas.font_ascent(20, FontStyle::Monospace), 16);
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
  pub unsafe extern "C" fn wodex_canvas_styled_text(
    canvas: *mut Canvas,
    value: *const c_char,
    x: i32,
    y: i32,
    size: i32,
    color: Color,
    style: u32,
  ) -> i32 {
    let (Some(canvas), false) = (canvas.as_mut(), value.is_null()) else { return 0 };
    match (CStr::from_ptr(value).to_str(), FontStyle::from_bits(style)) {
      (Ok(text), Some(style)) => canvas.styled_text(text, x, y, size, color, 0, style),
      _ => {
        canvas.record("Text or font style is invalid.".to_string());
        0
      }
    }
  }

  #[no_mangle]
  pub unsafe extern "C" fn wodex_canvas_styled_text_width(
    canvas: *mut Canvas,
    value: *const c_char,
    size: i32,
    style: u32,
  ) -> i32 {
    let (Some(canvas), false) = (canvas.as_mut(), value.is_null()) else { return 0 };
    match (CStr::from_ptr(value).to_str(), FontStyle::from_bits(style)) {
      (Ok(text), Some(style)) => canvas.styled_text_width(text, size, style),
      _ => {
        canvas.record("Text or font style is invalid.".to_string());
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
