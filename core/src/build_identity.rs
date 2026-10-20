//! Immutable identity compiled into the executable; settings cannot override it.
use alloc::{format, string::String};

use crate::update::UpdateChannel;

#[derive(Clone, Copy, Debug)]
pub(crate) struct BuildIdentity<'a> {
  pub channel: Option<UpdateChannel>,
  pub timestamp: &'a str,
  pub commit: &'a str,
}

impl BuildIdentity<'static> {
  pub fn embedded() -> Self {
    Self {
      channel: match env!("WODEX_BUILT_CHANNEL") {
        "nightly" => Some(UpdateChannel::Nightly),
        "version" => Some(UpdateChannel::Version),
        _ => None,
      },
      timestamp: env!("WODEX_BUILT_TIMESTAMP"),
      commit: env!("WODEX_BUILT_COMMIT"),
    }
  }

  #[cfg(test)]
  pub const fn development() -> Self {
    Self { channel: None, timestamp: "", commit: "" }
  }
}

/// Inputs must have passed build_metadata's validation before constructing IDs.
pub(crate) fn nightly_id(timestamp: &str, commit: &str) -> String {
  let compact: String =
    timestamp.chars().filter(|character| !matches!(character, '-' | ':')).collect();
  format!("{compact}-{}", &commit[..7])
}

pub(crate) fn label(version: &str) -> String {
  let identity = BuildIdentity::embedded();
  if identity.channel == Some(UpdateChannel::Nightly) {
    format!("{version} nightly {}", nightly_id(identity.timestamp, identity.commit))
  } else {
    String::from(version)
  }
}
