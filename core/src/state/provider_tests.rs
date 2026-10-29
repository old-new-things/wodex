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
fn changing_provider_saves_and_restores_its_own_credentials_and_options() {
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
  assert_eq!(settings.system_prompt, Settings::default().system_prompt);
  assert_eq!(settings.max_tokens, 4096);
  assert_eq!(settings.provider_profiles[&Provider::DeepSeek].api_key, "previous-key");
  assert_eq!(serde_json::to_value(&core.state.conversations).unwrap(), conversations);
  apply(
    &mut core,
    json!({"api_key":"generic-key", "model":"custom-model", "base_url":"https://example.invalid/api/v2", "token_limit_field":"max_completion_tokens"}),
  );
  let configured = serde_json::to_value(&core.state.settings).unwrap();
  assert_eq!(apply(&mut core, json!({"provider":"openai_compatible"}))["ok"], true);
  assert_eq!(serde_json::to_value(&core.state.settings).unwrap(), configured);
  assert_eq!(apply(&mut core, json!({"provider":"deepseek"}))["ok"], true);
  assert_eq!(core.state.settings.api_key, "previous-key");
  assert_eq!(core.state.settings.system_prompt, "Keep instructions");
  assert_eq!(core.state.settings.max_tokens, 1234);
  assert!(core.state.settings.thinking);
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

#[test]
fn every_provider_preserves_its_own_profile_through_switching_reload_and_transfer() {
  let mut core = WodexCore::default();
  for (index, provider) in Provider::ALL.into_iter().enumerate() {
    let model = format!("model-{index}");
    let patch = json!({
      "provider": provider,
      "api_key": format!("saved-key-{index}"),
      "base_url": format!("https://provider-{index}.invalid/v1"),
      "model": model,
      "system_prompt": format!("Instructions {index} 中文"),
      "max_tokens": 1000 + index,
    });
    assert_eq!(apply(&mut core, patch)["ok"], true);
  }
  assert_eq!(core.state.settings.provider_profiles.len(), Provider::ALL.len() - 1);
  let saved = core.execute(Command::Export {}).unwrap()["data"].to_string();
  let reloaded = WodexCore::new(Some(&saved));
  let config = reloaded.export_transfer(TransferChannel::Config).unwrap();
  let mut restored =
    WodexCore::default().prepare_transfer(TransferChannel::Config, &config).unwrap().core;
  for (index, provider) in Provider::ALL.into_iter().enumerate() {
    assert_eq!(apply(&mut restored, json!({"provider":provider}))["ok"], true);
    let active = &restored.state.settings;
    assert_eq!(active.api_key, format!("saved-key-{index}"));
    assert_eq!(active.model, format!("model-{index}"));
    assert_eq!(active.system_prompt, format!("Instructions {index} 中文"));
    assert_eq!(active.max_tokens, 1000 + index as u32);
    assert!(!active.provider_profiles.contains_key(&provider));
  }
  let sessions = restored.export_transfer(TransferChannel::Sessions).unwrap();
  assert!(!sessions.contains("saved-key-"));
  assert!(!sessions.contains("provider_profiles"));
}

#[test]
fn imported_inactive_profiles_are_validated_before_any_configuration_changes() {
  let mut core = WodexCore::default();
  apply(&mut core, json!({"api_key":"keep-current-key"}));
  let before = core.export_transfer(TransferChannel::Config).unwrap();
  let profile = serde_json::to_value(ProviderProfile::for_provider(Provider::OpenAi)).unwrap();
  for (provider, field, value) in [
    ("deepseek", "api_key", json!("active-duplicate")),
    ("unknown", "api_key", json!("unknown-provider")),
    ("openai", "api_key", json!("invalid\nkey")),
    ("openai", "api_key", json!("invalid密钥")),
    ("openai", "thinking", json!(true)),
    ("openai", "allow_insecure_http", json!(true)),
    ("openai", "model", json!("invalid model")),
    ("openai", "max_tokens", json!(0)),
    ("openai", "surprise", json!(true)),
  ] {
    let mut document: Value = serde_json::from_str(&before).unwrap();
    let mut invalid = profile.clone();
    invalid[field] = value;
    document["settings"]["provider_profiles"] = json!({(provider): invalid});
    assert!(
      core.prepare_transfer(TransferChannel::Config, &document.to_string()).is_err(),
      "{provider}/{field}"
    );
    assert_eq!(core.export_transfer(TransferChannel::Config).unwrap(), before);
  }
}

#[test]
fn llama_private_http_is_explicit_and_optional_credentials_never_become_a_bearer_header() {
  let mut core = WodexCore::default();
  assert_eq!(apply(&mut core, json!({"provider":"llama_cpp","model":"local-model"}))["ok"], true);
  for endpoint in [
    "http://192.168.1.9:8080/v1",
    "http://10.2.3.4/v1",
    "http://172.20.1.9/v1",
    "http://desktop.LOCAL/v1",
  ] {
    assert_eq!(apply(&mut core, json!({"base_url":endpoint}))["ok"], false);
    assert_eq!(
      apply(&mut core, json!({"base_url":endpoint,"allow_insecure_http":true}))["ok"],
      true
    );
    let response = core.call(r#"{"op":"send","content":"hello"}"#);
    assert_eq!(response["ok"], true, "{response}");
    assert!(response["request"]["headers"]
      .as_array()
      .unwrap()
      .iter()
      .all(|header| { !header[0].as_str().unwrap().eq_ignore_ascii_case("authorization") }));
    core.execute(Command::Cancel {}).unwrap();
    apply(&mut core, json!({"base_url":"http://127.0.0.1:8080/v1","allow_insecure_http":false}));
  }
  for endpoint in [
    "http://example.com/v1",
    "http://8.8.8.8/v1",
    "http://172.32.1.1/v1",
    "http://192.168.01.2/v1",
    "http://0.0.0.0/v1",
  ] {
    assert_eq!(
      apply(&mut core, json!({"base_url":endpoint,"allow_insecure_http":true}))["ok"],
      false,
      "{endpoint}"
    );
  }
}

#[test]
fn many_large_provider_profiles_reserve_json_storage_before_accepting_stream_content() {
  let mut core = WodexCore::default();
  for provider in Provider::ALL {
    assert_eq!(
      apply(
        &mut core,
        json!({
          "provider":provider,"model":"model-test","api_key":"test-key",
          "system_prompt":"\u{1}".repeat(16*1024)
        })
      )["ok"],
      true
    );
  }
  let mut rejected = false;
  for _ in 0..2 {
    assert_eq!(core.call(r#"{"op":"send","content":"hello"}"#)["ok"], true);
    let event =
      format!("data: {}\n\n", json!({"choices":[{"delta":{"content":"\u{1}".repeat(16*1024)}}]}));
    for _ in 0..32 {
      if core.feed(event.as_bytes())["ok"] == false {
        rejected = true;
        break;
      }
    }
    core.execute(Command::Cancel {}).unwrap();
  }
  assert!(
    rejected,
    "Escaped provider settings must reserve space before reply text fills the save."
  );
  let storage = core.execute(Command::Export {}).unwrap()["data"].to_string();
  assert!(storage.len() <= MAX_SAVED_BYTES);
  assert!(WodexCore::new(Some(&storage)).load_error.is_none());
}

#[test]
fn native_messages_require_the_final_stop_event_before_core_marks_them_complete() {
  for terminal in [false, true] {
    let mut core = WodexCore::default();
    assert_eq!(
      apply(
        &mut core,
        json!({
          "provider":"opencode_go", "api_key":"fixture-key", "model":"minimax-m2.5"
        })
      )["ok"],
      true
    );
    let request = core.call(r#"{"op":"send","content":"hello"}"#);
    assert_eq!(request["request"]["protocol"], "messages");
    let mut wire = String::from(concat!(
      "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":5}}}\n\n",
      "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Retain this reply 中文\"}}\n\n",
      "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
    ));
    if terminal {
      wire.push_str("data: {\"type\":\"message_stop\"}\n\n");
    }
    assert_eq!(core.feed(wire.as_bytes())["ok"], true);
    core.execute(Command::Finish { error: None, http_status: Some(200) }).unwrap();
    let reply = core.state.conversations[0].messages.last().unwrap();
    assert_eq!(reply.content, "Retain this reply 中文");
    assert_eq!(reply.status, if terminal { MessageStatus::Complete } else { MessageStatus::Error });
    assert_eq!(core.state.error.is_some(), !terminal);
  }
}
