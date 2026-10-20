use alloc::{string::String, vec::Vec};
use serde_json::Value;

const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_EVENT_BYTES: usize = 512 * 1024;

#[derive(Default)]
pub(crate) struct Stream {
  line: Vec<u8>,
  data: Vec<u8>,
  seen_line: bool,
  skip_lf: bool,
  pub done: bool,
  pub finish_reason: Option<String>,
  pub usage: Option<Value>,
  pub total_bytes: usize,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Event {
  Delta { content: String, reasoning: String },
  Error(String),
}

impl Stream {
  /// SSE is split as bytes first so a UTF-8 codepoint may span any number of
  /// network callbacks. CR, LF, and CRLF are valid SSE line endings.
  pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    for &byte in bytes {
      if self.done {
        break;
      }
      if self.skip_lf {
        self.skip_lf = false;
        if byte == b'\n' {
          continue;
        }
      }
      if byte == b'\n' || byte == b'\r' {
        self.skip_lf = byte == b'\r';
        let result = self.end_line();
        if !self.record(result, &mut events) {
          break;
        }
        self.line.clear();
      } else {
        if self.line.len() >= MAX_LINE_BYTES {
          events.push(Event::Error("The server sent an oversized stream line.".into()));
          self.done = true;
          break;
        }
        self.line.push(byte);
      }
    }
    events
  }

  pub fn finish(&mut self) -> Vec<Event> {
    let mut events = Vec::new();
    if !self.done {
      if !self.line.is_empty() {
        let result = self.end_line();
        if !self.record(result, &mut events) {
          return events;
        }
        self.line.clear();
      }
      let result = self.dispatch();
      self.record(result, &mut events);
    }
    events
  }

  fn record(&mut self, result: Result<Option<Event>, String>, events: &mut Vec<Event>) -> bool {
    match result {
      Ok(Some(event)) => {
        let failed = matches!(event, Event::Error(_));
        events.push(event);
        if failed {
          self.done = true;
        }
        !failed
      }
      Ok(None) => true,
      Err(error) => {
        events.push(Event::Error(error));
        self.done = true;
        false
      }
    }
  }

  fn end_line(&mut self) -> Result<Option<Event>, String> {
    if !self.seen_line {
      self.seen_line = true;
      if self.line.starts_with(&[0xef, 0xbb, 0xbf]) {
        self.line.drain(..3);
      }
    }
    if self.line.is_empty() {
      return self.dispatch();
    }
    if self.line[0] == b':' {
      return Ok(None);
    }
    let (field, value) = match self.line.iter().position(|&byte| byte == b':') {
      Some(index) => {
        let value = &self.line[index + 1..];
        (&self.line[..index], value.strip_prefix(b" ").unwrap_or(value))
      }
      None => (self.line.as_slice(), &[][..]),
    };
    if field == b"data" {
      if self.data.len() + value.len() + 1 > MAX_EVENT_BYTES {
        return Err("The server sent an oversized stream event.".into());
      }
      self.data.extend_from_slice(value);
      self.data.push(b'\n');
    }
    Ok(None)
  }

  fn dispatch(&mut self) -> Result<Option<Event>, String> {
    if self.data.is_empty() {
      return Ok(None);
    }
    let data = core::mem::take(&mut self.data);
    let text = core::str::from_utf8(&data)
      .map_err(|_| String::from("The server sent invalid UTF-8."))?
      .trim();
    if text == "[DONE]" {
      self.done = true;
      return Ok(None);
    }
    if text.is_empty() {
      return Ok(None);
    }
    let value: Value = serde_json::from_str(text)
      .map_err(|_| String::from("The server sent an invalid JSON stream event."))?;
    if !value.is_object() {
      return Err("The server sent a non-object stream event.".into());
    }
    if let Some(error) = value.get("error") {
      let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("The API returned an error.");
      return Ok(Some(Event::Error(message.into())));
    }
    if let Some(usage) = value.get("usage").filter(|usage| usage.is_object()) {
      // Save only numeric counters instead of arbitrary server-provided data.
      let mut counters = serde_json::Map::new();
      for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
        if let Some(value) = usage.get(key).and_then(Value::as_u64) {
          counters.insert(key.into(), value.into());
        }
      }
      self.usage = Some(Value::Object(counters));
    }
    let Some(choices) = value.get("choices").and_then(Value::as_array) else {
      return Ok(None);
    };
    let choice = choices
      .iter()
      .find(|choice| choice.get("index").and_then(Value::as_u64) == Some(0))
      .or_else(|| choices.first().filter(|choice| choice.get("index").is_none()));
    let Some(choice) = choice else { return Ok(None) };
    if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
      if reason.is_empty() || reason.len() > 64 || reason.chars().any(char::is_control) {
        return Err("The server sent an invalid finish reason.".into());
      }
      self.finish_reason = Some(reason.into());
    }
    let Some(delta) = choice.get("delta") else { return Ok(None) };
    let content = read_text(delta, "content")?;
    let reasoning = read_text(delta, "reasoning_content")?;
    if content.is_empty() && reasoning.is_empty() {
      return Ok(None);
    }
    Ok(Some(Event::Delta { content, reasoning }))
  }
}

fn read_text(value: &Value, key: &str) -> Result<String, String> {
  match value.get(key) {
    None | Some(Value::Null) => Ok(String::new()),
    Some(Value::String(text)) if !text.contains('\0') => Ok(text.clone()),
    _ => Err("The server sent a non-text chat delta.".into()),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use alloc::{format, vec};

  #[test]
  fn handles_every_split_and_utf8() {
    let wire = ": keepalive\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"思考\"}}]}\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"你好 👋\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
    let expected = vec![
      Event::Delta { content: String::new(), reasoning: "思考".into() },
      Event::Delta { content: "你好 👋".into(), reasoning: String::new() },
    ];
    for split in 0..=wire.len() {
      let mut stream = Stream::default();
      let mut events = stream.feed(&wire.as_bytes()[..split]);
      events.extend(stream.feed(&wire.as_bytes()[split..]));
      assert_eq!(events, expected, "split {split}");
      assert!(stream.done);
      assert_eq!(stream.finish_reason.as_deref(), Some("stop"));
    }
    let mut stream = Stream::default();
    let mut events = Vec::new();
    for byte in wire.as_bytes() {
      events.extend(stream.feed(&[*byte]));
    }
    assert_eq!(events, expected);
  }

  #[test]
  fn multiline_data_usage_and_final_line() {
    let mut stream = Stream::default();
    stream.feed(b"data: {\nignored: value\ndata: \"choices\":[],\ndata: \"usage\":{\"total_tokens\":42,\"other\":\"secret\"}}\n\n");
    assert_eq!(stream.usage, Some(serde_json::json!({"total_tokens":42})));
    stream.feed(b"data: [DONE]");
    stream.finish();
    assert!(stream.done);
  }

  #[test]
  fn accepts_bom_and_bare_carriage_returns_across_every_split() {
    let wire =
      "\u{feff}data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\r\rdata: [DONE]\r\r";
    for split in 0..=wire.len() {
      let mut stream = Stream::default();
      let mut events = stream.feed(&wire.as_bytes()[..split]);
      events.extend(stream.feed(&wire.as_bytes()[split..]));
      assert_eq!(events, vec![Event::Delta { content: "你好".into(), reasoning: String::new() }]);
      assert!(stream.done);
    }
  }

  #[test]
  fn rejects_bad_events_and_surfaces_api_errors() {
    for bytes in [
      &b"data: \xff\n\n"[..],
      &b"data: nope\n\n"[..],
      &b"data: []\n\n"[..],
      &b"data: {\"choices\":[{\"delta\":{\"content\":42}}]}\n\n"[..],
    ] {
      assert!(matches!(Stream::default().feed(bytes).last(), Some(Event::Error(_))));
    }
    let events = Stream::default().feed(b"data: {\"error\":{\"message\":\"Bad key\"}}\n\n");
    assert_eq!(events, vec![Event::Error("Bad key".into())]);
    assert!(matches!(
      Stream::default().feed(format!("data: {}", "x".repeat(MAX_LINE_BYTES)).as_bytes()).last(),
      Some(Event::Error(_))
    ));
  }
}
