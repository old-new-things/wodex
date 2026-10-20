use std::{collections::BTreeMap, env, path::PathBuf, process};
use wodex_publish_pages::{publish, Channel, Options};

fn arguments() -> Result<Options, String> {
  let mut values = BTreeMap::new();
  let mut args = env::args().skip(1);
  while let Some(key) = args.next() {
    if ![
      "--artifact-dir",
      "--previous-dir",
      "--output-dir",
      "--channel",
      "--timestamp",
      "--commit",
      "--base-url",
      "--tag",
    ]
    .contains(&key.as_str())
    {
      return Err("Unknown argument; see tools/publish-pages/README.md.".into());
    }
    let value = args.next().ok_or("Each argument requires a value.")?;
    if values.insert(key, value).is_some() {
      return Err("Duplicate argument.".into());
    }
  }
  let mut take = |name| values.remove(name).ok_or_else(|| format!("Missing argument: {name}"));
  let artifact_dir = PathBuf::from(take("--artifact-dir")?);
  let output_dir = PathBuf::from(take("--output-dir")?);
  let channel = match take("--channel")?.as_str() {
    "nightly" => Channel::Nightly,
    "version" => Channel::Version,
    _ => return Err("Channel must be nightly or version.".into()),
  };
  let timestamp = take("--timestamp")?;
  let commit = take("--commit")?;
  let base_url = take("--base-url")?;
  Ok(Options {
    artifact_dir,
    output_dir,
    channel,
    timestamp,
    commit,
    base_url,
    previous_dir: values.remove("--previous-dir").map(PathBuf::from),
    tag: values.remove("--tag"),
  })
}

fn main() {
  match arguments().and_then(|options| publish(&options)) {
    Ok(catalog) => {
      println!("Published a verified catalog containing {} builds.", catalog.builds.len())
    }
    Err(error) => {
      eprintln!("Publication failed: {error}");
      process::exit(1);
    }
  }
}
