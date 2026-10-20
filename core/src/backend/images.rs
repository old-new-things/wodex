//! Bounded, credential-free image downloads and decoding on one Rust worker.
use super::{
  http::{Http, HttpError, Request, UrlParts},
  runtime::{JoinHandle, Mutex, Runtime},
};
use alloc::{format, string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};

const MAX_URL_BYTES: usize = 4096;
const MAX_DOWNLOAD_BYTES: usize = 4 * 1024 * 1024;
const MAX_SOURCE_AXIS: usize = 4096;
const MAX_SOURCE_PIXELS: usize = 2 * 1024 * 1024;
const MAX_OUTPUT_AXIS: usize = 1024;
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug)]
pub struct DecodedImage {
  pub width: u32,
  pub height: u32,
  pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub struct ImageResult {
  pub url: String,
  pub result: Result<DecodedImage, String>,
}

struct Shared {
  result: Mutex<Option<ImageResult>>,
  cancelled: AtomicBool,
  active: AtomicBool,
}

pub struct ImageLoader {
  runtime: Runtime,
  http: Http,
  ca_bundle: String,
  shared: Arc<Shared>,
  worker: Option<JoinHandle>,
}

impl ImageLoader {
  pub fn new(runtime: Runtime, http: Http, ca_bundle: String) -> Result<Self, String> {
    Ok(Self {
      runtime,
      http,
      ca_bundle,
      shared: Arc::new(Shared {
        result: runtime.mutex(None)?,
        cancelled: AtomicBool::new(false),
        active: AtomicBool::new(false),
      }),
      worker: None,
    })
  }

  /// Accept one request, including invalid URLs which produce a pollable error.
  /// An unconsumed result occupies the only slot, keeping memory bounded.
  pub fn request(&mut self, url: &str) -> bool {
    if self.is_running() || self.shared.result.lock().is_some() {
      return false;
    }
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
    self.shared.cancelled.store(false, Ordering::Release);
    self.shared.active.store(true, Ordering::Release);
    // Preserve the source URL even for validation failures so the UI can
    // resolve its loading entry. Markdown source is separately bounded.
    if url.len() > MAX_URL_BYTES {
      self.shared.finish(url.into(), Err("Image URL exceeds the 4096-byte limit.".into()));
      return true;
    }
    let url = String::from(url);
    if !valid_url(self.http, &url) {
      self.shared.finish(url, Err("Images require a direct HTTPS URL without credentials.".into()));
      return true;
    }
    let mut request = Request::default();
    request.url = url.clone();
    request.headers = vec!["Accept: image/png, image/jpeg".into()];
    request.ca_bundle = self.ca_bundle.clone();
    request.timeout_seconds = 30;
    request.connect_timeout_seconds = 10;
    request.low_speed_bytes = 128;
    request.low_speed_seconds = 10;
    request.body_status = Some(200);
    request.abort_rejected_body = true;
    let shared = Arc::clone(&self.shared);
    let http = self.http;
    let failure_url = url.clone();
    match self.runtime.spawn(move || {
      let result = download(http, &request, &shared.cancelled)
        .and_then(|bytes| decode(&bytes, &shared.cancelled));
      shared.finish(url, result);
    }) {
      Ok(worker) => self.worker = Some(worker),
      Err(error) => self.shared.finish(failure_url, Err(error)),
    }
    true
  }

  pub fn poll(&mut self) -> Option<ImageResult> {
    self.shared.result.lock().take()
  }

  pub fn is_running(&self) -> bool {
    self.shared.active.load(Ordering::Acquire)
  }

  pub fn cancel(&self) {
    self.shared.cancelled.store(true, Ordering::Release);
  }
}

impl Drop for ImageLoader {
  fn drop(&mut self) {
    self.cancel();
    if let Some(worker) = self.worker.take() {
      worker.join();
    }
  }
}

impl Shared {
  fn finish(&self, url: String, result: Result<DecodedImage, String>) {
    let mut slot = self.result.lock();
    let result = if self.cancelled.load(Ordering::Acquire) {
      Err("Image loading cancelled.".into())
    } else {
      result
    };
    *slot = Some(ImageResult { url, result });
    self.active.store(false, Ordering::Release);
  }
}

fn valid_url(http: Http, url: &str) -> bool {
  !url.is_empty()
    && url.len() <= MAX_URL_BYTES
    && !url.bytes().any(|byte| byte <= 0x20 || byte == 0x7f || byte == b'\\')
    && http.parse_url(url).is_ok_and(|parts| valid_url_parts(&parts))
}

fn valid_url_parts(parts: &UrlParts) -> bool {
  parts.scheme.eq_ignore_ascii_case("https")
    && !parts.host.is_empty()
    && !parts.has_user
    && !parts.has_password
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
  if cancelled.load(Ordering::Acquire) {
    Err("Image loading cancelled.".into())
  } else {
    Ok(())
  }
}

fn download(http: Http, request: &Request, cancelled: &AtomicBool) -> Result<Vec<u8>, String> {
  let mut bytes = Vec::new();
  let response = http
    .perform(request, cancelled, &mut |chunk| {
      check_cancelled(cancelled)?;
      if chunk.len() > MAX_DOWNLOAD_BYTES - bytes.len() {
        return Err("Image download exceeds the 4 MiB limit.".into());
      }
      let needed = bytes.len() + chunk.len();
      if needed > bytes.capacity() {
        let capacity =
          needed.max(bytes.capacity().saturating_mul(2)).clamp(16 * 1024, MAX_DOWNLOAD_BYTES);
        bytes
          .try_reserve_exact(capacity - bytes.len())
          .map_err(|_| String::from("Not enough memory to download the image."))?;
      }
      bytes.extend_from_slice(chunk);
      Ok(())
    })
    .map_err(|error| match error {
      HttpError::Cancelled => String::from("Image loading cancelled."),
      HttpError::Failure(error) => error,
    })?;
  if (300..400).contains(&response.http_status) {
    return Err("Image URL redirects; use a direct HTTPS image URL.".into());
  }
  if response.http_status != 200 {
    return Err(format!("Image server returned HTTP {}.", response.http_status));
  }
  if response.curl_code != 0 {
    return Err("Cannot download the image. Check the network and image URL.".into());
  }
  Ok(bytes)
}

fn decoder_options() -> DecoderOptions {
  DecoderOptions::new_safe()
    .set_max_width(MAX_SOURCE_AXIS)
    .set_max_height(MAX_SOURCE_AXIS)
    .set_strict_mode(true)
    .png_set_strip_to_8bit(true)
    .png_set_confirm_crc(true)
    .inflate_set_confirm_adler(true)
    .jpeg_set_out_colorspace(ColorSpace::RGBA)
    .jpeg_set_max_scans(32)
}

fn dimensions(width: usize, height: usize) -> Result<(), String> {
  if width == 0
    || height == 0
    || width > MAX_SOURCE_AXIS
    || height > MAX_SOURCE_AXIS
    || width.checked_mul(height).is_none_or(|pixels| pixels > MAX_SOURCE_PIXELS)
  {
    return Err("Image exceeds 4096 pixels per side or the 2-megapixel limit.".into());
  }
  Ok(())
}

/// Remove ancillary metadata before decoding. zune-png decompresses ICC/zTXt
/// metadata separately from its bounded pixel inflater; it is unnecessary for
/// an inline preview and must never consume unbounded memory.
/// Discarding APNG control chunks selects its static default image.
fn png_pixels_only(bytes: &[u8]) -> Result<Vec<u8>, String> {
  let invalid = || String::from("Invalid or truncated PNG image.");
  if bytes.len() < 33 || bytes.get(12..16) != Some(b"IHDR") {
    return Err(invalid());
  }
  let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
  let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
  dimensions(width, height)?;
  if bytes[24] > 8 {
    return Err("16-bit PNG images are not supported. Use an 8-bit PNG or JPEG.".into());
  }
  let (depth, color) = (bytes[24], bytes[25]);
  if !matches!((color, depth), (0 | 3, 1 | 2 | 4 | 8) | (2 | 4 | 6, 8)) {
    return Err(invalid());
  }
  let mut filtered = Vec::new();
  filtered
    .try_reserve_exact(bytes.len())
    .map_err(|_| String::from("Not enough memory to decode the image."))?;
  filtered.extend_from_slice(PNG_SIGNATURE);
  let mut offset = PNG_SIGNATURE.len();
  let mut saw_header = false;
  let mut saw_pixels = false;
  let mut palette_entries = 0;
  let mut saw_transparency = false;
  while offset < bytes.len() {
    let header = bytes.get(offset..offset.saturating_add(8)).ok_or_else(invalid)?;
    let length = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
    let end = offset.checked_add(length).and_then(|end| end.checked_add(12)).ok_or_else(invalid)?;
    let chunk = bytes.get(offset..end).ok_or_else(invalid)?;
    let kind = &header[4..8];
    match kind {
      b"IHDR" => {
        if saw_header || offset != PNG_SIGNATURE.len() || length != 13 {
          return Err(invalid());
        }
        saw_header = true;
      }
      b"IDAT" => saw_pixels = true,
      b"IEND" => {
        if !saw_header
          || !saw_pixels
          || length != 0
          || end != bytes.len()
          || (color == 3 && palette_entries == 0)
        {
          return Err(invalid());
        }
        filtered.extend_from_slice(chunk);
        return Ok(filtered);
      }
      b"PLTE" => {
        if saw_pixels
          || palette_entries != 0
          || matches!(color, 0 | 4)
          || length == 0
          || length > 768
          || !length.is_multiple_of(3)
        {
          return Err(invalid());
        }
        palette_entries = length / 3;
      }
      b"tRNS" => {
        let valid_length = match color {
          0 => length == 2,
          2 => length == 6,
          3 => length > 0 && length <= palette_entries,
          _ => false,
        };
        if saw_pixels || saw_transparency || !valid_length {
          return Err(invalid());
        }
        saw_transparency = true;
      }
      _ if kind[0] & 0x20 == 0 => {
        return Err("Unsupported PNG image extension.".into());
      }
      _ => {
        offset = end;
        continue;
      }
    }
    filtered.extend_from_slice(chunk);
    offset = end;
  }
  Err(invalid())
}

fn decode(bytes: &[u8], cancelled: &AtomicBool) -> Result<DecodedImage, String> {
  check_cancelled(cancelled)?;
  if bytes.len() > MAX_DOWNLOAD_BYTES {
    return Err("Image download exceeds the 4 MiB limit.".into());
  }
  if bytes.starts_with(PNG_SIGNATURE) {
    let filtered = png_pixels_only(bytes)?;
    let mut decoder =
      zune_png::PngDecoder::new_with_options(ZCursor::new(&filtered), decoder_options());
    decoder.decode_headers().map_err(|_| String::from("Cannot decode this PNG image."))?;
    let (width, height) = decoder.dimensions().ok_or("PNG image has no dimensions.")?;
    dimensions(width, height)?;
    let colorspace = decoder.colorspace().ok_or("PNG image has no color format.")?;
    let size = decoder.output_buffer_size().ok_or("PNG image is too large.")?;
    let mut pixels = pixel_buffer(size)?;
    check_cancelled(cancelled)?;
    decoder.decode_into(&mut pixels).map_err(|_| String::from("Cannot decode this PNG image."))?;
    // Release inflater, metadata and compressed copies before output conversion.
    drop(decoder);
    drop(filtered);
    resize_rgba(pixels, width, height, colorspace, cancelled)
  } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
    let mut decoder =
      zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), decoder_options());
    decoder.decode_headers().map_err(|_| String::from("Cannot decode this JPEG image."))?;
    let info = decoder.info().ok_or("JPEG image has no dimensions.")?;
    let (width, height) = (usize::from(info.width), usize::from(info.height));
    dimensions(width, height)?;
    if !matches!(info.components, 1 | 3 | 4) {
      return Err("Unsupported JPEG color format.".into());
    }
    let size = decoder.output_buffer_size().ok_or("JPEG image is too large.")?;
    let mut pixels = pixel_buffer(size)?;
    check_cancelled(cancelled)?;
    decoder.decode_into(&mut pixels).map_err(|_| String::from("Cannot decode this JPEG image."))?;
    drop(decoder);
    resize_rgba(pixels, width, height, ColorSpace::RGBA, cancelled)
  } else {
    Err("Unsupported image format. Use PNG or JPEG.".into())
  }
}

fn pixel_buffer(size: usize) -> Result<Vec<u8>, String> {
  if size > MAX_SOURCE_PIXELS * 4 {
    return Err("Decoded image exceeds the memory limit.".into());
  }
  let mut pixels = Vec::new();
  pixels
    .try_reserve_exact(size)
    .map_err(|_| String::from("Not enough memory to decode the image."))?;
  pixels.resize(size, 0);
  Ok(pixels)
}

fn resize_rgba(
  pixels: Vec<u8>,
  width: usize,
  height: usize,
  colorspace: ColorSpace,
  cancelled: &AtomicBool,
) -> Result<DecodedImage, String> {
  check_cancelled(cancelled)?;
  let components = match colorspace {
    ColorSpace::Luma => 1,
    ColorSpace::LumaA => 2,
    ColorSpace::RGB => 3,
    ColorSpace::RGBA => 4,
    _ => return Err("Unsupported image color format.".into()),
  };
  if pixels.len() != width * height * components {
    return Err("Image pixels do not match its dimensions.".into());
  }
  let largest = width.max(height).max(MAX_OUTPUT_AXIS);
  let output_width = (width * MAX_OUTPUT_AXIS / largest).max(1);
  let output_height = (height * MAX_OUTPUT_AXIS / largest).max(1);
  if width == output_width && height == output_height && colorspace == ColorSpace::RGBA {
    return Ok(DecodedImage { width: width as u32, height: height as u32, rgba: pixels });
  }
  let mut rgba = pixel_buffer(output_width * output_height * 4)?;
  // Area averaging avoids aliasing in downscaled screenshots/diagrams, with
  // alpha-weighted color so transparent pixels cannot create dark fringes.
  for y in 0..output_height {
    check_cancelled(cancelled)?;
    let top = y * height / output_height;
    let bottom = ((y + 1) * height / output_height).max(top + 1);
    for x in 0..output_width {
      let left = x * width / output_width;
      let right = ((x + 1) * width / output_width).max(left + 1);
      let mut sums = [0u64; 4];
      let count = ((bottom - top) * (right - left)) as u64;
      for source_y in top..bottom {
        for source_x in left..right {
          let start = (source_y * width + source_x) * components;
          let source = &pixels[start..start + components];
          let pixel = match colorspace {
            ColorSpace::Luma => [source[0], source[0], source[0], 255],
            ColorSpace::LumaA => [source[0], source[0], source[0], source[1]],
            ColorSpace::RGB => [source[0], source[1], source[2], 255],
            ColorSpace::RGBA => [source[0], source[1], source[2], source[3]],
            _ => unreachable!(),
          };
          let alpha = u64::from(pixel[3]);
          for channel in 0..3 {
            sums[channel] += u64::from(pixel[channel]) * alpha;
          }
          sums[3] += alpha;
        }
      }
      let target = (y * output_width + x) * 4;
      for channel in 0..3 {
        rgba[target + channel] = sums[channel].checked_div(sums[3]).unwrap_or(0) as u8;
      }
      rgba[target + 3] = (sums[3] / count) as u8;
    }
  }
  Ok(DecodedImage { width: output_width as u32, height: output_height as u32, rgba })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::backend::{
    http::{HttpApi, Progress, Receive, UrlPart},
    runtime::tests::runtime,
  };
  use alloc::{boxed::Box, collections::BTreeMap};
  use core::ffi::{c_char, c_void, CStr};
  use std::time::{Duration, Instant};

  const PNG: &[u8] = include_bytes!("../../../tests/fixtures/image-quadrants.png");
  const JPEG: &[u8] = include_bytes!("../../../tests/fixtures/image-quadrants.jpg");
  const PROGRESSIVE: &[u8] = include_bytes!("../../../tests/fixtures/image-progressive.jpg");
  const GRAY_JPEG: &[u8] = include_bytes!("../../../tests/fixtures/image-gray.jpg");
  const GRAY: &[u8] = include_bytes!("../../../tests/fixtures/image-gray.png");
  const INDEXED: &[u8] = include_bytes!("../../../tests/fixtures/image-indexed.png");

  fn decode_image(bytes: &[u8]) -> Result<DecodedImage, String> {
    decode(bytes, &AtomicBool::new(false))
  }

  #[test]
  fn png_jpeg_gray_and_palette_preserve_colors_and_transparency() {
    let png = decode_image(PNG).unwrap();
    assert_eq!((png.width, png.height, png.rgba.len()), (256, 128, 256 * 128 * 4));
    assert_eq!(&png.rgba[..4], &[242, 30, 42, 255]);
    let transparent = ((64 * 256 + 128) * 4) as usize;
    assert_eq!(png.rgba[transparent + 3], 0);
    let jpeg = decode_image(JPEG).unwrap();
    assert_eq!((jpeg.width, jpeg.height, jpeg.rgba.len()), (256, 128, png.rgba.len()));
    assert!(jpeg.rgba[0] > 200 && jpeg.rgba[1] < 60 && jpeg.rgba[2] < 60);
    assert_eq!(jpeg.rgba[3], 255);
    let gray = decode_image(GRAY).unwrap();
    assert_eq!(&gray.rgba[..4], &[112, 112, 112, 255]);
    let indexed = decode_image(INDEXED).unwrap();
    assert_eq!(&indexed.rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(indexed.rgba[7], 0);
    assert_eq!(decode_image(PROGRESSIVE).unwrap().rgba, jpeg.rgba);
    assert_eq!(&decode_image(GRAY_JPEG).unwrap().rgba[..4], &[112, 112, 112, 255]);
  }

  #[test]
  fn dimensions_and_bit_depth_are_rejected_before_pixel_decoding() {
    for (width, height) in [(0u32, 1u32), (4097, 1), (2048, 1025), (u32::MAX, u32::MAX)] {
      let mut bytes = PNG.to_vec();
      bytes[16..20].copy_from_slice(&width.to_be_bytes());
      bytes[20..24].copy_from_slice(&height.to_be_bytes());
      assert!(decode_image(&bytes).unwrap_err().contains("2-megapixel"));
    }
    let mut bytes = PNG.to_vec();
    bytes[24] = 16;
    assert!(decode_image(&bytes).unwrap_err().contains("16-bit"));
    assert!(dimensions(1920, 1080).is_ok());
    assert!(dimensions(4096, 512).is_ok());
  }

  #[test]
  fn malformed_unsupported_and_oversized_inputs_fail_without_panicking() {
    for bytes in [&b""[..], b"GIF89a", b"<svg/>", b"\xff\xd8\xff", &PNG[..PNG.len() - 1]] {
      assert!(decode_image(bytes).is_err());
    }
    let mut corrupt = PNG.to_vec();
    corrupt[29] ^= 0xff;
    assert!(decode_image(&corrupt).is_err());
    assert!(decode_image(&vec![0; MAX_DOWNLOAD_BYTES + 1]).unwrap_err().contains("4 MiB"));
    for end in 0..PNG.len() {
      assert!(decode_image(&PNG[..end]).is_err());
    }
    // Real-world truncated/malformed JPEG inputs exercise headers and entropy
    // decoding, including paths not reached by a signature-only invalid file.
    for end in (0..JPEG.len()).step_by(17) {
      let _ = decode_image(&JPEG[..end]);
    }
    for index in (0..JPEG.len()).step_by(11) {
      let mut mutated = JPEG.to_vec();
      mutated[index] ^= 0xff;
      let _ = decode_image(&mutated);
    }
  }

  #[test]
  fn ancillary_compressed_metadata_never_reaches_the_png_decoder() {
    // Deliberately not valid zlib data or CRC: ignored ancillary metadata must
    // not be decompressed (or interpreted) by an image-only preview.
    let mut with_metadata = PNG[..33].to_vec();
    for kind in [b"iCCP", b"zTXt", b"iTXt", b"eXIf", b"acTL", b"fcTL", b"fdAT"] {
      with_metadata.extend_from_slice(&8u32.to_be_bytes());
      with_metadata.extend_from_slice(kind);
      with_metadata.extend_from_slice(b"metadata");
      with_metadata.extend_from_slice(&0u32.to_be_bytes());
    }
    with_metadata.extend_from_slice(&PNG[33..]);
    assert_eq!(png_pixels_only(&with_metadata).unwrap(), PNG);
    assert_eq!(decode_image(&with_metadata).unwrap().rgba, decode_image(PNG).unwrap().rgba);
    with_metadata[37..41].copy_from_slice(b"ZTXT");
    assert!(decode_image(&with_metadata).unwrap_err().contains("extension"));
  }

  #[test]
  fn malformed_palette_and_transparency_cannot_desynchronize_chunk_filtering() {
    for (color, kind, length) in [
      (2, b"PLTE", 0usize),
      (2, b"PLTE", 4),
      (2, b"PLTE", 771),
      (0, b"tRNS", 3),
      (2, b"tRNS", 7),
      (3, b"tRNS", 1),
      (6, b"tRNS", 2),
    ] {
      let mut bytes = PNG[..33].to_vec();
      bytes[25] = color;
      bytes.extend_from_slice(&(length as u32).to_be_bytes());
      bytes.extend_from_slice(kind);
      bytes.extend_from_slice(&vec![0; length + 4]);
      bytes.extend_from_slice(&PNG[33..]);
      assert!(png_pixels_only(&bytes).is_err(), "invalid {kind:?} length {length}");
    }
    let mut bytes = PNG.to_vec();
    bytes[24] = 1;
    bytes[25] = 2;
    assert!(png_pixels_only(&bytes).is_err(), "RGB PNG requires at least 8-bit samples");
  }

  #[test]
  fn resize_bounds_preserve_aspect_ratio_and_alpha_weight_colors() {
    let cancelled = AtomicBool::new(false);
    let mut pixels = vec![0; 2048 * 4];
    for x in (0..2048).step_by(2) {
      pixels[x * 4..x * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    let image = resize_rgba(pixels, 2048, 1, ColorSpace::RGBA, &cancelled).unwrap();
    assert_eq!((image.width, image.height), (1024, 1));
    assert_eq!(&image.rgba[..4], &[255, 0, 0, 127]);
    let image =
      resize_rgba(vec![77; 1920 * 1080], 1920, 1080, ColorSpace::Luma, &cancelled).unwrap();
    assert_eq!((image.width, image.height), (1024, 576));
    assert_eq!(&image.rgba[..4], &[77, 77, 77, 255]);
    cancelled.store(true, Ordering::Release);
    assert!(decode(PNG, &cancelled).unwrap_err().contains("cancelled"));
  }

  #[derive(Default)]
  struct Fake {
    strings: BTreeMap<u32, String>,
    numbers: BTreeMap<u32, i64>,
    headers: Vec<String>,
    context: *mut c_void,
    receive: Option<Receive>,
    progress: Option<Progress>,
    status: u16,
  }
  unsafe extern "C" fn init() -> bool {
    true
  }
  unsafe extern "C" fn create() -> *mut c_void {
    Box::into_raw(Box::new(Fake::default())).cast()
  }
  unsafe extern "C" fn destroy(handle: *mut c_void) {
    unsafe { drop(Box::<Fake>::from_raw(handle.cast())) };
  }
  unsafe extern "C" fn string(handle: *mut c_void, id: u32, value: *const c_char) -> bool {
    unsafe { &mut *handle.cast::<Fake>() }
      .strings
      .insert(id, unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned());
    true
  }
  unsafe extern "C" fn number(handle: *mut c_void, id: u32, value: i64) -> bool {
    unsafe { &mut *handle.cast::<Fake>() }.numbers.insert(id, value);
    true
  }
  unsafe extern "C" fn body(_: *mut c_void, _: *const u8, _: usize) -> bool {
    false
  }
  unsafe extern "C" fn header(handle: *mut c_void, value: *const c_char) -> bool {
    unsafe { &mut *handle.cast::<Fake>() }
      .headers
      .push(unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned());
    true
  }
  unsafe extern "C" fn callbacks(
    handle: *mut c_void,
    context: *mut c_void,
    receive: Receive,
    _: Receive,
    progress: Progress,
  ) -> bool {
    let fake = unsafe { &mut *handle.cast::<Fake>() };
    fake.context = context;
    fake.receive = Some(receive);
    fake.progress = Some(progress);
    true
  }
  unsafe extern "C" fn perform(handle: *mut c_void) -> u32 {
    let fake = unsafe { &mut *handle.cast::<Fake>() };
    if fake.strings.get(&5).map(String::as_str) != Some("https")
      || fake.headers != ["Accept: image/png, image/jpeg"]
      || fake.numbers.get(&2) != Some(&0)
      || fake.numbers.get(&3) != Some(&1)
      || fake.numbers.get(&4) != Some(&2)
      || fake.strings.get(&2).map(String::as_str) != Some("/test/ca.pem")
    {
      return 43;
    }
    let url = fake.strings.get(&1).unwrap();
    fake.status = if url.ends_with("denied") {
      403
    } else if url.ends_with("redirect") {
      302
    } else {
      200
    };
    let (context, receive, progress) =
      (fake.context, fake.receive.unwrap(), fake.progress.unwrap());
    if url.ends_with("slow") {
      let deadline = Instant::now() + Duration::from_secs(2);
      while Instant::now() < deadline {
        if unsafe { progress(context) } {
          return 42;
        }
        std::thread::yield_now();
      }
      return 28;
    }
    let large = vec![0; 64 * 1024];
    let (data, count): (&[u8], usize) = if url.ends_with("oversize") {
      (&large, 65)
    } else if url.ends_with("invalid") {
      (b"not an image", 1)
    } else {
      (PNG, 1)
    };
    for _ in 0..count {
      if unsafe { receive(context, data.as_ptr(), data.len()) } != data.len() {
        return 23;
      }
    }
    0
  }
  unsafe extern "C" fn status(handle: *mut c_void) -> u16 {
    unsafe { (*handle.cast::<Fake>()).status }
  }
  unsafe extern "C" fn parse(_: *const c_char, value: *const c_char) -> *mut c_void {
    let text = unsafe { CStr::from_ptr(value) }.to_str().unwrap();
    let Some((scheme, rest)) = text.split_once("://") else { return core::ptr::null_mut() };
    let authority = rest.split('/').next().unwrap_or("");
    Box::into_raw(Box::new(UrlParts {
      scheme: scheme.into(),
      host: authority.into(),
      has_user: authority.contains('@'),
      has_password: authority.contains('@'),
      ..UrlParts::default()
    }))
    .cast()
  }
  unsafe extern "C" fn part(handle: *mut c_void, id: u32) -> UrlPart {
    let value = unsafe { &*handle.cast::<UrlParts>() };
    let text = match id {
      1 => &value.scheme,
      2 => &value.host,
      _ => "",
    };
    UrlPart {
      data: text.as_ptr(),
      len: text.len(),
      present: match id {
        6 => value.has_user,
        7 => value.has_password,
        _ => !text.is_empty(),
      },
    }
  }
  unsafe extern "C" fn free(handle: *mut c_void) {
    unsafe { drop(Box::<UrlParts>::from_raw(handle.cast())) };
  }
  fn http() -> Http {
    unsafe {
      Http::new(&HttpApi {
        init,
        create,
        destroy,
        set_string: string,
        set_number: number,
        set_body: body,
        append_header: header,
        callbacks,
        perform,
        status,
        url_parse: parse,
        url_part: part,
        url_free: free,
      })
      .unwrap()
    }
  }
  fn loader() -> ImageLoader {
    ImageLoader::new(runtime(), http(), "/test/ca.pem".into()).unwrap()
  }
  fn wait(loader: &mut ImageLoader) -> ImageResult {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
      if let Some(result) = loader.poll() {
        return result;
      }
      assert!(Instant::now() < deadline, "image worker did not finish");
      std::thread::yield_now();
    }
  }

  #[test]
  fn worker_is_single_slot_and_download_is_credential_free() {
    let mut loader = loader();
    let url = "https://image.test/picture?signature=public-token";
    assert!(loader.request(url));
    assert!(!loader.request("https://image.test/other"));
    let result = wait(&mut loader);
    assert_eq!(result.url, url);
    assert_eq!(result.result.unwrap().width, 256);
    assert!(!loader.is_running());
    assert!(loader.request("https://image.test/other"));
    let deadline = Instant::now() + Duration::from_secs(3);
    while loader.is_running() {
      assert!(Instant::now() < deadline);
      std::thread::yield_now();
    }
    assert!(!loader.request(url), "unpolled image must occupy the only slot");
    assert!(loader.poll().unwrap().result.is_ok());
  }

  #[test]
  fn invalid_urls_and_worker_failures_resolve_to_pollable_errors() {
    let mut loader = loader();
    for url in [
      "http://127.0.0.1/image",
      "file:///secret",
      "data:image/png;base64,ABC",
      "https://user:password@image.test/picture",
      "https://image.test/back\\slash",
      "https://image.test/new\nline",
      "https:///picture",
    ] {
      assert!(loader.request(url));
      let result = wait(&mut loader);
      assert_eq!(result.url, url);
      assert!(result.result.is_err());
      assert!(!loader.is_running());
    }
    let too_long = format!("https://image.test/{}", "x".repeat(MAX_URL_BYTES));
    assert!(loader.request(&too_long));
    let result = wait(&mut loader);
    assert_eq!(result.url, too_long);
    assert!(result.result.unwrap_err().contains("4096"));
    for (path, expected) in [
      ("denied", "HTTP 403"),
      ("redirect", "redirect"),
      ("oversize", "4 MiB"),
      ("invalid", "PNG or JPEG"),
    ] {
      assert!(loader.request(&format!("https://image.test/{path}")));
      assert!(wait(&mut loader).result.unwrap_err().contains(expected));
    }
  }

  #[test]
  fn cancellation_returns_a_result_and_drop_joins_the_worker() {
    let mut loader = loader();
    assert!(loader.request("https://image.test/slow"));
    loader.cancel();
    assert!(wait(&mut loader).result.unwrap_err().contains("cancelled"));
    assert!(loader.request("https://image.test/picture"));
    assert!(wait(&mut loader).result.is_ok());
    assert!(loader.request("https://image.test/slow"));
    let started = Instant::now();
    drop(loader);
    assert!(started.elapsed() < Duration::from_secs(1));
  }
}
