use alloc::{format, string::String, vec::Vec};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const MAX_UPDATE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_VERSION_BYTES: usize = 64;
pub(crate) const MAX_NRO_BYTES: u64 = 128 * 1024 * 1024;
const NRO_HEADER_BYTES: u64 = 0x80;
const ASSET_HEADER_BYTES: u64 = 0x38;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateFeed {
  schema_version: u32,
  channel: String,
  #[serde(deserialize_with = "Option::deserialize")]
  build: Option<PublishedBuild>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishedBuild {
  version: String,
  url: String,
  size: u64,
  sha256: String,
  notes: String,
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

/// Project Pages paths are derived from the configured repository. Published
/// metadata never gets to choose a different host, repository, or artifact.
pub(crate) fn pages_base(repository: &str) -> Option<String> {
  if !valid_repository(repository) {
    return None;
  }
  let (owner, repo) = repository.split_once('/')?;
  Some(format!("https://{}.github.io/{repo}", owner.to_ascii_lowercase()))
}

pub(crate) fn inspect(feed_json: &str, current_version: &str, repository: &str) -> Value {
  match inspect_feed(feed_json, current_version, repository) {
    Ok(value) => value,
    Err(error) => json!({"ok":false,"available":false,"error":error}),
  }
}

fn inspect_feed(
  feed_json: &str,
  current_version: &str,
  repository: &str,
) -> Result<Value, &'static str> {
  if feed_json.len() > MAX_UPDATE_BYTES {
    return Err("The update metadata exceeds the 1 MiB limit.");
  }
  let base = pages_base(repository).ok_or("The configured update repository is invalid.")?;
  let current = Version::parse(current_version).ok_or("The installed version is invalid.")?;
  let feed: UpdateFeed = serde_json::from_str(feed_json)
    .map_err(|_| "The update source returned invalid version metadata.")?;
  if feed.schema_version != 1 || feed.channel != "version" {
    return Err("The update source did not return a supported version feed.");
  }
  let Some(build) = feed.build else {
    return Ok(json!({"ok":true,"available":false}));
  };
  let version = Version::parse(&build.version)
    .filter(|version| version.normalized() == build.version)
    .ok_or("The published version must use major.minor.patch format.")?;
  if !(NRO_HEADER_BYTES..=MAX_NRO_BYTES).contains(&build.size) {
    return Err("The published update size is invalid.");
  }
  let expected_url = format!("{base}/builds/version/{}/wodex.nro", build.version);
  if build.url != expected_url {
    return Err("The update must use the configured project's exact GitHub Pages version URL.");
  }
  if build.sha256.len() != 64 || !build.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return Err("The published update is missing a valid SHA-256 digest.");
  }
  if version <= current {
    return Ok(json!({"ok":true,"available":false}));
  }
  let notes: String = build
    .notes
    .chars()
    .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    .take(4096)
    .collect();
  Ok(json!({"ok":true,"available":true,"build":{
    "version":version.normalized(),"download_url":build.url,
    "size":build.size,"sha256":build.sha256.to_ascii_lowercase(),"notes":notes
  }}))
}

/// Incremental hashing is independent from update parsing and never reads an
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

  fn feed(version: &str) -> Value {
    json!({
      "schema_version":1,"channel":"version",
      "build":{
        "version":version,"size":26_000_000,"notes":"New version\nWith improvements",
        "url":format!("https://example.github.io/wodex/builds/version/{version}/wodex.nro"),
        "sha256":"ab".repeat(32)
      }
    })
  }

  fn inspect_value(feed: &Value, current: &str) -> Value {
    inspect(&feed.to_string(), current, "example/wodex")
  }

  #[test]
  fn stable_versions_compare_numerically_and_normalize() {
    let result = inspect_value(&feed("0.10.0"), "0.9.99");
    assert_eq!(result["ok"], true);
    assert_eq!(result["available"], true);
    assert_eq!(result["build"]["version"], "0.10.0");
    assert_eq!(result["build"]["sha256"], "ab".repeat(32));
    assert_eq!(result["build"]["notes"], "New version\nWith improvements");
    for (version, current) in
      [("0.2.0", "0.2.0"), ("0.2.0", "v0.3.0"), ("1.2.99", "1.10.0"), ("9.999.999", "10.0.0")]
    {
      assert_eq!(inspect_value(&feed(version), current), json!({"ok":true,"available":false}));
    }
    for version in [
      "v1.2.0",
      "1.2.0-rc1",
      "1.2.0+metadata",
      "1.02.0",
      "V1.2.0",
      "1.2",
      "1.2.3.4",
      "-1.2.3",
      " 1.2.3",
      "1.2.18446744073709551616",
    ] {
      assert_eq!(inspect_value(&feed(version), "0.1.0")["ok"], false, "{version}");
    }
    for current in ["", "garbage", "v1.2.3-rc1", "1.2"] {
      assert_eq!(inspect_value(&feed("1.2.3"), current)["ok"], false);
    }
  }

  #[test]
  fn version_feed_requires_supported_schema_and_explicit_nullable_build() {
    let empty = json!({"schema_version":1,"channel":"version","build":null});
    assert_eq!(inspect_value(&empty, "0.1.0"), json!({"ok":true,"available":false}));
    for (field, value) in [
      ("schema_version", json!(0)),
      ("schema_version", json!(2)),
      ("schema_version", json!("1")),
      ("channel", json!("nightly")),
      ("channel", json!(null)),
      ("build", json!([])),
      ("unknown", json!(true)),
    ] {
      let mut metadata = feed("0.2.0");
      metadata[field] = value;
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{field}");
    }
    for field in ["schema_version", "channel", "build"] {
      let mut metadata = feed("0.2.0");
      metadata.as_object_mut().unwrap().remove(field);
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{field}");
    }
    assert_eq!(inspect_value(&empty, "invalid")["ok"], false);
    assert_eq!(inspect(&empty.to_string(), "0.1.0", "../bad")["ok"], false);
  }

  #[test]
  fn rejects_incomplete_malformed_and_oversized_version_metadata() {
    for (field, value) in [
      ("size", json!(127)),
      ("size", json!(MAX_NRO_BYTES + 1)),
      ("size", json!(-1)),
      ("size", json!("26000000")),
      ("sha256", Value::Null),
      ("sha256", json!("sha256:bad")),
      ("sha256", json!("zz".repeat(32))),
      ("notes", Value::Null),
      ("unexpected", json!(true)),
    ] {
      let mut metadata = feed("0.2.0");
      metadata["build"][field] = value;
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{field}");
    }
    for field in ["version", "url", "size", "sha256", "notes"] {
      let mut metadata = feed("0.2.0");
      metadata["build"].as_object_mut().unwrap().remove(field);
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{field}");
    }
    let private_error = inspect("{\"message\":\"secret user error\"}", "0.1.0", "example/wodex");
    assert_eq!(private_error["ok"], false);
    assert!(!private_error.to_string().contains("secret user error"));
    assert_eq!(inspect(&"x".repeat(MAX_UPDATE_BYTES + 1), "0.1.0", "example/wodex")["ok"], false);
    // serde rejects duplicate fields rather than allowing ambiguous feeds.
    assert_eq!(
      inspect(
        "{\"schema_version\":1,\"channel\":\"version\",\"build\":null,\"build\":null}",
        "0.1.0",
        "example/wodex"
      )["ok"],
      false
    );
  }

  #[test]
  fn download_urls_are_bound_to_the_configured_project_and_exact_version_path() {
    for url in [
      "http://example.github.io/wodex/builds/version/0.2.0/wodex.nro",
      "https://example.github.io.evil.test/wodex/builds/version/0.2.0/wodex.nro",
      "https://example.github.io@evil.test/wodex/builds/version/0.2.0/wodex.nro",
      "https://user@example.github.io/wodex/builds/version/0.2.0/wodex.nro",
      "https://example.github.io:443/wodex/builds/version/0.2.0/wodex.nro",
      "https://other.github.io/wodex/builds/version/0.2.0/wodex.nro",
      "https://example.github.io/other/builds/version/0.2.0/wodex.nro",
      "https://example.github.io/wodex/builds/version/0.1.0/wodex.nro",
      "https://example.github.io/wodex/builds/version/0.2.0/wodex.nro?token=secret",
      "https://example.github.io/wodex/builds/version/0.2.0/wodex.nro#fragment",
      "https://example.github.io/wodex/builds/version/0.2.0/other.nro",
      "https://example.github.io/wodex/../builds/version/0.2.0/wodex.nro",
      "https://example.github.io/wodex/builds/version/%30.2.0/wodex.nro",
      "https://example.github.io/wodex/builds/nightly/0.2.0/wodex.nro",
      "https://github.com/example/wodex/releases/download/0.2.0/wodex.nro",
    ] {
      let mut metadata = feed("0.2.0");
      metadata["build"]["url"] = json!(url);
      assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false, "{url}");
      // Invalid metadata must not become acceptable just because it is old.
      assert_eq!(inspect_value(&metadata, "1.0.0")["ok"], false, "{url}");
    }
    let mut metadata = feed("0.2.0");
    metadata["build"]["sha256"] = json!("AB".repeat(32));
    // DNS owner names are case-insensitive; project Pages paths retain case.
    assert_eq!(inspect(&metadata.to_string(), "0.1.0", "EXAMPLE/Wodex")["ok"], false);
    metadata["build"]["url"] =
      json!("https://example.github.io/Wodex/builds/version/0.2.0/wodex.nro");
    let result = inspect(&metadata.to_string(), "0.1.0", "EXAMPLE/Wodex");
    assert_eq!(result["ok"], true);
    assert_eq!(result["build"]["sha256"], "ab".repeat(32));
    assert_eq!(inspect_value(&metadata, "0.1.0")["ok"], false);
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
      assert!(pages_base(repository).is_none());
    }
    assert!(!valid_repository(&format!("{}/repo", "a".repeat(40))));
    assert!(!valid_repository(&format!("owner/{}", "a".repeat(101))));
    let mut metadata = feed("0.2.0");
    metadata["build"]["notes"] = json!(format!("\u{0001}{}", "文".repeat(5000)));
    let response = inspect_value(&metadata, "0.1.0");
    assert_eq!(response["build"]["notes"].as_str().unwrap().chars().count(), 4096);
    assert!(!response["build"]["notes"].as_str().unwrap().contains('\u{0001}'));
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
