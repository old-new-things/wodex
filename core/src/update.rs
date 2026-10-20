use alloc::{format, string::String, vec::Vec};
use serde::{Deserialize, Serialize};

use crate::{
  build_identity::{self, BuildIdentity},
  build_metadata::{valid_commit, valid_timestamp},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const MAX_UPDATE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_VERSION_BYTES: usize = 64;
pub(crate) const MAX_NRO_BYTES: u64 = 128 * 1024 * 1024;
const NRO_HEADER_BYTES: u64 = 0x80;
const ASSET_HEADER_BYTES: u64 = 0x38;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
  #[default]
  Version,
  Nightly,
}

impl UpdateChannel {
  pub fn label(self) -> &'static str {
    match self {
      Self::Version => "Versions",
      Self::Nightly => "Nightly",
    }
  }

  pub(crate) fn feed_name(self) -> &'static str {
    match self {
      Self::Version => "stable.json",
      Self::Nightly => "nightly.json",
    }
  }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
struct UpdateFeed<T> {
  schema_version: u32,
  channel: UpdateChannel,
  #[serde(deserialize_with = "Option::deserialize")]
  build: Option<T>,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NightlyBuild {
  version: String,
  url: String,
  size: u64,
  sha256: String,
  notes: String,
  id: String,
  timestamp: String,
  commit: String,
}

struct Candidate {
  build: PublishedBuild,
  nightly: Option<(String, String)>,
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
  inspect_channel(
    feed_json,
    current_version,
    repository,
    UpdateChannel::Version,
    BuildIdentity { channel: None, timestamp: "", commit: "" },
  )
}

pub(crate) fn inspect_channel(
  feed_json: &str,
  current_version: &str,
  repository: &str,
  channel: UpdateChannel,
  installed: BuildIdentity<'_>,
) -> Value {
  match inspect_feed(feed_json, current_version, repository, channel, installed) {
    Ok(value) => value,
    Err(error) => json!({"ok":false,"available":false,"error":error}),
  }
}

fn parse_feed<T: for<'de> Deserialize<'de>>(
  feed_json: &str,
  channel: UpdateChannel,
) -> Result<Option<T>, &'static str> {
  let feed: UpdateFeed<T> = serde_json::from_str(feed_json)
    .map_err(|_| "The update source returned invalid build metadata.")?;
  if feed.schema_version != 1 || feed.channel != channel {
    return Err("The update source did not return the selected channel's supported feed.");
  }
  Ok(feed.build)
}

fn inspect_feed(
  feed_json: &str,
  current_version: &str,
  repository: &str,
  channel: UpdateChannel,
  installed: BuildIdentity<'_>,
) -> Result<Value, &'static str> {
  if feed_json.len() > MAX_UPDATE_BYTES {
    return Err("The update metadata exceeds the 1 MiB limit.");
  }
  let base = pages_base(repository).ok_or("The configured update repository is invalid.")?;
  let current = Version::parse(current_version).ok_or("The installed version is invalid.")?;
  let candidate = match channel {
    UpdateChannel::Version => parse_feed::<PublishedBuild>(feed_json, channel)?
      .map(|build| Candidate { build, nightly: None }),
    UpdateChannel::Nightly => match parse_feed::<NightlyBuild>(feed_json, channel)? {
      Some(nightly) => {
        if !valid_timestamp(&nightly.timestamp)
          || !valid_commit(&nightly.commit)
          || nightly.id != build_identity::nightly_id(&nightly.timestamp, &nightly.commit)
        {
          return Err("The published nightly build identity is invalid.");
        }
        Some(Candidate {
          build: PublishedBuild {
            version: nightly.version,
            url: nightly.url,
            size: nightly.size,
            sha256: nightly.sha256,
            notes: nightly.notes,
          },
          nightly: Some((nightly.id, nightly.timestamp)),
        })
      }
      None => None,
    },
  };
  let Some(Candidate { build, nightly }) = candidate else {
    return Ok(json!({"ok":true,"available":false}));
  };
  let version = Version::parse(&build.version)
    .filter(|version| version.normalized() == build.version)
    .ok_or("The published version must use major.minor.patch format.")?;
  if !(NRO_HEADER_BYTES..=MAX_NRO_BYTES).contains(&build.size) {
    return Err("The published update size is invalid.");
  }
  let path = match &nightly {
    Some((id, _)) => format!("nightly/{id}"),
    None => format!("version/{}", build.version),
  };
  let expected_url = format!("{base}/builds/{path}/wodex.nro");
  if build.url != expected_url {
    return Err("The update must use the configured project's exact GitHub Pages build URL.");
  }
  if build.sha256.len() != 64 || !build.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
    return Err("The published update is missing a valid SHA-256 digest.");
  }
  // Nightlies follow the publisher's chronological order independently of their
  // base version. Returning from Nightly to Versions selects the published
  // version even when its number is lower than the installed nightly's.
  let available = match &nightly {
    Some((id, timestamp)) => {
      installed.timestamp.is_empty()
        || timestamp.as_str() > installed.timestamp
        || (installed.channel == Some(UpdateChannel::Nightly)
          && timestamp.as_str() == installed.timestamp
          && *id > build_identity::nightly_id(installed.timestamp, installed.commit))
    }
    None => installed.channel == Some(UpdateChannel::Nightly) || version > current,
  };
  if !available {
    return Ok(json!({"ok":true,"available":false}));
  }
  let label = match &nightly {
    Some((id, _)) => format!("{} nightly {id}", build.version),
    None => version.normalized(),
  };
  let notes: String = build
    .notes
    .chars()
    .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
    .take(4096)
    .collect();
  Ok(json!({"ok":true,"available":true,"build":{
    "version":version.normalized(),"label":label,"download_url":build.url,
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

  fn nightly_feed(version: &str, timestamp: &str) -> Value {
    let commit = "abcdef0123456789abcdef0123456789abcdef01";
    let id = build_identity::nightly_id(timestamp, commit);
    let mut value = feed(version);
    value["channel"] = json!("nightly");
    value["build"]["id"] = json!(id);
    value["build"]["timestamp"] = json!(timestamp);
    value["build"]["commit"] = json!(commit);
    value["build"]["url"] =
      json!(format!("https://example.github.io/wodex/builds/nightly/{id}/wodex.nro"));
    value
  }

  fn inspect_nightly(value: &Value, version: &str, identity: BuildIdentity<'_>) -> Value {
    inspect_channel(&value.to_string(), version, "example/wodex", UpdateChannel::Nightly, identity)
  }

  #[test]
  fn nightly_updates_follow_build_times_independently_of_semantic_versions() {
    let installed = BuildIdentity {
      channel: Some(UpdateChannel::Nightly),
      timestamp: "2026-10-20T01:02:03Z",
      commit: "abcdef0123456789abcdef0123456789abcdef01",
    };
    for channel in [UpdateChannel::Nightly, UpdateChannel::Version] {
      let identity = BuildIdentity { channel: Some(channel), ..installed };
      for version in ["0.2.1", "0.3.0", "0.4.0"] {
        for (timestamp, available) in [
          ("2026-10-19T23:59:59Z", false),
          ("2026-10-20T01:02:02Z", false),
          ("2026-10-20T01:02:03Z", false),
          ("2026-10-20T01:02:04Z", true),
        ] {
          let response = inspect_nightly(&nightly_feed(version, timestamp), "0.3.0", identity);
          assert_eq!(response["ok"], true);
          assert_eq!(response["available"], available, "{channel:?}: {version} at {timestamp}");
        }
      }
    }
    let next = nightly_feed("0.3.0", "2026-10-20T01:02:04Z");
    let response = inspect_nightly(&next, "0.3.0", installed);
    assert_eq!(response["available"], true);
    assert_eq!(response["build"]["label"], "0.3.0 nightly 20261020T010204Z-abcdef0");
    assert_eq!(response["build"]["version"], "0.3.0");
    let updated = BuildIdentity { timestamp: "2026-10-20T01:02:04Z", ..installed };
    assert_eq!(inspect_nightly(&next, "0.3.0", updated)["available"], false);
    assert_eq!(inspect_nightly(&next, "0.4.0", installed)["available"], true);
    for version in ["0.2.1", "0.3.0", "0.4.0"] {
      assert_eq!(inspect_nightly(&next, version, BuildIdentity::development())["available"], true);
    }
    let stable = BuildIdentity { channel: Some(UpdateChannel::Version), ..updated };
    assert_eq!(inspect_nightly(&next, "0.3.0", stable)["available"], false);
  }

  #[test]
  fn equal_time_nightlies_use_the_publishers_build_id_tie_break() {
    let installed = BuildIdentity {
      channel: Some(UpdateChannel::Nightly),
      timestamp: "2026-10-20T01:02:03Z",
      commit: "abcdef0123456789abcdef0123456789abcdef01",
    };
    for (commit, available) in [
      ("a0cdef0123456789abcdef0123456789abcdef01", false),
      (installed.commit, false),
      ("bbcdef0123456789abcdef0123456789abcdef01", true),
    ] {
      let mut candidate = nightly_feed("0.3.0", installed.timestamp);
      let id = build_identity::nightly_id(installed.timestamp, commit);
      candidate["build"]["id"] = json!(id);
      candidate["build"]["commit"] = json!(commit);
      candidate["build"]["url"] =
        json!(format!("https://example.github.io/wodex/builds/nightly/{id}/wodex.nro"));
      for version in ["0.2.1", "0.3.0", "0.4.0"] {
        candidate["build"]["version"] = json!(version);
        assert_eq!(inspect_nightly(&candidate, "0.3.0", installed)["available"], available);
        let stable = BuildIdentity { channel: Some(UpdateChannel::Version), ..installed };
        assert_eq!(inspect_nightly(&candidate, "0.3.0", stable)["available"], false);
      }
    }
    let older_time = nightly_feed("0.4.0", "2026-10-19T01:02:03Z");
    assert_eq!(inspect_nightly(&older_time, "0.3.0", installed)["available"], false);
  }

  #[test]
  fn embedded_identity_prevents_reoffering_the_installed_build() {
    // CI also rebuilds this test with both published channels to exercise the
    // actual build script and immutable identity embedded in the executable.
    let installed = BuildIdentity::embedded();
    let Some(channel) = installed.channel else { return };
    let version = env!("CARGO_PKG_VERSION");
    let mut candidate = nightly_feed(version, installed.timestamp);
    let id = build_identity::nightly_id(installed.timestamp, installed.commit);
    candidate["build"]["id"] = json!(id);
    candidate["build"]["commit"] = json!(installed.commit);
    candidate["build"]["url"] =
      json!(format!("https://example.github.io/wodex/builds/nightly/{id}/wodex.nro"));
    assert_eq!(
      inspect_nightly(&candidate, version, installed),
      json!({"ok":true,"available":false})
    );
    let stable = inspect_channel(
      &feed(version).to_string(),
      version,
      "example/wodex",
      UpdateChannel::Version,
      installed,
    );
    assert_eq!(stable["ok"], true);
    assert_eq!(stable["available"], channel == UpdateChannel::Nightly);
    // A published Nightly can return to a lower Versions build. The same
    // transition on a published Version must remain unavailable.
    let lower_version = inspect_channel(
      &feed("0.2.1").to_string(),
      "0.3.0",
      "example/wodex",
      UpdateChannel::Version,
      installed,
    );
    assert_eq!(lower_version["ok"], true);
    assert_eq!(lower_version["available"], channel == UpdateChannel::Nightly);
    // A higher base version must not make the installed nightly available again.
    candidate["build"]["version"] = json!("9.0.0");
    assert_eq!(
      inspect_nightly(&candidate, "0.3.0", installed),
      json!({"ok":true,"available":false})
    );
  }

  #[test]
  fn switching_from_nightly_can_install_any_latest_version() {
    let installed = BuildIdentity {
      channel: Some(UpdateChannel::Nightly),
      timestamp: "2026-10-20T01:02:03Z",
      commit: "abcdef0123456789abcdef0123456789abcdef01",
    };
    for channel in [Some(UpdateChannel::Nightly), Some(UpdateChannel::Version), None] {
      let identity = match channel {
        None => BuildIdentity::development(),
        _ => BuildIdentity { channel, ..installed },
      };
      for version in ["0.2.1", "0.3.0", "0.3.1"] {
        let response = inspect_channel(
          &feed(version).to_string(),
          "0.3.0",
          "example/wodex",
          UpdateChannel::Version,
          identity,
        );
        assert_eq!(response["ok"], true);
        assert_eq!(
          response["available"],
          channel == Some(UpdateChannel::Nightly) || version == "0.3.1",
          "{channel:?}: {version}"
        );
      }
    }
  }

  #[test]
  fn nightly_feed_identity_and_url_must_agree_and_channels_are_strict() {
    let valid = nightly_feed("0.3.0", "2026-10-20T01:02:03Z");
    for (field, value) in [
      ("id", json!("20261020T010203Z-0000000")),
      ("id", json!("../20261020T010203Z-abcdef0")),
      ("timestamp", json!("2026-02-29T01:02:03Z")),
      ("timestamp", json!("2026-10-20T01:02:04Z")),
      ("commit", json!("ABCDEF0123456789abcdef0123456789abcdef01")),
      ("commit", json!("abcdef0")),
      ("url", json!("https://example.github.io/wodex/builds/version/0.3.0/wodex.nro")),
      (
        "url",
        json!(
          "https://example.github.io/wodex/builds/nightly/20261020T010203Z-abcdef0/wodex.nro?x=1"
        ),
      ),
      (
        "url",
        json!("https://other.github.io/wodex/builds/nightly/20261020T010203Z-abcdef0/wodex.nro"),
      ),
      ("extra", json!(true)),
    ] {
      let mut invalid = valid.clone();
      invalid["build"][field] = value;
      assert_eq!(
        inspect_nightly(&invalid, "0.1.0", BuildIdentity::development())["ok"],
        false,
        "{field}"
      );
      assert_eq!(
        inspect_nightly(&invalid, "9.0.0", BuildIdentity::development())["ok"],
        false,
        "{field}"
      );
    }
    for field in ["id", "timestamp", "commit"] {
      let mut invalid = valid.clone();
      invalid["build"].as_object_mut().unwrap().remove(field);
      assert_eq!(
        inspect_nightly(&invalid, "0.1.0", BuildIdentity::development())["ok"],
        false,
        "{field}"
      );
    }
    assert_eq!(inspect_value(&valid, "0.1.0")["ok"], false);
    assert_eq!(inspect_nightly(&feed("0.3.0"), "0.1.0", BuildIdentity::development())["ok"], false);
    let mut wrong_channel = valid.clone();
    wrong_channel["channel"] = json!("version");
    assert_eq!(inspect_nightly(&wrong_channel, "0.1.0", BuildIdentity::development())["ok"], false);
    let empty = json!({"schema_version":1,"channel":"nightly","build":null});
    assert_eq!(
      inspect_nightly(&empty, "0.1.0", BuildIdentity::development()),
      json!({"ok":true,"available":false})
    );
    let mut missing_build = empty;
    missing_build.as_object_mut().unwrap().remove("build");
    assert_eq!(inspect_nightly(&missing_build, "0.1.0", BuildIdentity::development())["ok"], false);
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
