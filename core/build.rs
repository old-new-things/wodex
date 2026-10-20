#[path = "src/build_metadata.rs"]
mod build_metadata;

use std::env;

fn main() {
  let names = ["WODEX_BUILD_CHANNEL", "WODEX_BUILD_TIMESTAMP", "WODEX_BUILD_COMMIT"];
  let values = names.map(|name| {
    println!("cargo:rerun-if-env-changed={name}");
    match env::var(name) {
      Ok(value) => Some(value).filter(|value| !value.is_empty()),
      Err(env::VarError::NotPresent) => None,
      Err(env::VarError::NotUnicode(_)) => panic!("Wodex build identity must be UTF-8."),
    }
  });
  println!("cargo:rerun-if-changed=src/build_metadata.rs");
  println!("cargo:rerun-if-changed=build.rs");
  let [channel, timestamp, commit] = match values {
    [None, None, None] => ["development".into(), String::new(), String::new()],
    [Some(channel), Some(timestamp), Some(commit)] => {
      assert!(matches!(channel.as_str(), "version" | "nightly"), "Invalid Wodex build channel.");
      assert!(build_metadata::valid_timestamp(&timestamp), "Invalid Wodex build timestamp.");
      assert!(build_metadata::valid_commit(&commit), "Invalid Wodex build commit.");
      [channel, timestamp, commit]
    }
    _ => panic!("Wodex build channel, timestamp and commit must be supplied together."),
  };
  println!("cargo:rustc-env=WODEX_BUILT_CHANNEL={channel}");
  println!("cargo:rustc-env=WODEX_BUILT_TIMESTAMP={timestamp}");
  println!("cargo:rustc-env=WODEX_BUILT_COMMIT={commit}");
}
