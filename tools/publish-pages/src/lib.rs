use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
  collections::{BTreeMap, BTreeSet},
  fs::{self, File, OpenOptions},
  io::{Read, Write},
  path::{Component, Path, PathBuf},
  sync::atomic::{AtomicU64, Ordering},
};

const FILES: [&str; 9] = [
  "wodex.nro",
  "wodex.nro.sha256",
  "VERSION",
  "LICENSE",
  "THIRD_PARTY.md",
  "licenses.tar.gz",
  "licenses.tar.gz.sha256",
  "NotoSansCJK-OFL.txt",
  "NotoSansCJK-README.md",
];
const MAX_NRO: u64 = 128 * 1024 * 1024;
const MAX_SITE: u64 = 450 * 1024 * 1024;
const MAX_CATALOG: u64 = 4 * 1024 * 1024;
const NIGHTLIES: usize = 7;
const VERSIONS: usize = 10;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
  Nightly,
  Version,
}

impl Channel {
  fn path(self) -> &'static str {
    match self {
      Self::Nightly => "nightly",
      Self::Version => "version",
    }
  }
}

pub struct Options {
  pub artifact_dir: PathBuf,
  pub previous_dir: Option<PathBuf>,
  pub output_dir: PathBuf,
  pub channel: Channel,
  pub timestamp: String,
  pub commit: String,
  pub base_url: String,
  pub tag: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildInfo {
  schema_version: u32,
  version: String,
  channel: Channel,
  timestamp: String,
  commit: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Asset {
  pub size: u64,
  pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Build {
  pub channel: Channel,
  pub id: String,
  pub version: String,
  pub timestamp: String,
  pub commit: String,
  pub size: u64,
  pub sha256: String,
  pub licenses_sha256: String,
  pub assets: BTreeMap<String, Asset>,
}

impl Build {
  fn path(&self) -> PathBuf {
    PathBuf::from("builds").join(self.channel.path()).join(&self.id)
  }

  fn total_size(&self) -> u64 {
    self.assets.values().map(|asset| asset.size).sum()
  }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
  pub schema_version: u32,
  pub builds: Vec<Build>,
  // Keep digest records after pruning downloads so a version can never be reassigned.
  pub version_locks: BTreeMap<String, BTreeMap<String, Asset>>,
}

impl Default for Catalog {
  fn default() -> Self {
    Self { schema_version: 1, builds: Vec::new(), version_locks: BTreeMap::new() }
  }
}

fn io<T>(result: std::io::Result<T>, message: &str) -> Result<T> {
  result.map_err(|_| message.to_owned())
}

fn version(text: &str) -> Result<[u64; 3]> {
  if text.len() > 15 {
    return Err("Version must be a stable X.Y.Z value of at most 15 characters.".into());
  }
  let parts: Vec<_> = text.split('.').collect();
  if parts.len() != 3 {
    return Err("Version must contain three numeric components.".into());
  }
  let mut parsed = [0; 3];
  for (index, part) in parts.iter().enumerate() {
    if part.is_empty()
      || !part.bytes().all(|byte| byte.is_ascii_digit())
      || (part.len() > 1 && part.starts_with('0'))
    {
      return Err("Version contains an invalid component.".into());
    }
    parsed[index] = part.parse().map_err(|_| "Version component is too large.")?;
  }
  Ok(parsed)
}

fn timestamp(text: &str) -> Result<String> {
  let bytes = text.as_bytes();
  if bytes.len() != 20
    || [4, 7].iter().any(|&index| bytes[index] != b'-')
    || bytes[10] != b'T'
    || [13, 16].iter().any(|&index| bytes[index] != b':')
    || bytes[19] != b'Z'
    || bytes
      .iter()
      .enumerate()
      .any(|(index, byte)| ![4, 7, 10, 13, 16, 19].contains(&index) && !byte.is_ascii_digit())
  {
    return Err("Timestamp must be UTC YYYY-MM-DDTHH:MM:SSZ.".into());
  }
  let number = |start: usize, end: usize| text[start..end].parse::<u32>().unwrap();
  let (year, month, day) = (number(0, 4), number(5, 7), number(8, 10));
  let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
  let days = match month {
    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
    4 | 6 | 9 | 11 => 30,
    2 => {
      if leap {
        29
      } else {
        28
      }
    }
    _ => 0,
  };
  if year < 2000
    || day == 0
    || day > days
    || number(11, 13) > 23
    || number(14, 16) > 59
    || number(17, 19) > 59
  {
    return Err("Timestamp is not a valid UTC date and time.".into());
  }
  Ok(text.chars().filter(|character| !matches!(character, '-' | ':')).collect())
}

fn hexadecimal(text: &str, len: usize) -> bool {
  text.len() == len
    && text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn base_url(text: &str) -> Result<()> {
  let rest = text.strip_prefix("https://").ok_or("Pages base URL must use HTTPS.")?;
  let (host, repo) = rest.split_once('/').ok_or("Pages base URL must identify a project.")?;
  let owner = host.strip_suffix(".github.io").ok_or("Pages base URL must use github.io.")?;
  if owner.is_empty()
    || owner.len() > 39
    || !owner.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    || owner.starts_with('-')
    || owner.ends_with('-')
    || owner.contains("--")
    || repo.is_empty()
    || repo.len() > 100
    || !repo.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    || repo.contains("..")
    || repo == "."
  {
    return Err(
      "Pages base URL must be a canonical HTTPS project URL without query or fragment.".into(),
    );
  }
  Ok(())
}

// Check every component, including parent directories, instead of allowing a
// restored artifact to redirect reads or writes outside its designated tree.
fn safe_path(path: &Path, may_be_missing: bool) -> Result<()> {
  let absolute = if path.is_absolute() {
    path.to_owned()
  } else {
    io(std::env::current_dir(), "Cannot resolve working directory.")?.join(path)
  };
  let mut parent = PathBuf::new();
  for component in absolute.components() {
    match component {
      Component::ParentDir | Component::Prefix(_) => return Err("Unsafe path component.".into()),
      Component::CurDir => continue,
      _ => parent.push(component.as_os_str()),
    }
    match fs::symlink_metadata(&parent) {
      Ok(metadata) if metadata.file_type().is_symlink() => {
        return Err("Symbolic links are not accepted in publication paths.".into())
      }
      Ok(_) => {}
      Err(error) if may_be_missing && error.kind() == std::io::ErrorKind::NotFound => {}
      Err(_) => return Err("Publication path is unavailable.".into()),
    }
  }
  Ok(())
}

fn safe_directory(path: &Path) -> Result<()> {
  safe_path(path, false)?;
  if !io(fs::metadata(path), "Cannot inspect input directory.")?.is_dir() {
    return Err("Input must be a directory.".into());
  }
  Ok(())
}

fn asset_limit(name: &str) -> u64 {
  match name {
    "wodex.nro" => MAX_NRO,
    "licenses.tar.gz" => 16 * 1024 * 1024,
    _ => 1024 * 1024,
  }
}

fn file(path: &Path, limit: u64) -> Result<File> {
  safe_path(path, false)?;
  let metadata = io(fs::symlink_metadata(path), "Cannot inspect publication file.")?;
  if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
    return Err("Publication file type or size is invalid.".into());
  }
  io(File::open(path), "Cannot open publication file.")
}

fn bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
  let mut value = Vec::new();
  io(file(path, limit)?.take(limit + 1).read_to_end(&mut value), "Cannot read publication file.")?;
  if value.len() as u64 > limit {
    return Err("Publication file exceeds its size limit.".into());
  }
  Ok(value)
}

fn asset(path: &Path, limit: u64) -> Result<Asset> {
  let mut input = file(path, limit)?;
  let mut hash = Sha256::new();
  let mut buffer = [0; 64 * 1024];
  let mut size = 0;
  loop {
    let count = io(input.read(&mut buffer), "Cannot hash publication file.")?;
    if count == 0 {
      break;
    }
    size += count as u64;
    if size > limit {
      return Err("Publication file exceeds its size limit.".into());
    }
    hash.update(&buffer[..count]);
  }
  Ok(Asset { size, sha256: format!("{:x}", hash.finalize()) })
}

fn checksum(dir: &Path, name: &str, expected: &str) -> Result<()> {
  let value = bytes(&dir.join(format!("{name}.sha256")), 256)?;
  let text = std::str::from_utf8(&value).map_err(|_| "Checksum must be UTF-8.")?;
  if text != format!("{expected}  {name}\n") && text != format!("{expected} *{name}\n") {
    return Err(format!("Checksum does not match {name}."));
  }
  Ok(())
}

fn inspect_artifacts(dir: &Path) -> Result<(String, BTreeMap<String, Asset>)> {
  safe_directory(dir)?;
  let mut assets = BTreeMap::new();
  for name in FILES {
    assets.insert(name.to_owned(), asset(&dir.join(name), asset_limit(name))?);
  }
  let source = bytes(&dir.join("VERSION"), 16)?;
  let text = std::str::from_utf8(&source).map_err(|_| "VERSION must be UTF-8.")?;
  let parsed_version = text.strip_suffix('\n').unwrap_or(text);
  version(parsed_version)?;
  let nro = &assets["wodex.nro"];
  if nro.size < 0x80 {
    return Err("NRO is smaller than its executable header.".into());
  }
  let mut header = [0; 0x80];
  io(file(&dir.join("wodex.nro"), MAX_NRO)?.read_exact(&mut header), "Cannot read NRO header.")?;
  if &header[0x10..0x14] != b"NRO0" {
    return Err("Artifact does not contain the NRO0 executable signature.".into());
  }
  let declared_size = u32::from_le_bytes(header[0x18..0x1c].try_into().unwrap()) as u64;
  if declared_size < 0x80 || declared_size > nro.size {
    return Err("NRO executable length is outside its file bounds.".into());
  }
  checksum(dir, "wodex.nro", &nro.sha256)?;
  checksum(dir, "licenses.tar.gz", &assets["licenses.tar.gz"].sha256)?;
  let mut gzip = [0; 2];
  io(
    file(&dir.join("licenses.tar.gz"), 16 * 1024 * 1024)?.read_exact(&mut gzip),
    "Cannot read license archive.",
  )?;
  if gzip != [0x1f, 0x8b] {
    return Err("License archive must be gzip data.".into());
  }
  Ok((parsed_version.to_owned(), assets))
}

fn verify_build_info(options: &Options, version: &str) -> Result<()> {
  let info: BuildInfo =
    serde_json::from_slice(&bytes(&options.artifact_dir.join("build-info.json"), 1024)?)
      .map_err(|_| "Incoming build identity metadata is invalid.")?;
  if info.schema_version != 1
    || info.version != version
    || info.channel != options.channel
    || info.timestamp != options.timestamp
    || info.commit != options.commit
  {
    return Err("Incoming build identity does not match the publication metadata.".into());
  }
  Ok(())
}

fn validate_assets(assets: &BTreeMap<String, Asset>) -> Result<()> {
  if assets.len() != FILES.len() || FILES.iter().any(|name| !assets.contains_key(*name)) {
    return Err("Catalog must contain exactly the supported download files.".into());
  }
  for (name, value) in assets {
    if value.size == 0 || value.size > asset_limit(name) || !hexadecimal(&value.sha256, 64) {
      return Err("Catalog contains invalid asset metadata.".into());
    }
  }
  Ok(())
}

fn validate_build(build: &Build) -> Result<()> {
  version(&build.version)?;
  let compact_time = timestamp(&build.timestamp)?;
  if !hexadecimal(&build.commit, 40) {
    return Err("Catalog commit must contain 40 lowercase hexadecimal characters.".into());
  }
  let expected_id = match build.channel {
    Channel::Nightly => format!("{compact_time}-{}", &build.commit[..7]),
    Channel::Version => build.version.clone(),
  };
  if build.id != expected_id {
    return Err("Catalog build path does not match its validated identity.".into());
  }
  validate_assets(&build.assets)?;
  if build.size != build.assets["wodex.nro"].size
    || build.sha256 != build.assets["wodex.nro"].sha256
    || build.licenses_sha256 != build.assets["licenses.tar.gz"].sha256
  {
    return Err("Catalog digest fields disagree.".into());
  }
  Ok(())
}

fn previous(dir: Option<&Path>) -> Result<Catalog> {
  let Some(dir) = dir else { return Ok(Catalog::default()) };
  safe_directory(dir)?;
  let catalog: Catalog = serde_json::from_slice(&bytes(&dir.join("catalog.json"), MAX_CATALOG)?)
    .map_err(|_| "Previous catalog is invalid.")?;
  if catalog.schema_version != 1 || catalog.builds.len() > NIGHTLIES + VERSIONS {
    return Err("Previous catalog schema or build count is unsupported.".into());
  }
  for (value, assets) in &catalog.version_locks {
    version(value)?;
    validate_assets(assets)?;
  }
  let mut identities = BTreeSet::new();
  for build in &catalog.builds {
    validate_build(build)?;
    if !identities.insert((build.channel, &build.id)) {
      return Err("Previous catalog contains duplicate builds.".into());
    }
    if build.channel == Channel::Version
      && catalog.version_locks.get(&build.id) != Some(&build.assets)
    {
      return Err("Version does not match its immutable digest record.".into());
    }
    let (stored_version, assets) = inspect_artifacts(&dir.join(build.path()))?;
    if stored_version != build.version || assets != build.assets {
      return Err("Previous download files do not match their catalog.".into());
    }
  }
  Ok(catalog)
}

fn retain(builds: &mut Vec<Build>, incoming: &(Channel, String)) -> Result<()> {
  builds.sort_by(|left, right| {
    left.channel.cmp(&right.channel).then_with(|| match left.channel {
      Channel::Nightly => right.timestamp.cmp(&left.timestamp).then_with(|| right.id.cmp(&left.id)),
      Channel::Version => version(&right.version).unwrap().cmp(&version(&left.version).unwrap()),
    })
  });
  let (mut nightlies, mut versions) = (0, 0);
  builds.retain(|build| {
    let (count, limit) = match build.channel {
      Channel::Nightly => (&mut nightlies, NIGHTLIES),
      Channel::Version => (&mut versions, VERSIONS),
    };
    *count += 1;
    *count <= limit
  });
  while builds.iter().map(Build::total_size).sum::<u64>() > MAX_SITE {
    let mut seen = BTreeSet::new();
    let oldest = builds
      .iter()
      .enumerate()
      .filter(|(_, build)| {
        !seen.insert(build.channel) && (build.channel, build.id.clone()) != *incoming
      })
      .min_by_key(|(_, build)| &build.timestamp)
      .map(|(index, _)| index)
      .ok_or("Newest downloads exceed the total site size limit.")?;
    builds.remove(oldest);
  }
  Ok(())
}

fn escaped(value: &str) -> String {
  value.chars().fold(String::new(), |mut output, character| {
    match character {
      '&' => output.push_str("&amp;"),
      '<' => output.push_str("&lt;"),
      '>' => output.push_str("&gt;"),
      '"' => output.push_str("&quot;"),
      '\'' => output.push_str("&#39;"),
      _ => output.push(character),
    }
    output
  })
}

fn html(catalog: &Catalog) -> String {
  let mut content = String::new();
  for (channel, title, description) in [
    (
      Channel::Nightly,
      "Nightly",
      "Recent successful builds from the main branch. Opt in to Nightly in the app's update settings. Dates are in UTC.",
    ),
    (
      Channel::Version,
      "Versions",
      "Explicitly tagged versions, listed from newest version to oldest.",
    ),
  ] {
    content.push_str(&format!(
      "<section aria-labelledby=\"{}\"><div class=\"section-heading\"><h2 id=\"{}\">{title}</h2><p>{description}</p></div><div class=\"builds\">",
      channel.path(), channel.path()
    ));
    let mut count = 0;
    for build in catalog.builds.iter().filter(|build| build.channel == channel) {
      let path = format!("builds/{}/{}", channel.path(), escaped(&build.id));
      let title = match channel {
        Channel::Nightly => format!("{} UTC", &build.timestamp[..10]),
        Channel::Version => format!("v{}", build.version),
      };
      let marker = if count == 0 { "<span class=\"badge\">Latest</span>" } else { "" };
      content.push_str(&format!(
        "<article><div class=\"build-heading\"><h3>{}</h3>{marker}</div>\
        <p class=\"metadata\"><time datetime=\"{}\">{} UTC</time><span>Commit <code>{}</code></span></p>\
        <p class=\"build-id\">{}</p><div class=\"downloads\"><a class=\"download\" href=\"{path}/wodex.nro\" download>Download .nro <span>({:.1} MiB)</span></a>\
        <a href=\"{path}/licenses.tar.gz\" download>License archive</a></div>\
        <details><summary>SHA-256 &amp; notices</summary><code class=\"checksum\">{}</code>\
        <ul><li><a href=\"{path}/wodex.nro.sha256\">Executable checksum</a></li>\
        <li><a href=\"{path}/licenses.tar.gz.sha256\">License archive checksum</a></li>\
        <li><a href=\"{path}/LICENSE\">Wodex license</a></li>\
        <li><a href=\"{path}/THIRD_PARTY.md\">Third-party notices</a></li>\
        <li><a href=\"{path}/NotoSansCJK-OFL.txt\">Font license</a></li>\
        <li><a href=\"{path}/NotoSansCJK-README.md\">Font attribution</a></li></ul></details></article>",
        escaped(&title), escaped(&build.timestamp), escaped(&build.timestamp.replace('T', " ").replace('Z', "")),
        &build.commit[..7], escaped(&build.id), build.size as f64 / 1_048_576.0, build.sha256
      ));
      count += 1;
    }
    if count == 0 {
      content.push_str("<p class=\"empty\">No builds have been published in this channel yet.</p>");
    }
    content.push_str("</div></section>");
  }
  format!(
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
    <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
    <meta name=\"color-scheme\" content=\"dark light\">\
    <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'self'; base-uri 'none'; form-action 'none'\">\
    <meta name=\"description\" content=\"Download Wodex for Nintendo Switch: timestamped nightly builds and explicit versions.\">\
    <title>Wodex · Downloads</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\
    <a class=\"skip\" href=\"#downloads\">Skip to downloads</a><header><p class=\"eyebrow\">WODEX / NINTENDO SWITCH</p>\
    <h1>Your conversations.<br>Your console.</h1><p class=\"intro\">A native DeepSeek chat client for Nintendo Switch. \
    Pick a recent nightly or an explicit version below.</p><nav aria-label=\"Download channels\">\
    <a href=\"#nightly\">Nightly</a><a href=\"#version\">Versions</a></nav></header>\
    <main id=\"downloads\">{content}</main><footer>Copy <code>wodex.nro</code> to <code>sdmc:/switch/wodex/</code> and open it with the Homebrew Menu. \
    In Wodex, open Settings → Updates to check this site. Versions is the default channel; Nightly is optional. \
    <a href=\"catalog.json\">Build catalog</a></footer></body></html>\n"
  )
}

fn stable(catalog: &Catalog, base: &str) -> serde_json::Value {
  let build = catalog
    .builds
    .iter()
    .filter(|build| build.channel == Channel::Version)
    .max_by_key(|build| version(&build.version).unwrap())
    .map(|build| {
      serde_json::json!({
        "version": build.version,
        "url": format!("{base}/builds/version/{}/wodex.nro", build.version),
        "size": build.size,
        "sha256": build.sha256,
        "notes": format!("Wodex {}.", build.version),
      })
    });
  serde_json::json!({"schema_version":1, "channel":"version", "build":build})
}

fn nightly(catalog: &Catalog, base: &str) -> serde_json::Value {
  let build = catalog
    .builds
    .iter()
    .filter(|build| build.channel == Channel::Nightly)
    .max_by(|left, right| left.timestamp.cmp(&right.timestamp).then_with(|| left.id.cmp(&right.id)))
    .map(|build| {
      serde_json::json!({
        "version": build.version,
        "url": format!("{base}/builds/nightly/{}/wodex.nro", build.id),
        "size": build.size,
        "sha256": build.sha256,
        "notes": format!("Wodex {} nightly, built {}.", build.version, build.timestamp),
        "id": build.id,
        "timestamp": build.timestamp,
        "commit": build.commit,
      })
    });
  serde_json::json!({"schema_version":1, "channel":"nightly", "build":build})
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
  let mut output =
    io(OpenOptions::new().write(true).create_new(true).open(path), "Cannot create site file.")?;
  io(output.write_all(contents), "Cannot write site file.")
}

fn write_site(
  options: &Options,
  catalog: &Catalog,
  staging: &Path,
  incoming: &Build,
) -> Result<()> {
  for build in &catalog.builds {
    let source = if build.channel == incoming.channel && build.id == incoming.id {
      options.artifact_dir.clone()
    } else {
      options
        .previous_dir
        .as_ref()
        .ok_or("Previous build directory is missing.")?
        .join(build.path())
    };
    let destination = staging.join(build.path());
    io(fs::create_dir_all(&destination), "Cannot create download directory.")?;
    for name in FILES {
      safe_path(&source.join(name), false)?;
      io(fs::copy(source.join(name), destination.join(name)), "Cannot copy verified download.")?;
      if asset(&destination.join(name), asset_limit(name))? != build.assets[name] {
        return Err("Copied download failed its integrity check.".into());
      }
    }
  }
  let catalog_json = serde_json::to_vec_pretty(catalog).map_err(|_| "Cannot serialize catalog.")?;
  if catalog_json.len() as u64 > MAX_CATALOG {
    return Err("Catalog exceeds the metadata size limit.".into());
  }
  write(&staging.join("catalog.json"), &catalog_json)?;
  write(&staging.join("index.html"), html(catalog).as_bytes())?;
  write(&staging.join("style.css"), include_bytes!("style.css"))?;
  write(&staging.join(".nojekyll"), b"")?;
  io(fs::create_dir(staging.join("updates")), "Cannot create update metadata directory.")?;
  write(
    &staging.join("updates/stable.json"),
    &serde_json::to_vec_pretty(&stable(catalog, &options.base_url))
      .map_err(|_| "Cannot serialize update feed.")?,
  )?;
  write(
    &staging.join("updates/nightly.json"),
    &serde_json::to_vec_pretty(&nightly(catalog, &options.base_url))
      .map_err(|_| "Cannot serialize update feed.")?,
  )
}

/// Build a complete static snapshot. The destination is created atomically and
/// must not already exist. Prior snapshots are read-only inputs.
pub fn publish(options: &Options) -> Result<Catalog> {
  base_url(&options.base_url)?;
  let compact_time = timestamp(&options.timestamp)?;
  if !hexadecimal(&options.commit, 40) {
    return Err("Commit must contain 40 lowercase hexadecimal characters.".into());
  }
  let (artifact_version, assets) = inspect_artifacts(&options.artifact_dir)?;
  verify_build_info(options, &artifact_version)?;
  match (options.channel, options.tag.as_deref()) {
    (Channel::Nightly, None) => {}
    (Channel::Version, Some(tag)) if tag == format!("v{artifact_version}") => {}
    _ => {
      return Err(
        "Explicit versions require a vX.Y.Z tag matching VERSION; nightlies have no tag.".into(),
      )
    }
  }
  let id = match options.channel {
    Channel::Nightly => format!("{compact_time}-{}", &options.commit[..7]),
    Channel::Version => artifact_version.clone(),
  };
  let incoming = Build {
    channel: options.channel,
    id,
    version: artifact_version,
    timestamp: options.timestamp.clone(),
    commit: options.commit.clone(),
    size: assets["wodex.nro"].size,
    sha256: assets["wodex.nro"].sha256.clone(),
    licenses_sha256: assets["licenses.tar.gz"].sha256.clone(),
    assets,
  };
  let mut catalog = previous(options.previous_dir.as_deref())?;
  if incoming.channel == Channel::Version {
    if let Some(locked) = catalog.version_locks.get(&incoming.id) {
      if locked != &incoming.assets {
        return Err(
          "Published versions are immutable; increment VERSION before publishing different bytes."
            .into(),
        );
      }
    }
    catalog.version_locks.insert(incoming.id.clone(), incoming.assets.clone());
  }
  if let Some(existing) =
    catalog.builds.iter().find(|build| build.channel == incoming.channel && build.id == incoming.id)
  {
    if existing.assets != incoming.assets
      || (incoming.channel == Channel::Nightly && existing.commit != incoming.commit)
    {
      return Err("An existing build identity cannot be replaced with different content.".into());
    }
  } else {
    catalog.builds.push(incoming.clone());
  }
  retain(&mut catalog.builds, &(incoming.channel, incoming.id.clone()))?;
  safe_path(&options.output_dir, true)?;
  if options.output_dir.exists() {
    return Err("Output directory must not already exist.".into());
  }
  let parent = options
    .output_dir
    .parent()
    .filter(|path| !path.as_os_str().is_empty())
    .unwrap_or(Path::new("."));
  safe_directory(parent)?;
  let staging = parent.join(format!(
    ".wodex-pages-{}-{}",
    std::process::id(),
    NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
  ));
  io(fs::create_dir(&staging), "Cannot create staging directory.")?;
  let result = write_site(options, &catalog, &staging, &incoming)
    .and_then(|()| io(fs::rename(&staging, &options.output_dir), "Cannot finalize site snapshot."));
  if result.is_err() {
    let _ = fs::remove_dir_all(&staging);
  }
  result?;
  Ok(catalog)
}

#[cfg(test)]
mod tests;
