use super::*;

struct Fixture(PathBuf);

impl Fixture {
  fn new() -> Self {
    let root = fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
      "wodex-publisher-test-{}-{}",
      std::process::id(),
      NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    Self(root)
  }

  fn artifact(&self, name: &str, version: &str, byte: u8) -> PathBuf {
    let dir = self.0.join(name);
    fs::create_dir(&dir).unwrap();
    let mut nro = vec![byte; 256];
    nro[0x10..0x14].copy_from_slice(b"NRO0");
    nro[0x18..0x1c].copy_from_slice(&128_u32.to_le_bytes());
    fs::write(dir.join("wodex.nro"), nro).unwrap();
    fs::write(dir.join("VERSION"), format!("{version}\n")).unwrap();
    fs::write(dir.join("licenses.tar.gz"), [0x1f, 0x8b, 0, 0, 0]).unwrap();
    for name in ["LICENSE", "THIRD_PARTY.md", "NotoSansCJK-OFL.txt", "NotoSansCJK-README.md"] {
      fs::write(dir.join(name), "Required upstream license notices.\n").unwrap();
    }
    self.checksum(&dir, "wodex.nro");
    self.checksum(&dir, "licenses.tar.gz");
    dir
  }

  fn checksum(&self, dir: &Path, name: &str) {
    let hash = asset(&dir.join(name), asset_limit(name)).unwrap().sha256;
    fs::write(dir.join(format!("{name}.sha256")), format!("{hash}  {name}\n")).unwrap();
  }

  fn options(&self, artifact: PathBuf, output: &str) -> Options {
    Options {
      artifact_dir: artifact,
      previous_dir: None,
      output_dir: self.0.join(output),
      channel: Channel::Nightly,
      timestamp: "2026-10-20T12:34:56Z".into(),
      commit: "0123456789abcdef0123456789abcdef01234567".into(),
      base_url: "https://old-new-things.github.io/wodex".into(),
      tag: None,
    }
  }

  fn build_info(&self, options: &Options) -> serde_json::Value {
    serde_json::json!({
      "schema_version": 1,
      "version": fs::read_to_string(options.artifact_dir.join("VERSION")).unwrap().trim(),
      "channel": options.channel,
      "timestamp": options.timestamp,
      "commit": options.commit,
    })
  }

  fn publish(&self, options: &Options) -> Result<Catalog> {
    fs::write(
      options.artifact_dir.join("build-info.json"),
      serde_json::to_vec(&self.build_info(options)).unwrap(),
    )
    .unwrap();
    publish(options)
  }
}

impl Drop for Fixture {
  fn drop(&mut self) {
    fs::remove_dir_all(&self.0).unwrap();
  }
}

#[test]
fn timestamps_are_calendar_validated_and_ids_are_deterministic() {
  assert_eq!(timestamp("2024-02-29T23:59:59Z").unwrap(), "20240229T235959Z");
  for invalid in [
    "2023-02-29T23:59:59Z",
    "2100-02-29T00:00:00Z",
    "2026-04-31T00:00:00Z",
    "2026-01-00T00:00:00Z",
    "2026-13-01T00:00:00Z",
    "2026-10-20T24:00:00Z",
    "2026-10-20T00:60:00Z",
    "2026-10-20T00:00:60Z",
    "2026-10-20T00:00:00+00:00",
    "2026-10-20T00:00:00.000Z",
    "../../unsafe",
  ] {
    assert!(timestamp(invalid).is_err(), "{invalid}");
  }
  let fixture = Fixture::new();
  let options = fixture.options(fixture.artifact("artifact", "0.2.0", 0), "site");
  let catalog = fixture.publish(&options).unwrap();
  assert_eq!(catalog.builds[0].id, "20261020T123456Z-0123456");
  assert!(options.output_dir.join(catalog.builds[0].path()).join("wodex.nro").is_file());
  assert_eq!(
    stable(&catalog, &options.base_url),
    serde_json::json!({"schema_version":1,"channel":"version","build":null})
  );
}

#[test]
fn artifact_integrity_and_header_are_checked_before_output_is_created() {
  let fixture = Fixture::new();
  for (index, tamper) in
    ["digest", "header", "size", "version", "gzip", "checksum_path"].iter().enumerate()
  {
    let artifact = fixture.artifact(&format!("artifact{index}"), "0.2.0", 0);
    match *tamper {
      "digest" => fs::write(artifact.join("wodex.nro.sha256"), "bad").unwrap(),
      "header" => {
        let mut binary = fs::read(artifact.join("wodex.nro")).unwrap();
        binary[0x10] = 0;
        fs::write(artifact.join("wodex.nro"), binary).unwrap();
        fixture.checksum(&artifact, "wodex.nro");
      }
      "size" => {
        let mut binary = fs::read(artifact.join("wodex.nro")).unwrap();
        binary[0x18..0x1c].copy_from_slice(&1000_u32.to_le_bytes());
        fs::write(artifact.join("wodex.nro"), binary).unwrap();
        fixture.checksum(&artifact, "wodex.nro");
      }
      "version" => fs::write(artifact.join("VERSION"), "0.2.0\nextra\n").unwrap(),
      "gzip" => {
        fs::write(artifact.join("licenses.tar.gz"), "not gzip").unwrap();
        fixture.checksum(&artifact, "licenses.tar.gz");
      }
      "checksum_path" => {
        let hash = asset(&artifact.join("wodex.nro"), MAX_NRO).unwrap().sha256;
        fs::write(artifact.join("wodex.nro.sha256"), format!("{hash}  ../wodex.nro\n")).unwrap();
      }
      _ => unreachable!(),
    }
    let options = fixture.options(artifact, &format!("site{index}"));
    assert!(fixture.publish(&options).is_err(), "{tamper}");
    assert!(!options.output_dir.exists());
  }
}

#[test]
fn incoming_build_identity_is_required_and_must_match_the_source_build() {
  let fixture = Fixture::new();
  let options = fixture.options(fixture.artifact("artifact", "0.3.0", 0), "site");
  assert!(publish(&options).is_err(), "Artifacts without a build identity record are rejected.");
  let info_path = options.artifact_dir.join("build-info.json");
  let original = fixture.build_info(&options);
  for (field, value) in [
    ("schema_version", serde_json::json!(2)),
    ("version", serde_json::json!("0.2.0")),
    ("channel", serde_json::json!("version")),
    ("timestamp", serde_json::json!("2026-10-19T12:34:56Z")),
    ("commit", serde_json::json!("a".repeat(40))),
    ("unknown", serde_json::json!("ignored?")),
  ] {
    let mut info = original.clone();
    info[field] = value;
    fs::write(&info_path, serde_json::to_vec(&info).unwrap()).unwrap();
    assert!(publish(&options).is_err(), "{field}");
    assert!(!options.output_dir.exists());
  }
  for invalid in [
    format!("{{\"schema_version\":1,\"schema_version\":1,\"version\":\"0.3.0\",\"channel\":\"nightly\",\"timestamp\":\"{}\",\"commit\":\"{}\"}}", options.timestamp, options.commit),
    "[".into(),
    " ".repeat(1025),
  ] {
    fs::write(&info_path, invalid).unwrap();
    assert!(publish(&options).is_err());
  }
  let catalog = fixture.publish(&options).unwrap();
  let destination = options.output_dir.join(catalog.builds[0].path());
  assert!(
    !destination.join("build-info.json").exists(),
    "CI identity metadata is not a public asset."
  );
  assert_eq!(catalog.builds[0].assets.len(), 9);
}

#[test]
fn versions_reject_unsafe_values_and_feed_uses_semantic_order() {
  for invalid in
    ["../1", "01.2.0", "1.2", "1.2.3-rc1", "v1.2.3", "1.2.3\n", "1.2.3/evil", "9999999999999.0.1"]
  {
    assert!(version(invalid).is_err(), "{invalid}");
  }
  let fixture = Fixture::new();
  let mut previous_dir = None;
  let mut catalog = Catalog::default();
  for (index, value) in ["0.9.0", "0.10.0", "0.8.0"].iter().enumerate() {
    let mut options = fixture
      .options(fixture.artifact(&format!("artifact{index}"), value, 0), &format!("site{index}"));
    options.channel = Channel::Version;
    options.tag = Some(format!("v{value}"));
    options.previous_dir = previous_dir;
    catalog = fixture.publish(&options).unwrap();
    previous_dir = Some(options.output_dir);
  }
  let feed = stable(&catalog, "https://old-new-things.github.io/wodex");
  assert_eq!(feed["build"]["version"], "0.10.0");
  assert_eq!(
    feed["build"]["url"],
    "https://old-new-things.github.io/wodex/builds/version/0.10.0/wodex.nro"
  );
  assert_eq!(feed["build"]["sha256"], catalog.builds[0].sha256);
  assert_eq!(
    feed["build"].as_object().unwrap().keys().cloned().collect::<Vec<_>>(),
    ["notes", "sha256", "size", "url", "version"],
    "Versions feed retains the strict schema consumed by older Wodex apps."
  );
  assert_eq!(
    nightly(&catalog, "https://old-new-things.github.io/wodex"),
    serde_json::json!({"schema_version":1,"channel":"nightly","build":null})
  );
}

#[test]
fn nightly_feed_uses_build_identity_and_timestamp_order_independent_of_version() {
  let fixture = Fixture::new();
  let mut options = fixture.options(fixture.artifact("artifact", "0.3.0", 0), "first");
  let first = fixture.publish(&options).unwrap();
  options.previous_dir = Some(options.output_dir.clone());
  options.output_dir = fixture.0.join("second");
  options.timestamp = "2026-10-21T00:00:00Z".into();
  options.commit = "a".repeat(40);
  let mut catalog = fixture.publish(&options).unwrap();
  assert_eq!(first.builds[0].version, catalog.builds[0].version);
  assert_ne!(first.builds[0].id, catalog.builds[0].id);
  let expected = serde_json::json!({
    "schema_version": 1,
    "channel": "nightly",
    "build": {
      "version": "0.3.0",
      "url": "https://old-new-things.github.io/wodex/builds/nightly/20261021T000000Z-aaaaaaa/wodex.nro",
      "size": catalog.builds[0].size,
      "sha256": catalog.builds[0].sha256,
      "notes": "Wodex 0.3.0 nightly, built 2026-10-21T00:00:00Z.",
      "id": "20261021T000000Z-aaaaaaa",
      "timestamp": "2026-10-21T00:00:00Z",
      "commit": "a".repeat(40),
    }
  });
  let stored: serde_json::Value =
    serde_json::from_slice(&fs::read(options.output_dir.join("updates/nightly.json")).unwrap())
      .unwrap();
  assert_eq!(stored, expected);
  catalog.builds.reverse();
  catalog.builds[0].version = "0.9.0".into();
  assert_eq!(nightly(&catalog, &options.base_url), expected);
  assert_eq!(stable(&catalog, &options.base_url)["build"], serde_json::Value::Null);
}

#[test]
fn immutable_versions_accept_retries_and_reject_changes_in_every_file() {
  let fixture = Fixture::new();
  let artifact = fixture.artifact("artifact", "0.2.0", 0);
  let mut options = fixture.options(artifact.clone(), "first");
  options.channel = Channel::Version;
  options.tag = Some("v0.2.0".into());
  let original = fixture.publish(&options).unwrap();
  options.previous_dir = Some(options.output_dir.clone());
  options.output_dir = fixture.0.join("second");
  options.timestamp = "2026-10-21T00:00:00Z".into();
  options.commit = "a".repeat(40);
  let retry = fixture.publish(&options).unwrap();
  assert_eq!(retry, original, "A retry preserves the original publication metadata.");
  fs::write(artifact.join("LICENSE"), "changed license bytes").unwrap();
  options.output_dir = fixture.0.join("third");
  assert!(fixture.publish(&options).unwrap_err().contains("immutable"));
  assert!(!options.output_dir.exists());
}

#[test]
fn pruned_versions_keep_immutable_records_and_latest_versions_are_retained() {
  let fixture = Fixture::new();
  let mut options = fixture.options(fixture.artifact("artifact", "0.0.0", 0), "site0");
  options.channel = Channel::Version;
  let mut result = Catalog::default();
  for index in 0..12 {
    let value = format!("0.{index}.0");
    fs::write(options.artifact_dir.join("VERSION"), format!("{value}\n")).unwrap();
    options.tag = Some(format!("v{value}"));
    options.output_dir = fixture.0.join(format!("site{index}"));
    result = fixture.publish(&options).unwrap();
    options.previous_dir = Some(options.output_dir.clone());
  }
  assert_eq!(result.builds.len(), 10);
  assert_eq!(result.builds[0].version, "0.11.0");
  assert_eq!(result.builds[9].version, "0.2.0");
  assert_eq!(result.version_locks.len(), 12);
  assert!(!options.output_dir.join("builds/version/0.0.0").exists());
  fs::write(options.artifact_dir.join("VERSION"), "0.0.0\n").unwrap();
  fs::write(options.artifact_dir.join("LICENSE"), "different old version content").unwrap();
  options.tag = Some("v0.0.0".into());
  options.output_dir = fixture.0.join("reused");
  assert!(fixture.publish(&options).unwrap_err().contains("immutable"));
}

#[test]
fn nightlies_retain_seven_by_timestamp_without_changing_versions() {
  let fixture = Fixture::new();
  let mut options = fixture.options(fixture.artifact("artifact", "0.2.1", 0), "version");
  options.channel = Channel::Version;
  options.tag = Some("v0.2.1".into());
  let original = fixture.publish(&options).unwrap();
  let original_feed = stable(&original, &options.base_url);
  options.previous_dir = Some(options.output_dir.clone());
  options.channel = Channel::Nightly;
  options.tag = None;
  let mut result = original.clone();
  for index in 1..=9 {
    // A newer nightly can have a lower base version than the preceding build.
    fs::write(options.artifact_dir.join("VERSION"), format!("0.{}.0\n", 10 - index)).unwrap();
    options.timestamp = format!("2026-10-{index:02}T00:00:00Z");
    options.output_dir = fixture.0.join(format!("site{index}"));
    result = fixture.publish(&options).unwrap();
    options.previous_dir = Some(options.output_dir.clone());
  }
  // A delayed older workflow must not restore an expired nightly or change the feed.
  fs::write(options.artifact_dir.join("VERSION"), "99.0.0\n").unwrap();
  options.timestamp = "2026-10-02T12:00:00Z".into();
  options.output_dir = fixture.0.join("delayed");
  let before_delayed = result.clone();
  result = fixture.publish(&options).unwrap();
  assert_eq!(result, before_delayed);

  let nightlies: Vec<_> =
    result.builds.iter().filter(|build| build.channel == Channel::Nightly).collect();
  assert_eq!(nightlies.len(), 7);
  assert_eq!(nightlies[0].id, "20261009T000000Z-0123456");
  assert_eq!(nightlies[6].id, "20261003T000000Z-0123456");
  let actual_directories: BTreeSet<_> = fs::read_dir(options.output_dir.join("builds/nightly"))
    .unwrap()
    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
    .collect();
  let expected_directories = nightlies.iter().map(|build| build.id.clone()).collect();
  assert_eq!(actual_directories, expected_directories, "Expired payloads must also be removed.");
  assert!(
    fixture.0.join("site1/builds/nightly/20261001T000000Z-0123456/wodex.nro").is_file(),
    "Previously accepted snapshots remain intact for recovery."
  );
  let published_feed: serde_json::Value =
    serde_json::from_slice(&fs::read(options.output_dir.join("updates/nightly.json")).unwrap())
      .unwrap();
  assert_eq!(published_feed, nightly(&result, &options.base_url));
  assert_eq!(published_feed["build"]["id"], nightlies[0].id);
  assert_eq!(published_feed["build"]["version"], "0.1.0");
  assert_eq!(stable(&result, &options.base_url), original_feed);
  assert_eq!(result.version_locks, original.version_locks);
  assert_eq!(
    result.builds.iter().find(|build| build.channel == Channel::Version),
    original.builds.first(),
  );
}

#[test]
fn total_size_budget_preserves_latest_in_both_channels() {
  let fixture = Fixture::new();
  let options = fixture.options(fixture.artifact("artifact", "0.2.0", 0), "site");
  let template = fixture.publish(&options).unwrap().builds.remove(0);
  let mut builds = Vec::new();
  for channel in [Channel::Nightly, Channel::Version] {
    for index in 1..=5 {
      let mut build = template.clone();
      build.channel = channel;
      build.version = format!("0.{index}.0");
      build.timestamp = format!("2026-10-{index:02}T00:00:00Z");
      build.id = match channel {
        Channel::Nightly => format!("202610{index:02}T000000Z-0123456"),
        Channel::Version => build.version.clone(),
      };
      build.assets.get_mut("wodex.nro").unwrap().size = MAX_NRO;
      builds.push(build);
    }
  }
  retain(&mut builds, &(Channel::Nightly, "20261005T000000Z-0123456".into())).unwrap();
  assert!(builds.iter().map(Build::total_size).sum::<u64>() <= MAX_SITE);
  assert!(builds.iter().any(|build| build.channel == Channel::Nightly && build.version == "0.5.0"));
  assert!(builds.iter().any(|build| build.channel == Channel::Version && build.version == "0.5.0"));
}

#[test]
fn restored_snapshot_rejects_tampering_and_path_injection() {
  let fixture = Fixture::new();
  let mut options = fixture.options(fixture.artifact("artifact", "0.2.0", 0), "first");
  let catalog = fixture.publish(&options).unwrap();
  options.previous_dir = Some(options.output_dir.clone());
  options.output_dir = fixture.0.join("second");
  options.timestamp = "2026-10-21T00:00:00Z".into();
  let previous = options.previous_dir.as_ref().unwrap();
  let download = previous.join(catalog.builds[0].path()).join("LICENSE");
  fs::write(&download, "tampered").unwrap();
  assert!(fixture.publish(&options).unwrap_err().contains("do not match"));
  fs::copy(options.artifact_dir.join("LICENSE"), download).unwrap();
  let mut damaged = catalog.clone();
  damaged.builds[0].id = "../../outside".into();
  fs::write(previous.join("catalog.json"), serde_json::to_vec(&damaged).unwrap()).unwrap();
  assert!(fixture.publish(&options).unwrap_err().contains("identity"));
  damaged = catalog.clone();
  let injected_asset = damaged.builds[0].assets["LICENSE"].clone();
  damaged.builds[0].assets.insert("../../outside".into(), injected_asset);
  fs::write(previous.join("catalog.json"), serde_json::to_vec(&damaged).unwrap()).unwrap();
  assert!(fixture.publish(&options).unwrap_err().contains("supported download"));
  damaged = catalog.clone();
  damaged.builds.push(damaged.builds[0].clone());
  fs::write(previous.join("catalog.json"), serde_json::to_vec(&damaged).unwrap()).unwrap();
  assert!(fixture.publish(&options).unwrap_err().contains("duplicate"));
  assert!(!options.output_dir.exists());
}

#[cfg(unix)]
#[test]
fn file_and_parent_symlinks_are_rejected() {
  use std::os::unix::fs::symlink;
  let fixture = Fixture::new();
  let artifact = fixture.artifact("artifact", "0.2.0", 0);
  let source = artifact.join("LICENSE");
  fs::rename(&source, fixture.0.join("original")).unwrap();
  symlink(fixture.0.join("original"), &source).unwrap();
  assert!(inspect_artifacts(&artifact).unwrap_err().contains("Symbolic"));
  fs::remove_file(&source).unwrap();
  fs::rename(fixture.0.join("original"), &source).unwrap();
  symlink(&artifact, fixture.0.join("linked")).unwrap();
  assert!(inspect_artifacts(&fixture.0.join("linked")).unwrap_err().contains("Symbolic"));
  let mut options = fixture.options(artifact, "linked/new-site");
  assert!(fixture.publish(&options).unwrap_err().contains("Symbolic"));
  options.output_dir = fixture.0.join("artifact/../new-site");
  assert!(fixture.publish(&options).unwrap_err().contains("Unsafe"));
}

#[test]
fn urls_tags_commits_and_html_are_safe() {
  for invalid in [
    "http://old-new-things.github.io/wodex",
    "https://old-new-things.github.io/wodex/",
    "https://old-new-things.github.io/../wodex",
    "https://evil.example/wodex",
    "https://old-new-things.github.io/wodex?actor=someone",
    "https://old-new-things.github.io/wodex#fragment",
    "https://name@old-new-things.github.io/wodex",
    "https://old-new-things.github.io/wodex/anything",
  ] {
    assert!(base_url(invalid).is_err(), "{invalid}");
  }
  assert_eq!(escaped("<a href=\"x\">&'</a>"), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;");
  let fixture = Fixture::new();
  let mut options = fixture.options(fixture.artifact("artifact", "0.2.0", 0), "first");
  options.channel = Channel::Version;
  options.tag = Some("v0.3.0".into());
  assert!(fixture.publish(&options).unwrap_err().contains("matching VERSION"));
  options.tag = Some("v0.2.0".into());
  options.commit = "<script>".repeat(5);
  assert!(fixture.publish(&options).unwrap_err().contains("Commit"));
  options.commit = "a".repeat(40);
  let catalog = fixture.publish(&options).unwrap();
  let page = html(&catalog);
  assert_eq!(page.matches("<section ").count(), 2);
  assert!(page.contains("<h2 id=\"nightly\">Nightly</h2>"));
  assert!(page.contains("At most 7 nightlies are kept; older downloads are removed automatically."));
  assert!(page.contains("<h2 id=\"version\">Versions</h2>"));
  assert!(!page.contains("<script"));
  assert!(!page.contains("https://"));
  assert!(!page.contains(&fixture.0.to_string_lossy().to_string()));
  assert!(fixture.publish(&options).unwrap_err().contains("must not already exist"));
}
