//! Shared API endpoint policy for saved profiles and the native HTTP worker.
use alloc::string::String;

pub(crate) fn validate(url: &str, allow_lan_http: bool) -> Result<(), String> {
  validate_with_limit(url, allow_lan_http, 2048)
}

pub(crate) fn validate_request(url: &str, allow_lan_http: bool) -> Result<(), String> {
  validate_with_limit(url, allow_lan_http, 4096)
}

fn validate_with_limit(url: &str, allow_lan_http: bool, max_bytes: usize) -> Result<(), String> {
  let fail = || {
    String::from("Use an HTTPS base URL without credentials, query, or fragment. Local llama.cpp HTTP requires its explicit setting.")
  };
  if url.len() > max_bytes
    || !url.is_ascii()
    || url.bytes().any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    || url.bytes().any(|byte| b"?#@\\\"<>".contains(&byte))
  {
    return Err(fail());
  }
  let (secure, remainder) = if let Some(rest) = url.strip_prefix("https://") {
    (true, rest)
  } else if let Some(rest) = url.strip_prefix("http://") {
    (false, rest)
  } else {
    return Err(fail());
  };
  let authority = remainder.split('/').next().unwrap_or_default();
  if authority.is_empty() || remainder.contains('%') {
    return Err(fail());
  }
  let (host, port) = if authority.starts_with('[') {
    let close = authority.find(']').ok_or_else(fail)?;
    let host = &authority[..=close];
    let tail = &authority[close + 1..];
    if host != "[::1]" || (!tail.is_empty() && !tail.starts_with(':')) {
      return Err(fail());
    }
    (host, tail.strip_prefix(':'))
  } else {
    let (host, port) =
      authority.split_once(':').map_or((authority, None), |(host, port)| (host, Some(port)));
    if host.is_empty()
      || !host.bytes().all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
      || host.starts_with('.')
      || host.ends_with('.')
      || host.contains("..")
    {
      return Err(fail());
    }
    (host, port)
  };
  if let Some(port) = port {
    if port.is_empty()
      || !port.bytes().all(|byte| byte.is_ascii_digit())
      || port.parse::<u16>().ok().filter(|&port| port > 0).is_none()
    {
      return Err(fail());
    }
  }
  if !secure
    && !matches!(host, "localhost" | "127.0.0.1" | "[::1]")
    && !(allow_lan_http && is_lan_host(host))
  {
    return Err(fail());
  }
  if remainder.contains("//") || remainder.split('/').skip(1).any(|part| matches!(part, "." | ".."))
  {
    return Err(fail());
  }
  Ok(())
}

/// Cleartext is opt-in and limited to private IPv4 addresses and mDNS names.
pub(crate) fn is_lan_host(host: &str) -> bool {
  let normalized = host.to_ascii_lowercase();
  let host = normalized.as_str();
  if let Some(name) = host.strip_suffix(".local") {
    return !name.is_empty() && !name.starts_with('.') && !name.ends_with('.');
  }
  let mut octets = [0u8; 4];
  let mut parts = host.split('.');
  for octet in &mut octets {
    let Some(part) = parts.next() else { return false };
    if part.is_empty()
      || (part.len() > 1 && part.starts_with('0'))
      || !part.bytes().all(|byte| byte.is_ascii_digit())
    {
      return false;
    }
    let Ok(value) = part.parse::<u8>() else { return false };
    *octet = value;
  }
  parts.next().is_none()
    && (octets[0] == 10
      || (octets[0] == 172 && (16..=31).contains(&octets[1]))
      || (octets[0] == 192 && octets[1] == 168))
}
