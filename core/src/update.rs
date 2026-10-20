use alloc::{format, string::String, vec::Vec};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const MAX_RELEASE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_VERSION_BYTES: usize = 64;
pub(crate) const MAX_NRO_BYTES: u64 = 128 * 1024 * 1024;
const NRO_HEADER_BYTES: u64 = 0x80;
const ASSET_HEADER_BYTES: u64 = 0x38;

#[derive(Deserialize)]
struct GithubRelease {
  tag_name: String,
  draft: bool,
  prerelease: bool,
  assets: Vec<GithubAsset>,
  body: Option<String>,
}

#[derive(Deserialize)]
struct GithubAsset {
  name: String,
  state: String,
  size: u64,
  browser_download_url: String,
  digest: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);

impl Version {
  fn parse(text: &str) -> Option<Self> {
    if text.len() > MAX_VERSION_BYTES {
      return None;
    }
    let mut components = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut number = || {
      let part = components.next()?;
      if part.is_empty()
        || (part.len() > 1 && part.starts_with('0'))
        || !part.bytes().all(|byte| byte.is_ascii_digit())
      {
        return None;
      }
      part.parse::<u64>().ok()
    };
    let version = Self(number()?, number()?, number()?);
    if components.next().is_some() {
      None
    } else {
      Some(version)
    }
  }

  fn normalized(self) -> String {
    format!("{}.{}.{}", self.0, self.1, self.2)
  }
}

pub(crate) fn valid_repository(repository: &str) -> bool {
  let Some((owner, repo)) = repository.split_once('/') else { return false };
  !owner.is_empty()
    && owner.len() <= 39
    && owner.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    && owner.as_bytes()[0].is_ascii_alphanumeric()
    && owner.as_bytes()[owner.len() - 1].is_ascii_alphanumeric()
    && !owner.contains("--")
    && !repo.is_empty()
    && repo.len() <= 100
    && repo.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    && !matches!(repo, "." | "..")
    && !repo.contains("..")
}

pub(crate) fn inspect(release_json: &str, current_version: &str) -> Value {
  match inspect_release(release_json, current_version) {
    Ok(value) => value,
    Err(error) => json!({"ok":false,"available":false,"error":error}),
  }
}

fn inspect_release(release_json: &str, current_version: &str) -> Result<Value, &'static str> {
  if release_json.len() > MAX_RELEASE_BYTES {
    return Err("The release metadata exceeds the 1 MiB limit.");
  }
  let current = Version::parse(current_version).ok_or("The installed version is invalid.")?;
  let release: GithubRelease = serde_json::from_str(release_json)
    .map_err(|_| "The update source returned invalid release metadata.")?;
  if release.draft || release.prerelease {
    return Err("The update source did not return a stable release.");
  }
  let version = Version::parse(&release.tag_name)
    .ok_or("The release tag must be a stable major.minor.patch version.")?;
  let mut candidates = release.assets.iter().filter(|asset| asset.name == "wodex.nro");
  let asset = candidates.next().ok_or("The release does not contain wodex.nro.")?;
  if candidates.next().is_some() {
    return Err("The release contains ambiguous wodex.nro assets.");
  }
  if asset.state != "uploaded" || !(NRO_HEADER_BYTES..=MAX_NRO_BYTES).contains(&asset.size) {
    return Err("The release asset is incomplete or its size is invalid.");
  }
  if !valid_download_url(&asset.browser_download_url, &release.tag_name) {
    return Err("The release asset must use its public GitHub release download URL.");
  }
  let digest = asset
    .digest
    .as_deref()
    .and_then(|value| value.strip_prefix("sha256:"))
    .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .ok_or("The release asset is missing a valid SHA-256 digest.")?;
  if version <= current {
    return Ok(json!({"ok":true,"available":false}));
  }
  let notes: String = release
    .body
    .unwrap_or_default()
    .chars()
    .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    .take(4096)
    .collect();
  Ok(json!({"ok":true,"available":true,"release":{
    "version":version.normalized(),"download_url":asset.browser_download_url,
    "size":asset.size,"sha256":digest.to_ascii_lowercase(),"notes":notes
  }}))
}

fn valid_download_url(url: &str, tag: &str) -> bool {
  if url.len() > 2048 || !url.is_ascii() {
    return false;
  }
  let Some(path) = url.strip_prefix("https://github.com/") else { return false };
  let mut parts = path.split('/');
  let Some(owner) = parts.next() else { return false };
  let Some(repo) = parts.next() else { return false };
  if !valid_repository(&format!("{owner}/{repo}"))
    || parts.next() != Some("releases")
    || parts.next() != Some("download")
  {
    return false;
  }
  let Some(encoded_tag) = parts.next() else { return false };
  if parts.next() != Some("wodex.nro") || parts.next().is_some() {
    return false;
  }
  decode_tag(encoded_tag).as_deref() == Some(tag)
}

fn decode_tag(tag: &str) -> Option<String> {
  if tag.len() > MAX_VERSION_BYTES * 3 {
    return None;
  }
  let mut bytes = tag.bytes();
  let mut decoded = Vec::new();
  while let Some(byte) = bytes.next() {
    if byte == b'%' {
      let high = (bytes.next()? as char).to_digit(16)? as u8;
      let low = (bytes.next()? as char).to_digit(16)? as u8;
      decoded.push((high << 4) | low);
    } else {
      decoded.push(byte);
    }
  }
  String::from_utf8(decoded).ok()
}

/// Incremental hashing is independent from release parsing and never reads an
/// entire downloaded binary into memory. Use the software backend consistently
/// on Switch; CPU-feature probes on freestanding Horizon are unavailable.
pub struct WodexSha256 {
  hasher: Option<Sha256>,
  digest: Option<[u8; 32]>,
  bytes: u64,
}

impl WodexSha256 {
  pub fn new() -> Self {
    Self { hasher: Some(Sha256::new()), digest: None, bytes: 0 }
  }

  pub fn update(&mut self, bytes: &[u8]) -> bool {
    if self.bytes + bytes.len() as u64 > MAX_NRO_BYTES {
      return false;
    }
    let Some(hasher) = self.hasher.as_mut() else { return false };
    hasher.update(bytes);
    self.bytes += bytes.len() as u64;
    true
  }

  pub fn finish(&mut self) -> String {
    if let Some(hasher) = self.hasher.take() {
      self.digest = Some(hasher.finalize().into());
    }
    let mut hex = String::with_capacity(64);
    for byte in self.digest.as_ref().expect("finalized digest") {
      hex.push(b"0123456789abcdef"[(byte >> 4) as usize] as char);
      hex.push(b"0123456789abcdef"[(byte & 15) as usize] as char);
    }
    hex
  }
}

impl Default for WodexSha256 {
  fn default() -> Self {
    Self::new()
  }
}

/// The field offsets and relative asset sections follow libnx's public nro.h.
/// A caller supplying only an initial prefix must separately seek to code_size
/// and validate the ASET header there; those bytes cannot be inferred here.
pub(crate) fn validate_nro(header: &[u8], file_size: u64) -> bool {
  if !(NRO_HEADER_BYTES..=MAX_NRO_BYTES).contains(&file_size)
    || header.len() < NRO_HEADER_BYTES as usize
    || header.len() as u64 > file_size
    || &header[0x10..0x14] != b"NRO0"
  {
    return false;
  }
  let read_u32 = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap()) as u64;
  let code_size = read_u32(0x18);
  if !(NRO_HEADER_BYTES..=file_size).contains(&code_size) {
    return false;
  }
  let mut previous_end = 0;
  for segment in 0..3 {
    let offset = read_u32(0x20 + segment * 8);
    let size = read_u32(0x24 + segment * 8);
    if offset < previous_end
      || offset > code_size
      || size > code_size - offset
      || (segment == 0 && (offset != 0 || size < NRO_HEADER_BYTES))
    {
      return false;
    }
    previous_end = offset + size;
  }
  if previous_end != code_size {
    return false;
  }
  if code_size == file_size {
    return true;
  }
  let asset_size = file_size - code_size;
  if asset_size < ASSET_HEADER_BYTES {
    return false;
  }
  if header.len() as u64 >= code_size + ASSET_HEADER_BYTES {
    return validate_assets(&header[code_size as usize..], asset_size);
  }
  true
}

fn validate_assets(header: &[u8], asset_size: u64) -> bool {
  if &header[..4] != b"ASET" || header[4..8] != [0; 4] {
    return false;
  }
  let read_u64 = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
  let mut ranges = Vec::new();
  for section in 0..3 {
    let offset = read_u64(8 + section * 16);
    let size = read_u64(16 + section * 16);
    if size == 0 {
      continue;
    }
    if offset < ASSET_HEADER_BYTES
      || offset > asset_size
      || size > asset_size - offset
      || ranges.iter().any(|&(start, end)| offset < end && start < offset + size)
    {
      return false;
    }
    ranges.push((offset, offset + size));
  }
  true
}

#[cfg(test)]
mod tests {
  use super::*;
  use alloc::vec;

  fn release(tag: &str) -> Value {
    json!({
      "tag_name":tag,"draft":false,"prerelease":false,"body":"New release\nWith improvements",
      "assets":[{
        "name":"wodex.nro","state":"uploaded","size":26_000_000,
        "browser_download_url":format!("https://github.com/example/wodex/releases/download/{tag}/wodex.nro"),
        "digest":format!("sha256:{}","ab".repeat(32))
      }]
    })
  }

  fn inspect_value(release: &Value, current: &str) -> Value {
    inspect(&release.to_string(), current)
  }

  #[test]
  fn stable_versions_compare_numerically_and_normalize() {
    let result = inspect_value(&release("v0.10.0"), "0.9.99");
    assert_eq!(result["ok"], true);
    assert_eq!(result["available"], true);
    assert_eq!(result["release"]["version"], "0.10.0");
    assert_eq!(result["release"]["sha256"], "ab".repeat(32));
    assert_eq!(result["release"]["notes"], "New release\nWith improvements");
    for (tag, current) in
      [("v0.2.0", "0.2.0"), ("0.2.0", "v0.3.0"), ("1.2.99", "1.10.0"), ("9.999.999", "10.0.0")]
    {
      assert_eq!(inspect_value(&release(tag), current), json!({"ok":true,"available":false}));
    }
    for tag in [
      "v1.2.0-rc1",
      "1.2.0+metadata",
      "1.02.0",
      "V1.2.0",
      "1.2",
      "1.2.3.4",
      "-1.2.3",
      " 1.2.3",
      "1.2.18446744073709551616",
    ] {
      assert_eq!(inspect_value(&release(tag), "0.1.0")["ok"], false, "{tag}");
    }
    for current in ["", "garbage", "v1.2.3-rc1", "1.2"] {
      assert_eq!(inspect_value(&release("1.2.3"), current)["ok"], false);
    }
  }

  #[test]
  fn rejects_unstable_incomplete_unsigned_and_ambiguous_releases() {
    for field in ["draft", "prerelease"] {
      let mut metadata = release("0.2.0");
      metadata[field] = json!(true);
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false);
    }
    for (field, value) in [
      ("name", json!("wodex.zip")),
      ("state", json!("starter")),
      ("size", json!(127)),
      ("size", json!(MAX_NRO_BYTES + 1)),
      ("size", json!(-1)),
      ("size", json!("26000000")),
      ("digest", Value::Null),
      ("digest", json!("sha256:bad")),
      ("digest", json!("md5:bad")),
      ("digest", json!(format!("sha256:{}", "zz".repeat(32)))),
    ] {
      let mut metadata = release("0.2.0");
      metadata["assets"][0][field] = value;
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{field}");
    }
    let mut metadata = release("0.2.0");
    metadata["assets"][0].as_object_mut().unwrap().remove("digest");
    assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false);
    metadata["assets"] = json!([]);
    assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false);
    let mut metadata = release("0.2.0");
    let duplicate = metadata["assets"][0].clone();
    metadata["assets"].as_array_mut().unwrap().push(duplicate);
    assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false);
    assert_eq!(inspect("{\"message\":\"secret user error\"}", "0.1.0")["ok"], false);
    assert!(!inspect("{\"message\":\"secret user error\"}", "0.1.0")
      .to_string()
      .contains("secret user error"));
    assert_eq!(inspect(&"x".repeat(MAX_RELEASE_BYTES + 1), "0.1.0")["ok"], false);
  }

  #[test]
  fn release_download_urls_are_confined_to_github_assets_for_the_same_tag() {
    for url in [
      "http://github.com/example/wodex/releases/download/0.2.0/wodex.nro",
      "https://github.com.evil.test/example/wodex/releases/download/0.2.0/wodex.nro",
      "https://github.com@evil.test/example/wodex/releases/download/0.2.0/wodex.nro",
      "https://user@github.com/example/wodex/releases/download/0.2.0/wodex.nro",
      "https://github.com:443/example/wodex/releases/download/0.2.0/wodex.nro",
      "https://github.com/example/wodex/releases/download/0.1.0/wodex.nro",
      "https://github.com/example/wodex/releases/download/0.2.0/wodex.nro?token=secret",
      "https://github.com/example/wodex/releases/download/0.2.0/wodex.nro#fragment",
      "https://github.com/example/wodex/releases/download/0.2.0/other.nro",
      "https://github.com/example/../releases/download/0.2.0/wodex.nro",
      "https://github.com/example/wodex/releases/download/%ZZ/wodex.nro",
      "https://github.com/example/wodex/releases/download/%2F0.2.0/wodex.nro",
    ] {
      let mut metadata = release("0.2.0");
      metadata["assets"][0]["browser_download_url"] = json!(url);
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{url}");
    }
    let mut metadata = release("v0.2.0");
    metadata["assets"][0]["browser_download_url"] =
      json!("https://github.com/example/wodex/releases/download/%760.2.0/wodex.nro");
    metadata["assets"][0]["digest"] = json!(format!("sha256:{}", "AB".repeat(32)));
    assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], true);
    assert_eq!(inspect_value(&metadata, "0.1.0")["release"]["sha256"], "ab".repeat(32));
  }

  #[test]
  fn repository_names_and_notes_are_bounded() {
    for repository in ["example/wodex", "switch-org/wodex_app", "example/.github", "a/b"] {
      assert!(valid_repository(repository));
    }
    for repository in [
      "",
      "owner",
      "owner/repo/extra",
      "https://github.com/owner/repo",
      "../repo",
      "owner/..",
      "owner/a..b",
      "owner/.",
      "-owner/repo",
      "owner-/repo",
      "own--er/repo",
      "owner/repo\n",
      "用户/repo",
      "owner/repo?token=x",
    ] {
      assert!(!valid_repository(repository), "{repository}");
    }
    assert!(!valid_repository(&format!("{}/repo", "a".repeat(40))));
    assert!(!valid_repository(&format!("owner/{}", "a".repeat(101))));
    let mut metadata = release("0.2.0");
    metadata["body"] = json!(format!("\u{0001}{}", "文".repeat(5000)));
    let response = inspect_value(&metadata, "0.1.0");
    assert_eq!(response["release"]["notes"].as_str().unwrap().chars().count(), 4096);
    assert!(!response["release"]["notes"].as_str().unwrap().contains('\u{0001}'));
  }

  #[test]
  fn sha256_matches_known_vectors_under_fragmentation() {
    for (input, expected) in [
      ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
      ("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
      (
        "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
      ),
    ] {
      for split in 0..=input.len() {
        let mut hash = WodexSha256::new();
        assert!(hash.update(&input.as_bytes()[..split]));
        assert!(hash.update(&input.as_bytes()[split..]));
        assert_eq!(hash.finish(), expected);
        assert_eq!(hash.finish(), expected);
        assert!(!hash.update(b"after finish"));
      }
    }
    let mut hash = WodexSha256::new();
    let chunk = vec![b'a'; 1000];
    for _ in 0..1000 {
      assert!(hash.update(&chunk));
    }
    assert_eq!(hash.finish(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
  }

  fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
  }

  fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
  }

  fn nro_prefix() -> Vec<u8> {
    let mut bytes = vec![0; 256];
    bytes[0x10..0x14].copy_from_slice(b"NRO0");
    put_u32(&mut bytes, 0x18, 0x3000);
    for (offset, value) in
      [(0x20, 0), (0x24, 0x1000), (0x28, 0x1000), (0x2c, 0x1000), (0x30, 0x2000), (0x34, 0x1000)]
    {
      put_u32(&mut bytes, offset, value);
    }
    bytes
  }

  #[test]
  fn nro_header_checks_magic_segment_bounds_and_truncation() {
    assert!(validate_nro(&nro_prefix(), 0x3000));
    assert!(validate_nro(&nro_prefix(), 0x3100));
    assert!(!validate_nro(&nro_prefix()[..127], 0x3000));
    assert!(!validate_nro(&nro_prefix(), 127));
    assert!(!validate_nro(&nro_prefix(), MAX_NRO_BYTES + 1));
    assert!(!validate_nro(&nro_prefix(), 0x3037));
    for (offset, value) in [
      (0x10, 0),
      (0x18, 0x7f),
      (0x18, 0x4000),
      (0x20, 4),
      (0x24, 127),
      (0x28, 0xff0),
      (0x2c, 0x3000),
      (0x30, u32::MAX),
      (0x34, u32::MAX),
      (0x34, 0xff0),
    ] {
      let mut header = nro_prefix();
      put_u32(&mut header, offset, value);
      assert!(!validate_nro(&header, 0x3000), "{offset:x}={value:x}");
    }
  }

  #[test]
  fn nro_asset_header_rejects_overflow_overlap_and_unsupported_layouts() {
    let mut file = nro_prefix();
    file.resize(0x3100, 0);
    file[0x3000..0x3004].copy_from_slice(b"ASET");
    for (offset, value) in [(8, 56), (16, 16), (24, 72), (32, 32), (40, 104), (48, 152)] {
      put_u64(&mut file, 0x3000 + offset, value);
    }
    assert!(validate_nro(&file, file.len() as u64));
    for (offset, value) in [(8, 55), (8, u64::MAX), (16, u64::MAX), (24, 64), (40, 256), (48, 153)]
    {
      let mut invalid = file.clone();
      put_u64(&mut invalid, 0x3000 + offset, value);
      assert!(!validate_nro(&invalid, invalid.len() as u64), "{offset}={value}");
    }
    let mut invalid = file.clone();
    put_u32(&mut invalid, 0x3004, 1);
    assert!(!validate_nro(&invalid, invalid.len() as u64));
    invalid[0x3000] = b'X';
    assert!(!validate_nro(&invalid, invalid.len() as u64));
  }
}
