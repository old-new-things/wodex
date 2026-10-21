use super::*;
use transfer::TransferChannel;

fn apply(core: &mut WodexCore, patch: Value) -> Value {
  core.call(&json!({"op": "settings", "settings": patch}).to_string())
}

#[test]
fn legacy_settings_keep_deepseek_and_the_previous_save_shape() {
  let mut original = WodexCore::default();
  assert_eq!(
    apply(&mut original, json!({"api_key":"saved-deepseek-key", "thinking":true}))["ok"],
    true
  );
  let storage = original.execute(Command::Export {}).unwrap()["data"].clone();
  assert!(storage["settings"].get("provider").is_none());
  assert!(storage["settings"].get("token_limit_field").is_none());
  let mut restored = WodexCore::new(Some(&storage.to_string()));
  assert_eq!(restored.state.settings.provider, Provider::DeepSeek);
  let request = restored.call(r#"{"op":"send","content":"hello"}"#);
  assert_eq!(request["request"]["api_key"], "saved-deepseek-key");
  assert_eq!(request["request"]["body"]["thinking"]["type"], "enabled");
}

#[test]
fn changing_provider_resets_credentials_and_provider_options_only() {
  let mut core = WodexCore::default();
  apply(
    &mut core,
    json!({"api_key":"previous-key", "thinking":true, "system_prompt":"Keep instructions", "max_tokens":1234}),
  );
  let conversations = serde_json::to_value(&core.state.conversations).unwrap();
  assert_eq!(apply(&mut core, json!({"provider":"openai_compatible"}))["ok"], true);
  let settings = &core.state.settings;
  assert_eq!(settings.provider, Provider::OpenAiCompatible);
  assert!(settings.api_key.is_empty());
  assert!(settings.model.is_empty());
  assert!(!settings.thinking);
  assert_eq!(settings.base_url, "https://api.openai.com/v1");
  assert_eq!(settings.system_prompt, "Keep instructions");
  assert_eq!(settings.max_tokens, 1234);
  assert_eq!(serde_json::to_value(&core.state.conversations).unwrap(), conversations);
  apply(
    &mut core,
    json!({"api_key":"generic-key", "model":"custom-model", "base_url":"https://example.invalid/api/v2", "token_limit_field":"max_completion_tokens"}),
  );
  let configured = serde_json::to_value(&core.state.settings).unwrap();
  assert_eq!(apply(&mut core, json!({"provider":"openai_compatible"}))["ok"], true);
  assert_eq!(serde_json::to_value(&core.state.settings).unwrap(), configured);
  assert_eq!(apply(&mut core, json!({"provider":"deepseek"}))["ok"], true);
  assert!(core.state.settings.api_key.is_empty());
  assert_eq!(core.state.settings.model, "deepseek-flash");
  assert_eq!(core.state.settings.base_url, "https://api.deepseek.com");
  assert_eq!(core.state.settings.token_limit_field, TokenLimitField::MaxTokens);
}

#[test]
fn generic_setup_can_be_saved_but_incomplete_setup_cannot_start_a_request() {
  let mut core = WodexCore::default();
  assert_eq!(
    apply(&mut core, json!({"provider":"openai_compatible", "api_key":"generic-key"}))["ok"],
    true
  );
  let exported = core.execute(Command::Export {}).unwrap()["data"].clone();
  let mut restored = WodexCore::new(Some(&exported.to_string()));
  let response = restored.call(r#"{"op":"send","content":"hello"}"#);
  assert_eq!(response["ok"], false);
  assert!(response["error"].as_str().unwrap().contains("model name"));
  assert!(restored.state.conversations[0].messages.is_empty());
  assert!(!restored.state.is_streaming);
}

#[test]
fn generic_request_and_transfer_round_trip_use_selected_provider_contract() {
  let mut core = WodexCore::default();
  assert_eq!(
    apply(
      &mut core,
      json!({
        "provider":"openai_compatible", "api_key":"generic-key", "model":"custom/model",
        "base_url":"https://example.invalid/api/v2/", "token_limit_field":"max_completion_tokens"
      })
    )["ok"],
    true
  );
  let config = core.export_transfer(TransferChannel::Config).unwrap();
  let document: Value = serde_json::from_str(&config).unwrap();
  assert_eq!(document["schema_version"], 1);
  assert_eq!(document["settings"]["provider"], "openai_compatible");
  assert_eq!(document["settings"]["token_limit_field"], "max_completion_tokens");
  let prepared = WodexCore::default().prepare_transfer(TransferChannel::Config, &config).unwrap();
  let mut restored = prepared.core;
  let request = restored.call(r#"{"op":"send","content":"hello"}"#);
  assert_eq!(request["ok"], true);
  assert_eq!(request["request"]["url"], "https://example.invalid/api/v2/chat/completions");
  assert_eq!(request["request"]["api_key"], "generic-key");
  let body = &request["request"]["body"];
  assert_eq!(body["max_completion_tokens"], 4096);
  for field in ["thinking", "reasoning_effort", "stream_options", "max_tokens"] {
    assert!(body.get(field).is_none(), "unexpected {field}");
  }
  assert_eq!(restored.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")["ok"], true);
  assert!(restored.execute(Command::Finish { http_status: Some(200), error: None }).is_ok());
  assert_eq!(restored.state.conversations[0].messages[1].content, "hello");
  let sessions = restored.export_transfer(TransferChannel::Sessions).unwrap();
  assert!(!sessions.contains("generic-key"));
  assert!(!sessions.contains("openai_compatible"));
}

#[test]
fn invalid_provider_options_are_rejected_without_partial_mutation() {
  let mut core = WodexCore::default();
  apply(&mut core, json!({"api_key":"keep-key"}));
  let before = serde_json::to_value(&core.state.settings).unwrap();
  for patch in [
    json!({"provider":"unknown"}),
    json!({"provider":"open_ai_compatible"}),
    json!({"provider":"openai_compatible", "thinking":true}),
    json!({"token_limit_field":"max_completion_tokens"}),
    json!({"token_limit_field":"unknown"}),
    json!({"provider":"openai_compatible", "model":"invalid model"}),
  ] {
    assert_eq!(apply(&mut core, patch)["ok"], false);
    assert_eq!(serde_json::to_value(&core.state.settings).unwrap(), before);
  }
}
