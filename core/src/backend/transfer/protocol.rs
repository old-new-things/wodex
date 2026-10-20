use crate::state::MAX_SAVED_BYTES;
use alloc::{format, string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Method {
  Get,
  Post,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Request<'a> {
  pub method: Method,
  pub path: &'a str,
  pub content_length: usize,
}

impl<'a> Request<'a> {
  pub fn parse(bytes: &'a [u8], authority: &str, code: &str) -> Result<Self, u16> {
    let text = core::str::from_utf8(bytes).map_err(|_| 400u16)?;
    if !text.is_ascii() || !text.ends_with("\r\n\r\n") || bytes.len() > super::HEADER_LIMIT {
      return Err(400);
    }
    let mut lines = text[..text.len() - 4].split("\r\n");
    let mut parts = lines.next().ok_or(400u16)?.split(' ');
    let method = match parts.next() {
      Some("GET") => Method::Get,
      Some("POST") => Method::Post,
      _ => return Err(405),
    };
    let path = parts.next().ok_or(400u16)?;
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() || !path.starts_with('/') {
      return Err(400);
    }
    let mut headers = Vec::new();
    for line in lines {
      let (name, value) = line.split_once(':').ok_or(400u16)?;
      if name.is_empty()
        || !name.bytes().all(header_token)
        || value.bytes().any(|byte| (byte < 32 && byte != b'\t') || byte == 127)
        || headers.iter().any(|(seen, _): &(&str, &str)| seen.eq_ignore_ascii_case(name))
        || headers.len() >= 64
      {
        return Err(400);
      }
      headers.push((name, value.trim_matches([' ', '\t'])));
    }
    let header = |name: &str| {
      headers
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
        .map(|(_, value)| *value)
    };
    if header("host") != Some(authority) {
      return Err(403);
    }
    if let Some(origin) = header("origin") {
      if origin != format!("http://{authority}") {
        return Err(403);
      }
    }
    if let Some(site) = header("sec-fetch-site") {
      if site != "same-origin" && site != "none" {
        return Err(403);
      }
    }
    if header("transfer-encoding").is_some() || header("expect").is_some() {
      return Err(400);
    }
    if header("content-encoding").is_some_and(|encoding| !encoding.eq_ignore_ascii_case("identity"))
    {
      return Err(415);
    }
    let content_length = match header("content-length") {
      Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
        value.parse::<usize>().map_err(|_| 413u16)?
      }
      Some(_) => return Err(400),
      None if method == Method::Post => return Err(411),
      None => 0,
    };
    if content_length > MAX_SAVED_BYTES {
      return Err(413);
    }
    if method == Method::Get && content_length != 0 {
      return Err(400);
    }
    let api = path.starts_with("/api/");
    if api {
      let valid = header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|provided| constant_time_equal(provided.as_bytes(), code.as_bytes()));
      if !valid {
        return Err(401);
      }
    }
    if method == Method::Post {
      if path != "/api/config" && path != "/api/sessions" {
        return Err(404);
      }
      let media = header("content-type").and_then(|value| value.split(';').next()).unwrap_or("");
      if !media.trim().eq_ignore_ascii_case("application/json") {
        return Err(415);
      }
      if content_length == 0 {
        return Err(400);
      }
    }
    Ok(Self { method, path, content_length })
  }
}

fn header_token(byte: u8) -> bool {
  byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
  if left.len() != right.len() {
    return false;
  }
  left.iter().zip(right).fold(0u8, |difference, (a, b)| difference | (a ^ b)) == 0
}

pub(super) struct Response<'a> {
  status: u16,
  content_type: &'static str,
  filename: Option<&'static str>,
  pub body: &'a [u8],
}

impl<'a> Response<'a> {
  pub fn error(status: u16) -> Self {
    let message: &'static [u8] = match status {
      400 => b"Malformed transfer request.",
      401 => b"Pairing code is missing or incorrect.",
      403 => b"This request is not from the PC transfer address.",
      404 => b"Transfer endpoint not found.",
      405 => b"Request method is not supported.",
      409 => b"An import is already waiting. Confirm or cancel it on your Switch.",
      411 => b"Content-Length is required.",
      413 => b"The document exceeds the 4 MiB limit.",
      415 => b"Upload an uncompressed JSON document.",
      431 => b"Request headers are too large.",
      _ => b"PC transfer failed.",
    };
    Self { status, content_type: "text/plain; charset=utf-8", filename: None, body: message }
  }

  pub fn asset(content_type: &'static str, body: &'a [u8]) -> Self {
    Self { status: 200, content_type, filename: None, body }
  }

  pub fn json(status: u16, body: &'a [u8]) -> Self {
    Self { status, content_type: "application/json; charset=utf-8", filename: None, body }
  }

  pub fn download(body: &'a [u8], config: bool) -> Self {
    Self {
      filename: Some(if config { "wodex-config.json" } else { "wodex-sessions.json" }),
      ..Self::json(200, body)
    }
  }

  pub fn header(&self) -> String {
    let reason = match self.status {
      200 => "OK",
      202 => "Accepted",
      400 => "Bad Request",
      401 => "Unauthorized",
      403 => "Forbidden",
      404 => "Not Found",
      405 => "Method Not Allowed",
      409 => "Conflict",
      411 => "Length Required",
      413 => "Content Too Large",
      415 => "Unsupported Media Type",
      431 => "Request Header Fields Too Large",
      _ => "Internal Server Error",
    };
    let mut header = format!(
      "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\
       Connection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
       Referrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\n\
       Cross-Origin-Resource-Policy: same-origin\r\n\
       Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; \
       connect-src 'self'; img-src 'self' data:; base-uri 'none'; frame-ancestors 'none'; \
       form-action 'self'\r\n",
      self.status,
      self.content_type,
      self.body.len()
    );
    if let Some(filename) = self.filename {
      header.push_str(&format!("Content-Disposition: attachment; filename=\"{filename}\"\r\n"));
    }
    header.push_str("\r\n");
    header
  }
}
