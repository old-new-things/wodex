//! Provider policy and request preparation over Tokn's portable provider drivers.

use crate::state::{Message, MessageStatus, Role, Settings};
use alloc::{format, string::String, vec::Vec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokn_core::{
  provider::{HeaderPatchCtx, ProviderRequestKind},
  upstream_url::CleartextHttpPolicy,
  Endpoint,
};
use tokn_endpoint_chat_completions::{
  ChatContent, ChatMessage, ChatRequest, ChatRequestParameters,
};
use tokn_endpoint_core::Role as ChatRole;
use tokn_headers::{AgentId, HeaderMap, TemplateVars};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
  #[default]
  #[serde(rename = "deepseek")]
  DeepSeek,
  #[serde(rename = "openai")]
  OpenAi,
  #[serde(rename = "opencode_go")]
  OpenCodeGo,
  #[serde(rename = "llama_cpp")]
  LlamaCpp,
  Zai,
  ZaiCodingPlan,
  #[serde(rename = "zhipuai")]
  ZhipuAi,
  #[serde(rename = "zhipuai_coding_plan")]
  ZhipuAiCodingPlan,
  #[serde(rename = "openai_compatible")]
  OpenAiCompatible,
}

impl Provider {
  pub const ALL: [Self; 9] = [
    Self::DeepSeek,
    Self::OpenAi,
    Self::OpenCodeGo,
    Self::LlamaCpp,
    Self::Zai,
    Self::ZaiCodingPlan,
    Self::ZhipuAi,
    Self::ZhipuAiCodingPlan,
    Self::OpenAiCompatible,
  ];

  pub fn id(self) -> &'static str {
    match self {
      Self::DeepSeek => "deepseek",
      Self::OpenAi => "openai",
      Self::OpenCodeGo => "opencode_go",
      Self::LlamaCpp => "llama_cpp",
      Self::Zai => "zai",
      Self::ZaiCodingPlan => "zai_coding_plan",
      Self::ZhipuAi => "zhipuai",
      Self::ZhipuAiCodingPlan => "zhipuai_coding_plan",
      Self::OpenAiCompatible => "openai_compatible",
    }
  }

  pub fn is_deepseek(&self) -> bool {
    *self == Self::DeepSeek
  }

  pub fn label(self) -> &'static str {
    match self {
      Self::DeepSeek => "DeepSeek",
      Self::OpenAi => "OpenAI",
      Self::OpenCodeGo => "OpenCode Go",
      Self::LlamaCpp => "llama.cpp",
      Self::Zai => "Z.ai",
      Self::ZaiCodingPlan => "Z.ai Coding Plan",
      Self::ZhipuAi => "Zhipu BigModel",
      Self::ZhipuAiCodingPlan => "Zhipu Coding Plan",
      Self::OpenAiCompatible => "OpenAI-compatible",
    }
  }

  pub fn default_base_url(self) -> &'static str {
    match self {
      Self::DeepSeek => tokn_provider_deepseek::native::DEFAULT_BASE_URL,
      Self::OpenAi | Self::OpenAiCompatible => tokn_provider_openai::native::OPENAI_BASE_URL,
      Self::OpenCodeGo => tokn_provider_opencode::native::OPENCODE_GO_BASE_URL,
      Self::LlamaCpp => tokn_provider_llama_cpp::native::DEFAULT_BASE_URL,
      Self::Zai => tokn_provider_zai::native::ZAI_BASE_URL,
      Self::ZaiCodingPlan => tokn_provider_zai::native::ZAI_CODING_PLAN_BASE_URL,
      Self::ZhipuAi => tokn_provider_zai::native::ZHIPUAI_BASE_URL,
      Self::ZhipuAiCodingPlan => tokn_provider_zai::native::ZHIPUAI_CODING_PLAN_BASE_URL,
    }
  }

  pub fn default_model(self) -> &'static str {
    match self {
      Self::DeepSeek => "deepseek-flash",
      _ => "",
    }
  }

  pub fn default_token_limit_field(self) -> TokenLimitField {
    if self == Self::OpenAi {
      TokenLimitField::MaxCompletionTokens
    } else {
      TokenLimitField::MaxTokens
    }
  }

  pub fn requires_api_key(self) -> bool {
    self != Self::LlamaCpp
  }

  pub fn allows_insecure_http(self) -> bool {
    self == Self::LlamaCpp
  }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireProtocol {
  #[default]
  ChatCompletions,
  Responses,
  Messages,
}

impl WireProtocol {
  pub fn endpoint(self) -> Endpoint {
    match self {
      Self::ChatCompletions => Endpoint::ChatCompletions,
      Self::Responses => Endpoint::Responses,
      Self::Messages => Endpoint::Messages,
    }
  }
}

pub(crate) struct PreparedChatRequest {
  pub url: String,
  pub headers: Vec<(String, String)>,
  pub body: Value,
  pub protocol: WireProtocol,
}

/// Use the same URL admission, credential scheme, body shaping and operation
/// routing as Tokn's standard providers, with Wodex supplying the HTTP worker.
pub(crate) fn prepare_request(
  settings: &Settings,
  history: &[Message],
) -> Result<PreparedChatRequest, String> {
  let provider = settings.provider;
  let protocol = if provider == Provider::OpenCodeGo {
    [WireProtocol::ChatCompletions, WireProtocol::Responses, WireProtocol::Messages]
      .into_iter()
      .find(|protocol| {
        tokn_provider_opencode::native::supports_model_endpoint(
          &settings.model,
          protocol.endpoint(),
          &settings.base_url,
        )
      })
      .ok_or_else(|| String::from("This OpenCode Go model has no supported endpoint."))?
  } else {
    WireProtocol::ChatCompletions
  };
  let endpoint = protocol.endpoint();
  let body = serde_json::to_value(chat_request(settings, history))
    .map_err(|_| String::from("Cannot encode the chat request."))?;
  let body = tokn_convert::convert_request(Endpoint::ChatCompletions, endpoint, &body)
    .map_err(|_| String::from("Cannot convert the chat request to the provider's protocol."))?;
  let cleartext = if provider.allows_insecure_http() && settings.allow_insecure_http {
    CleartextHttpPolicy::Allow
  } else {
    CleartextHttpPolicy::LoopbackOnly
  };
  let headers = HeaderMap::new();
  let prepare = match provider {
    Provider::DeepSeek => tokn_provider_deepseek::native::prepare_with_policy,
    Provider::OpenAi | Provider::OpenAiCompatible => {
      tokn_provider_openai::native::prepare_with_policy
    }
    Provider::OpenCodeGo => tokn_provider_opencode::native::prepare_with_policy,
    Provider::LlamaCpp => tokn_provider_llama_cpp::native::prepare_with_policy,
    Provider::Zai | Provider::ZaiCodingPlan | Provider::ZhipuAi | Provider::ZhipuAiCodingPlan => {
      tokn_provider_zai::native::prepare_with_policy
    }
  };
  let mut prepared =
    prepare(&settings.base_url, endpoint, body, &settings.api_key, headers, cleartext)
      .map_err(|error| format!("Cannot prepare the {} request: {error}", provider.label()))?;
  if prepared.body.len() > 4 * 1024 * 1024 {
    return Err("The provider request exceeds the 4 MiB limit.".into());
  }
  let body: Value = serde_json::from_slice(&prepared.body)
    .map_err(|_| String::from("The provider prepared an invalid request."))?;
  if provider == Provider::OpenCodeGo {
    let inbound = HeaderMap::new();
    let vars = TemplateVars::default();
    let agent = AgentId::from("wodex");
    let context = HeaderPatchCtx {
      request_kind: ProviderRequestKind::Operation(endpoint),
      body: &body,
      bearer_token: None,
      content_encoding: None,
      stream: true,
      initiator: "user",
      inbound_headers: &inbound,
      vars: &vars,
      agent_id: &agent,
    };
    if let Some(headers) =
      tokn_provider_opencode::native::normalize_headers(&mut prepared.headers, &context)
    {
      prepared.headers = headers;
    }
  }
  Ok(PreparedChatRequest {
    url: prepared.url,
    headers: prepared
      .headers
      .into_iter()
      .map(|(name, value)| (name.as_str().into(), value.as_str().into()))
      .collect(),
    body,
    protocol,
  })
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

  fn settings_for(provider: Provider, model: &str) -> Settings {
    Settings {
      provider,
      base_url: provider.default_base_url().into(),
      model: model.into(),
      api_key: if provider.requires_api_key() { "provider-test-key".into() } else { String::new() },
      token_limit_field: provider.default_token_limit_field(),
      ..Settings::default()
    }
  }

  fn header<'a>(request: &'a PreparedChatRequest, name: &str) -> Option<&'a str> {
    request
      .headers
      .iter()
      .find(|(key, _)| key.eq_ignore_ascii_case(name))
      .map(|(_, value)| value.as_str())
  }

  #[test]
  fn named_providers_use_tokn_urls_authentication_and_body_policy() {
    for provider in Provider::ALL {
      let settings = settings_for(provider, "glm-test");
      let request = prepare_request(&settings, &[]).unwrap();
      assert_eq!(request.url, format!("{}/chat/completions", provider.default_base_url()));
      assert_eq!(request.protocol, WireProtocol::ChatCompletions);
      assert_eq!(header(&request, "content-type"), Some("application/json"));
      assert_eq!(header(&request, "accept"), Some("text/event-stream"));
      assert_eq!(
        header(&request, "authorization"),
        if provider.requires_api_key() { Some("Bearer provider-test-key") } else { None }
      );
      assert_eq!(request.body["model"], "glm-test");
      assert_eq!(serde_json::to_value(provider).unwrap(), provider.id());
      assert_eq!(
        serde_json::from_str::<Provider>(&format!("\"{}\"", provider.id())).unwrap(),
        provider
      );
      if matches!(
        provider,
        Provider::Zai | Provider::ZaiCodingPlan | Provider::ZhipuAi | Provider::ZhipuAiCodingPlan
      ) {
        assert_eq!(request.body["thinking"], json!({"type":"enabled", "clear_thinking":false}));
      } else if provider != Provider::DeepSeek {
        assert!(request.body.get("thinking").is_none());
      }
    }
  }

  #[test]
  fn opencode_routes_models_and_converts_history_and_token_limits() {
    for (model, protocol, suffix) in [
      ("glm-test", WireProtocol::ChatCompletions, "chat/completions"),
      ("gpt-test", WireProtocol::Responses, "responses"),
      ("qwen-test", WireProtocol::Messages, "messages"),
    ] {
      let settings = settings_for(Provider::OpenCodeGo, model);
      let history = [Message {
        role: Role::User,
        content: "你好 🌍".into(),
        reasoning_content: String::new(),
        status: MessageStatus::Complete,
        finish_reason: None,
        usage: None,
      }];
      let request = prepare_request(&settings, &history).unwrap();
      assert_eq!(request.protocol, protocol);
      assert_eq!(request.url, format!("{}/{suffix}", settings.base_url));
      match protocol {
        WireProtocol::ChatCompletions => {
          assert_eq!(request.body["messages"][1]["content"], "你好 🌍")
        }
        WireProtocol::Responses => {
          assert_eq!(request.body["instructions"], settings.system_prompt);
          assert_eq!(request.body["max_output_tokens"], settings.max_tokens);
          assert_eq!(request.body["input"][0]["content"][0]["text"], "你好 🌍");
          assert!(request.body.get("messages").is_none());
        }
        WireProtocol::Messages => {
          assert_eq!(request.body["system"], settings.system_prompt);
          assert_eq!(request.body["max_tokens"], settings.max_tokens);
          assert_eq!(request.body["messages"][0]["content"][0]["text"], "你好 🌍");
          assert!(header(&request, "authorization").is_none());
          assert_eq!(header(&request, "x-api-key"), Some("provider-test-key"));
          assert_eq!(header(&request, "anthropic-version"), Some("2023-06-01"));
        }
      }
    }
  }

  #[test]
  fn provider_preparation_rejects_invalid_credentials_and_unsafe_upstream_urls() {
    for provider in Provider::ALL {
      let mut settings = settings_for(provider, "test-model");
      for key in ["bad\r\nx-api-key: wrong", "a b", "中文"] {
        settings.api_key = key.into();
        assert!(
          prepare_request(&settings, &[]).is_err(),
          "{} accepted a header credential",
          provider.id()
        );
      }
      settings.api_key = "test-key".into();
      for url in [
        "http://example.com/v1",
        "https://user:password@example.com/v1",
        "https://example.com/a/../v1",
        "https://example.com/v1?key=secret",
      ] {
        settings.base_url = url.into();
        assert!(prepare_request(&settings, &[]).is_err(), "{} accepted {url}", provider.id());
      }
    }
  }

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
