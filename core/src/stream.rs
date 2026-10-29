use alloc::{string::String, vec::Vec};
use serde_json::Value;
use tokn_convert::{
  ir::IrDelta,
  sse::{SseEvent, TerminalState},
};
use tokn_endpoint_chat_completions::ChatChunk;
use tokn_endpoint_core::FinishReason;

use crate::provider::WireProtocol;

const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_EVENT_BYTES: usize = 512 * 1024;
// Responses repeats the completed output in its terminal JSON event. A
// 512 KiB displayed answer can occupy about 3 MiB after JSON escaping.
const MAX_RESPONSES_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Stream {
  line: Vec<u8>,
  data: Vec<u8>,
  event_name: Vec<u8>,
  protocol: WireProtocol,
  native_terminal: Option<TerminalState>,
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
  pub fn new(protocol: WireProtocol) -> Self {
    Self {
      protocol,
      native_terminal: (protocol != WireProtocol::ChatCompletions)
        .then(|| TerminalState::new(protocol.endpoint())),
      ..Self::default()
    }
  }

  /// Chat Completions retains its historical finish-only compatibility. Native
  /// protocols must observe their own final lifecycle event before completing.
  pub fn is_complete(&self) -> bool {
    self.done || (self.protocol == WireProtocol::ChatCompletions && self.finish_reason.is_some())
  }

  /// SSE is split as bytes first so a UTF-8 codepoint may span any number of
  /// network callbacks. CR, LF, and CRLF are valid SSE line endings.
  pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
    let mut events = Vec::new();
    let line_limit = if self.protocol == WireProtocol::Responses {
      MAX_RESPONSES_EVENT_BYTES
    } else {
      MAX_LINE_BYTES
    };
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
        if self.line.len() >= line_limit {
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
      if !self.done {
        if let Some(terminal) = &self.native_terminal {
          if terminal.finish().is_err() {
            events.push(Event::Error(
              "The connection ended before the provider's final response event. You can retry."
                .into(),
            ));
            self.done = true;
          }
        }
      }
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
      let event_limit = if self.protocol == WireProtocol::Responses {
        MAX_RESPONSES_EVENT_BYTES
      } else {
        MAX_EVENT_BYTES
      };
      if self.data.len() + value.len() + 1 > event_limit {
        return Err("The server sent an oversized stream event.".into());
      }
      self.data.extend_from_slice(value);
      self.data.push(b'\n');
    } else if field == b"event" && self.protocol != WireProtocol::ChatCompletions {
      self.event_name = value.to_vec();
    }
    Ok(None)
  }

  fn dispatch(&mut self) -> Result<Option<Event>, String> {
    let event_name = core::mem::take(&mut self.event_name);
    if self.data.is_empty() {
      return Ok(None);
    }
    let data = core::mem::take(&mut self.data);
    let text = core::str::from_utf8(&data)
      .map_err(|_| String::from("The server sent invalid UTF-8."))?
      .trim();
    if text == "[DONE]" {
      if self.protocol != WireProtocol::ChatCompletions {
        return Err("The provider sent [DONE] before its final response event.".into());
      }
      self.done = true;
      return Ok(None);
    }
    if text.is_empty() {
      return Ok(None);
    }
    let mut value: Value = serde_json::from_str(text)
      .map_err(|_| String::from("The server sent an invalid JSON stream event."))?;
    if !value.is_object() {
      return Err("The server sent a non-object stream event.".into());
    }
    if self.protocol != WireProtocol::ChatCompletions {
      return self.dispatch_native(value, &event_name);
    }
    if let Some(error) = value.get("error") {
      let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .unwrap_or("The API returned an error.");
      return Ok(Some(Event::Error(message.into())));
    }
    normalize_chunk(&mut value);
    let chunk: ChatChunk = serde_json::from_value(value)
      .map_err(|_| String::from("The server sent an invalid chat stream event."))?;
    if let Some(usage) = chunk.usage {
      let mut counters = serde_json::Map::new();
      for (key, value) in [
        ("prompt_tokens", usage.prompt_tokens),
        ("completion_tokens", usage.completion_tokens),
        ("total_tokens", usage.total_tokens),
      ] {
        if let Some(value) = value {
          counters.insert(key.into(), value.into());
        }
      }
      self.usage = Some(Value::Object(counters));
    }
    let Some(choice) = chunk.choices.into_iter().next() else { return Ok(None) };
    if let Some(reason) = choice.finish_reason {
      let reason = match reason {
        FinishReason::Stop => String::from("stop"),
        FinishReason::Length => String::from("length"),
        FinishReason::ToolCalls => String::from("tool_calls"),
        FinishReason::ContentFilter => String::from("content_filter"),
        FinishReason::Other(reason) => reason,
      };
      self.set_finish_reason(reason)?;
    }
    let content = choice.delta.content.unwrap_or_default();
    let reasoning = choice.delta.reasoning_content.unwrap_or_default();
    if content.contains('\0') || reasoning.contains('\0') {
      return Err("The server sent a NUL character in its response.".into());
    }
    if content.is_empty() && reasoning.is_empty() {
      return Ok(None);
    }
    Ok(Some(Event::Delta { content, reasoning }))
  }

  fn set_finish_reason(&mut self, reason: String) -> Result<(), String> {
    if reason.is_empty() || reason.len() > 64 || reason.chars().any(char::is_control) {
      return Err("The server sent an invalid finish reason.".into());
    }
    self.finish_reason = Some(reason);
    Ok(())
  }

  /// Tokn normalizes provider-native deltas without accumulating a second
  /// copy of the reply. Wodex retains its own display and persistence bounds.
  fn dispatch_native(&mut self, value: Value, event_name: &[u8]) -> Result<Option<Event>, String> {
    let name = core::str::from_utf8(event_name)
      .map_err(|_| String::from("The provider sent an invalid SSE event name."))?;
    let event = SseEvent {
      event: (!name.is_empty()).then(|| String::from(name)),
      data: String::new(),
      json: Some(value),
    };
    let terminal = self.native_terminal.as_mut().expect("native protocol has terminal tracking");
    let completed = terminal
      .observe(&event)
      .map_err(|_| String::from("The provider sent a failed or inconsistent stream event."))?;
    let value = event.json.as_ref().expect("native event is JSON");
    validate_native_delta(self.protocol, value)?;
    let mut deltas = match self.protocol {
      WireProtocol::Responses => tokn_convert::value::responses::delta_from_responses_event(value),
      WireProtocol::Messages => tokn_convert::value::messages::delta_from_messages_event(value),
      WireProtocol::ChatCompletions => unreachable!(),
    };
    // Tokn's current Responses converter omits refusal text. Preserve it as
    // visible answer text instead of completing with a blank assistant reply.
    if self.protocol == WireProtocol::Responses
      && value.get("type").and_then(Value::as_str) == Some("response.refusal.delta")
    {
      deltas.push(IrDelta::Text(
        value.get("delta").and_then(Value::as_str).expect("validated refusal text").into(),
      ));
    }
    let mut content = String::new();
    let mut reasoning = String::new();
    for delta in deltas {
      match delta {
        IrDelta::Text(text) => content.push_str(&text),
        IrDelta::Reasoning(text) => reasoning.push_str(&text),
        IrDelta::Finish(Some(reason)) => {
          if reason == "tool_calls" {
            return Err(
              "The provider requested a tool call; Wodex supports text chat only.".into(),
            );
          }
          self.set_finish_reason(reason)?;
        }
        IrDelta::Finish(None) => {}
        IrDelta::Usage(usage) => {
          let counters = self
            .usage
            .get_or_insert_with(|| Value::Object(Default::default()))
            .as_object_mut()
            .expect("usage contains normalized counters");
          for (key, count) in [
            ("prompt_tokens", usage.input_tokens),
            ("completion_tokens", usage.output_tokens),
            ("total_tokens", usage.total_tokens),
          ] {
            if let Some(count) = count {
              counters.insert(key.into(), count.into());
            }
          }
          if self.protocol == WireProtocol::Messages {
            if let Some(total) = counters
              .get("prompt_tokens")
              .and_then(Value::as_u64)
              .zip(counters.get("completion_tokens").and_then(Value::as_u64))
              .and_then(|(input, output)| input.checked_add(output))
            {
              counters.insert("total_tokens".into(), total.into());
            }
          }
        }
        IrDelta::ToolCall { .. } => {
          return Err("The provider requested a tool call; Wodex supports text chat only.".into())
        }
      }
    }
    if content.contains('\0') || reasoning.contains('\0') {
      return Err("The server sent a NUL character in its response.".into());
    }
    if completed {
      self.done = true;
      if self.finish_reason.is_none() {
        self.finish_reason = Some("stop".into());
      }
    }
    if content.is_empty() && reasoning.is_empty() {
      Ok(None)
    } else {
      Ok(Some(Event::Delta { content, reasoning }))
    }
  }
}

fn validate_native_delta(protocol: WireProtocol, value: &Value) -> Result<(), String> {
  let kind = value
    .get("type")
    .and_then(Value::as_str)
    .ok_or_else(|| String::from("The provider's stream event has no valid type."))?;
  let text = match (protocol, kind) {
    (
      WireProtocol::Responses,
      "response.output_text.delta"
      | "response.refusal.delta"
      | "response.reasoning_summary_text.delta"
      | "response.reasoning_text.delta",
    ) => Some(value.get("delta")),
    (WireProtocol::Messages, "content_block_delta") => {
      let delta = value
        .get("delta")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("The provider sent an invalid content delta."))?;
      match delta.get("type").and_then(Value::as_str) {
        Some("text_delta") => Some(delta.get("text")),
        Some("thinking_delta") => Some(delta.get("thinking")),
        Some(_) => None,
        None => return Err("The provider sent an invalid content delta type.".into()),
      }
    }
    _ => None,
  };
  if text.is_some_and(|text| !text.is_some_and(Value::is_string)) {
    return Err("The provider sent a non-text content delta.".into());
  }
  Ok(())
}

// Preserve Wodex's compatibility policy before decoding Tokn's typed schema:
// retain only choice zero, allow finish-only chunks, and ignore nonnumeric
// usage extensions. Bounds are enforced by the SSE framer before this step.
fn normalize_chunk(value: &mut Value) {
  let object = value.as_object_mut().expect("stream event was checked as an object");
  if let Some(Value::Object(usage)) = object.remove("usage") {
    let mut counters = serde_json::Map::new();
    for key in ["prompt_tokens", "completion_tokens", "total_tokens"] {
      if let Some(value) = usage.get(key).and_then(Value::as_u64) {
        counters.insert(key.into(), value.into());
      }
    }
    object.insert("usage".into(), Value::Object(counters));
  }
  let mut selected = Vec::new();
  if let Some(Value::Array(mut choices)) = object.remove("choices") {
    let index = choices
      .iter()
      .position(|choice| choice.get("index").and_then(Value::as_u64) == Some(0))
      .or_else(|| choices.first().filter(|choice| choice.get("index").is_none()).map(|_| 0));
    if let Some(index) = index {
      let mut choice = choices.swap_remove(index);
      if let Some(object) = choice.as_object_mut() {
        if object.get("delta").is_none_or(Value::is_null) {
          object.insert("delta".into(), Value::Object(Default::default()));
        }
      }
      selected.push(choice);
    }
  }
  object.insert("choices".into(), Value::Array(selected));
}

#[cfg(test)]
mod tests {
  use super::*;
  use alloc::{format, vec};

  fn collect_native(protocol: WireProtocol, wire: &str, split: usize) -> (Stream, Vec<Event>) {
    let mut stream = Stream::new(protocol);
    let mut events = stream.feed(&wire.as_bytes()[..split]);
    events.extend(stream.feed(&wire.as_bytes()[split..]));
    events.extend(stream.finish());
    (stream, events)
  }

  #[test]
  fn native_responses_preserve_fragmented_text_reasoning_usage_and_terminal() {
    let wire = concat!(
      "event: response.reasoning_summary_text.delta\r\ndata: {\"type\":\"response.reasoning_summary_text.delta\",\"delta\":\"思考\"}\r\n\r\n",
      "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好 🌍\"}\n\n",
      "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":7,\"total_tokens\":11}}}\n\n",
    );
    for split in 0..=wire.len() {
      let (stream, events) = collect_native(WireProtocol::Responses, wire, split);
      assert_eq!(
        events,
        vec![
          Event::Delta { content: String::new(), reasoning: "思考".into() },
          Event::Delta { content: "你好 🌍".into(), reasoning: String::new() },
        ],
        "split {split}"
      );
      assert!(stream.done);
      assert_eq!(stream.finish_reason.as_deref(), Some("stop"));
      assert_eq!(
        stream.usage,
        Some(serde_json::json!({"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}))
      );
    }
  }

  #[test]
  fn native_messages_merge_partial_usage_and_require_message_stop() {
    let wire = concat!(
      "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":4,\"output_tokens\":0,\"private\":\"drop\"}}}\n\n",
      "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"想\"}}\n\n",
      "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"你好 🌍\"}}\n\n",
      "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},\"usage\":{\"output_tokens\":7}}\n\n",
      "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    for split in 0..=wire.len() {
      let (stream, events) = collect_native(WireProtocol::Messages, wire, split);
      assert_eq!(
        events,
        vec![
          Event::Delta { content: String::new(), reasoning: "想".into() },
          Event::Delta { content: "你好 🌍".into(), reasoning: String::new() },
        ],
        "split {split}"
      );
      assert!(stream.done);
      assert_eq!(stream.finish_reason.as_deref(), Some("length"));
      assert_eq!(
        stream.usage,
        Some(serde_json::json!({"prompt_tokens":4,"completion_tokens":7,"total_tokens":11}))
      );
    }
  }

  #[test]
  fn native_finish_metadata_does_not_replace_the_required_terminal_event() {
    let mut native = Stream::new(WireProtocol::Messages);
    native.feed(b"data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n");
    assert_eq!(native.finish_reason.as_deref(), Some("stop"));
    assert!(!native.is_complete());
    native.feed(b"data: {\"type\":\"message_stop\"}\n\n");
    assert!(native.is_complete());

    let mut compatible = Stream::default();
    compatible.feed(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
    assert!(compatible.is_complete());
  }

  #[test]
  fn native_streams_reject_early_eof_failure_wrong_terminal_and_mismatched_event_names() {
    for (protocol, wire) in [
      (
        WireProtocol::Responses,
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
      ),
      (
        WireProtocol::Messages,
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
      ),
      (WireProtocol::Responses, "data: [DONE]\n\n"),
      (WireProtocol::Messages, "data: [DONE]\n\n"),
      (
        WireProtocol::Responses,
        "event: response.failed\ndata: {\"type\":\"response.completed\"}\n\n",
      ),
      (WireProtocol::Responses, "data: {\"type\":\"response.incomplete\"}\n\n"),
      (WireProtocol::Messages, "event: error\ndata: {\"type\":\"message_stop\"}\n\n"),
      (
        WireProtocol::Messages,
        "data: {\"type\":\"error\",\"error\":{\"message\":\"private-key\"}}\n\n",
      ),
    ] {
      let (_, events) = collect_native(protocol, wire, wire.len() / 2);
      assert!(matches!(events.last(), Some(Event::Error(_))), "{protocol:?}: {wire}");
      assert!(!format!("{events:?}").contains("private-key"));
    }
  }

  #[test]
  fn native_streams_validate_text_and_reject_unsupported_tool_calls() {
    for (protocol, value) in [
      (
        WireProtocol::Responses,
        serde_json::json!({"type":"response.output_text.delta","delta":42}),
      ),
      (WireProtocol::Responses, serde_json::json!({"type":"response.refusal.delta","delta":false})),
      (
        WireProtocol::Responses,
        serde_json::json!({"type":"response.output_text.delta","delta":"has\0nul"}),
      ),
      (
        WireProtocol::Messages,
        serde_json::json!({"type":"content_block_delta","delta":{"type":"text_delta","text":false}}),
      ),
      (
        WireProtocol::Messages,
        serde_json::json!({"type":"content_block_start","content_block":{"type":"tool_use","name":"exec"}}),
      ),
      (
        WireProtocol::Responses,
        serde_json::json!({"type":"response.function_call_arguments.delta","delta":"{}"}),
      ),
    ] {
      let wire = format!("data: {value}\n\n");
      let (_, events) = collect_native(protocol, &wire, wire.len());
      assert!(matches!(events.last(), Some(Event::Error(_))), "{protocol:?}: {wire}");
    }
  }

  #[test]
  fn native_responses_display_refusals_and_still_require_a_successful_terminal() {
    let wire = concat!(
      "event: response.refusal.delta\ndata: {\"type\":\"response.refusal.delta\",\"delta\":\"不能帮助处理此请求。\"}\n\n",
      "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n",
    );
    for split in 0..=wire.len() {
      let (stream, events) = collect_native(WireProtocol::Responses, wire, split);
      assert_eq!(
        events,
        vec![Event::Delta {
          content: "不能帮助处理此请求。".into(), reasoning: String::new()
        }]
      );
      assert!(stream.done);
      assert_eq!(stream.finish_reason.as_deref(), Some("stop"));
    }
  }

  #[test]
  fn responses_terminal_snapshots_fit_display_limits_without_repeating_text() {
    for text in ["a".repeat(300 * 1024), "\u{1}".repeat(512 * 1024)] {
      let mut stream = Stream::new(WireProtocol::Responses);
      let mut displayed = String::new();
      for chunk in text.as_bytes().chunks(16 * 1024) {
        let value = serde_json::json!({"type":"response.output_text.delta", "delta":core::str::from_utf8(chunk).unwrap()});
        for event in stream.feed(format!("data: {value}\n\n").as_bytes()) {
          match event {
            Event::Delta { content, reasoning } => {
              displayed.push_str(&content);
              assert!(reasoning.is_empty());
            }
            Event::Error(error) => panic!("{error}"),
          }
        }
      }
      let terminal = serde_json::json!({
        "type":"response.completed",
        "response":{
          "status":"completed",
          "output":[{"type":"message", "content":[{"type":"output_text", "text":text}]}]
        }
      });
      let wire = format!("data: {terminal}\n\n");
      assert!(wire.len() > MAX_LINE_BYTES);
      for chunk in wire.as_bytes().chunks(16 * 1024) {
        assert!(stream.feed(chunk).is_empty(), "terminal snapshot repeated the answer or failed");
      }
      assert_eq!(displayed, text);
      assert!(stream.is_complete());
      assert!(stream.finish().is_empty());
    }
  }

  #[test]
  fn responses_larger_framing_limits_still_reject_oversized_data_and_event_names() {
    for prefix in ["data: ", "event: "] {
      let mut stream = Stream::new(WireProtocol::Responses);
      let wire = format!("{prefix}{}", "x".repeat(MAX_RESPONSES_EVENT_BYTES));
      assert!(matches!(stream.feed(wire.as_bytes()).last(), Some(Event::Error(_))));
    }
  }

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

  #[test]
  fn compatible_chunks_keep_primary_choice_and_finish_only_events() {
    let mut stream = Stream::default();
    let events = stream.feed(
      concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"fallback\"}},",
        "{\"index\":1,\"delta\":{\"content\":\"other\"}},",
        "{\"index\":0,\"delta\":{\"content\":\"primary\"}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"finish_reason\":\"custom_stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":-1,",
        "\"total_tokens\":\"invalid\",\"provider_secret\":\"discard\"}}\n\n",
        "data: [DONE]\n\n"
      )
      .as_bytes(),
    );
    assert_eq!(events, vec![Event::Delta { content: "primary".into(), reasoning: String::new() }]);
    assert_eq!(stream.finish_reason.as_deref(), Some("custom_stop"));
    assert_eq!(stream.usage, Some(serde_json::json!({"prompt_tokens":4})));
    assert!(stream.done);
  }

  #[test]
  fn typed_events_reject_invalid_text_and_finish_metadata() {
    for value in [
      serde_json::json!({"choices":[{"delta":{"content":["not text"]}}]}),
      serde_json::json!({"choices":[{"delta":{"reasoning_content":false}}]}),
      serde_json::json!({"choices":[{"delta":{"content":"contains\0nul"}}]}),
      serde_json::json!({"choices":[{"delta":{},"finish_reason":""}]}),
      serde_json::json!({"choices":[{"delta":{},"finish_reason":"x".repeat(65)}]}),
    ] {
      let mut stream = Stream::default();
      assert!(matches!(
        stream.feed(format!("data: {value}\n\n").as_bytes()).last(),
        Some(Event::Error(_))
      ));
      assert!(stream.done);
    }
  }
}
