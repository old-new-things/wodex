//! Provider policy over Tokn's portable Chat Completions wire types.

use crate::state::{Message, MessageStatus, Role, Settings};
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokn_endpoint_chat_completions::{
  ChatContent, ChatMessage, ChatRequest, ChatRequestParameters,
};
use tokn_endpoint_core::Role as ChatRole;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  #[default]
  #[serde(rename = "deepseek")]
  DeepSeek,
  #[serde(rename = "openai_compatible")]
  OpenAiCompatible,
}

impl Provider {
  pub fn is_deepseek(&self) -> bool {
    *self == Self::DeepSeek
  }

  pub fn label(self) -> &'static str {
    match self {
      Self::DeepSeek => "DeepSeek",
      Self::OpenAiCompatible => "OpenAI-compatible",
    }
  }

  pub fn default_base_url(self) -> &'static str {
    match self {
      Self::DeepSeek => "https://api.deepseek.com",
      Self::OpenAiCompatible => "https://api.openai.com/v1",
    }
  }

  pub fn default_model(self) -> &'static str {
    match self {
      Self::DeepSeek => "deepseek-flash",
      Self::OpenAiCompatible => "",
    }
  }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenLimitField {
  #[default]
  MaxTokens,
  MaxCompletionTokens,
}

impl TokenLimitField {
  pub fn is_max_tokens(&self) -> bool {
    *self == Self::MaxTokens
  }

  pub fn label(self) -> &'static str {
    match self {
      Self::MaxTokens => "max_tokens",
      Self::MaxCompletionTokens => "max_completion_tokens",
    }
  }
}

pub(crate) fn chat_request(settings: &Settings, history: &[Message]) -> ChatRequest {
  let mut messages = Vec::with_capacity(history.len() + 1);
  if !settings.system_prompt.is_empty() {
    messages.push(text_message(ChatRole::System, &settings.system_prompt));
  }
  for message in history.iter().filter(|message| message.status == MessageStatus::Complete) {
    let role = match message.role {
      Role::User => ChatRole::User,
      Role::Assistant => ChatRole::Assistant,
    };
    // Reasoning stays display-only; the chat history contains completed text.
    messages.push(text_message(role, &message.content));
  }
  let mut params = ChatRequestParameters::default();
  let mut extras = serde_json::Map::new();
  let token_limit = u64::from(settings.max_tokens);
  if settings.provider == Provider::DeepSeek {
    set_legacy_token_limit(&mut params, token_limit);
    params.thinking = Some(json!({
      "type": if settings.thinking { "enabled" } else { "disabled" }
    }));
    params.reasoning_effort = settings.thinking.then(|| String::from("high"));
    extras.insert("stream_options".into(), json!({"include_usage": true}));
  } else {
    match settings.token_limit_field {
      TokenLimitField::MaxTokens => set_legacy_token_limit(&mut params, token_limit),
      TokenLimitField::MaxCompletionTokens => params.max_completion_tokens = Some(token_limit),
    }
  }
  ChatRequest {
    model: settings.model.clone(),
    messages,
    stream: Some(true),
    tools: Vec::new(),
    stop: None,
    params,
    extra_params: Default::default(),
    extras,
  }
}

// DeepSeek and many compatible services still require this wire field. The
// choice is explicit rather than sending both mutually exclusive limits.
#[allow(deprecated)]
fn set_legacy_token_limit(params: &mut ChatRequestParameters, limit: u64) {
  params.max_tokens = Some(limit);
}

fn text_message(role: ChatRole, content: &str) -> ChatMessage {
  ChatMessage {
    role,
    content: Some(ChatContent::Text(content.into())),
    name: None,
    tool_call_id: None,
    tool_calls: Vec::new(),
    reasoning_content: None,
    extras: Default::default(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn generic_requests_use_only_the_selected_token_limit_and_no_deepseek_options() {
    for (token_limit_field, selected_field, omitted_field) in [
      (TokenLimitField::MaxTokens, "max_tokens", "max_completion_tokens"),
      (TokenLimitField::MaxCompletionTokens, "max_completion_tokens", "max_tokens"),
    ] {
      let settings = Settings {
        provider: Provider::OpenAiCompatible,
        token_limit_field,
        model: "custom-model".into(),
        thinking: true,
        ..Settings::default()
      };
      let wire = serde_json::to_value(chat_request(&settings, &[])).unwrap();
      assert_eq!(wire["model"], "custom-model");
      assert_eq!(wire["stream"], true);
      assert_eq!(wire[selected_field], settings.max_tokens);
      assert!(wire.get(omitted_field).is_none());
      for field in ["thinking", "reasoning_effort", "stream_options", "tools", "stop"] {
        assert!(wire.get(field).is_none(), "unexpected field: {field}");
      }
    }
  }

  #[test]
  fn deepseek_keeps_its_thinking_and_usage_contract() {
    for thinking in [false, true] {
      let settings = Settings {
        thinking,
        token_limit_field: TokenLimitField::MaxCompletionTokens,
        ..Settings::default()
      };
      let wire = serde_json::to_value(chat_request(&settings, &[])).unwrap();
      assert_eq!(wire["thinking"]["type"], if thinking { "enabled" } else { "disabled" });
      assert_eq!(wire["stream_options"]["include_usage"], true);
      assert_eq!(wire["max_tokens"], settings.max_tokens);
      assert!(wire.get("max_completion_tokens").is_none());
      assert_eq!(
        wire.get("reasoning_effort").and_then(|value| value.as_str()),
        thinking.then_some("high")
      );
    }
  }

  #[test]
  fn history_keeps_completed_text_and_omits_partial_messages_and_reasoning() {
    let history = [
      Message {
        role: Role::User,
        content: "你好 🌍".into(),
        reasoning_content: String::new(),
        status: MessageStatus::Complete,
        finish_reason: None,
        usage: None,
      },
      Message {
        role: Role::Assistant,
        content: "Complete answer".into(),
        reasoning_content: "Private reasoning".into(),
        status: MessageStatus::Complete,
        finish_reason: Some("stop".into()),
        usage: Some(json!({"total_tokens": 30})),
      },
      Message {
        role: Role::Assistant,
        content: "Partial answer".into(),
        reasoning_content: String::new(),
        status: MessageStatus::Interrupted,
        finish_reason: None,
        usage: None,
      },
    ];
    let settings = Settings { system_prompt: "Be concise".into(), ..Settings::default() };
    let wire = serde_json::to_value(chat_request(&settings, &history)).unwrap();
    assert_eq!(
      wire["messages"],
      json!([
        {"role":"system", "content":"Be concise"},
        {"role":"user", "content":"你好 🌍"},
        {"role":"assistant", "content":"Complete answer"}
      ])
    );
  }
}
