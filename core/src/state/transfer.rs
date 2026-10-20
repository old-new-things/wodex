//! Versioned, independent PC-transfer documents. Preparing an import never
//! changes live state: the application persists the candidate before swapping it.
use super::*;

const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransferChannel {
  Config,
  Sessions,
}

impl TransferChannel {
  pub(crate) fn as_str(self) -> &'static str {
    match self {
      Self::Config => "config",
      Self::Sessions => "sessions",
    }
  }

  pub(crate) fn label(self) -> &'static str {
    match self {
      Self::Config => "Configuration",
      Self::Sessions => "Sessions",
    }
  }
}

pub(crate) struct PreparedTransfer {
  pub core: WodexCore,
  /// Counts and fixed explanatory text only; never uploaded content or secrets.
  pub summary: String,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "channel", rename_all = "snake_case", deny_unknown_fields)]
enum Document {
  Config { schema_version: u32, settings: Settings },
  Sessions { schema_version: u32, conversations: Vec<Conversation>, active_id: String },
}

impl WodexCore {
  pub(crate) fn export_transfer(&self, channel: TransferChannel) -> Result<String, String> {
    self.check_transfer_ready()?;
    let document = match channel {
      TransferChannel::Config => {
        Document::Config { schema_version: SCHEMA_VERSION, settings: self.state.settings.clone() }
      }
      TransferChannel::Sessions => {
        let mut conversations = self.state.conversations.clone();
        normalize_interrupted(&mut conversations);
        Document::Sessions {
          schema_version: SCHEMA_VERSION,
          conversations,
          active_id: self.state.active_id.clone(),
        }
      }
    };
    let json = serde_json::to_string(&document).map_err(|_| "Could not serialize the transfer.")?;
    if json.len() > MAX_SAVED_BYTES {
      return Err("The transfer exceeds 4 MiB.".into());
    }
    Ok(json)
  }

  pub(crate) fn prepare_transfer(
    &self,
    channel: TransferChannel,
    json: &str,
  ) -> Result<PreparedTransfer, String> {
    self.check_transfer_ready()?;
    if json.len() > MAX_SAVED_BYTES {
      return Err("The transfer exceeds 4 MiB.".into());
    }
    let document: Document = serde_json::from_str(json)
      .map_err(|_| String::from("Invalid transfer JSON or unsupported fields."))?;
    let (document_channel, schema_version) = match &document {
      Document::Config { schema_version, .. } => (TransferChannel::Config, *schema_version),
      Document::Sessions { schema_version, .. } => (TransferChannel::Sessions, *schema_version),
    };
    if document_channel != channel {
      return Err(format!("Choose a {} transfer file.", channel.as_str()));
    }
    if schema_version != SCHEMA_VERSION {
      return Err("Unsupported transfer schema version.".into());
    }
    let mut saved = self.storage_snapshot()?;
    let summary = match document {
      Document::Config { settings, .. } => {
        saved.settings = settings;
        String::from(
          "Replace configuration, including the saved API key. Sessions remain unchanged. Imported provider settings determine where future messages and your API key are sent.",
        )
      }
      Document::Sessions { conversations, active_id, .. } => {
        let highest_id = conversations.iter().try_fold(0, |highest, conversation| {
          conversation_id(&conversation.id).map(|id| highest.max(id))
        })?;
        // Do not reuse an ID already issued by this installation, even when a
        // replacement imports older sessions or has removed the latest chat.
        saved.next_id =
          self.next_id.max(highest_id.checked_add(1).ok_or("Conversation IDs are exhausted.")?);
        let messages: usize = conversations.iter().map(|chat| chat.messages.len()).sum();
        let conversation_label =
          if conversations.len() == 1 { "conversation" } else { "conversations" };
        let message_label = if messages == 1 { "message" } else { "messages" };
        let summary = format!(
          "Replace all sessions with {} {conversation_label} and {messages} {message_label}. Configuration remains unchanged.",
          conversations.len()
        );
        saved.conversations = conversations;
        saved.active_id = active_id;
        summary
      }
    };
    // The normal storage validator checks settings, every message, IDs, limits,
    // and the combined serialized save after either channel is replaced.
    Ok(PreparedTransfer { core: Self::from_storage(saved)?, summary })
  }

  fn check_transfer_ready(&self) -> Result<(), String> {
    if self.state.is_streaming || self.stream.is_some() {
      return Err("Wait for the current response or cancel it before transferring.".into());
    }
    if self.load_error.is_some() {
      return Err("Transfers are disabled to preserve unreadable saved data.".into());
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn export_value(core: &WodexCore, channel: TransferChannel) -> Value {
    serde_json::from_str(&core.export_transfer(channel).unwrap()).unwrap()
  }

  fn snapshot(core: &WodexCore) -> Value {
    serde_json::to_value(&core.state).unwrap()
  }

  fn sessions(core: &WodexCore) -> Value {
    export_value(core, TransferChannel::Sessions)
  }

  fn configuration(core: &WodexCore) -> Value {
    export_value(core, TransferChannel::Config)
  }

  fn seed() -> WodexCore {
    let mut core = WodexCore::default();
    core.state.settings.api_key = "sk-saved-test-key".into();
    core.state.settings.system_prompt = "Answer concisely in 中文.".into();
    core
      .execute(Command::Send { content: "A session message kept separate from config.".into() })
      .unwrap();
    core.execute(Command::Cancel {}).unwrap();
    core
  }

  #[test]
  fn export_documents_are_independent_and_include_the_saved_key() {
    let core = seed();
    let config = configuration(&core);
    assert_eq!(config["schema_version"], 1);
    assert_eq!(config["channel"], "config");
    assert_eq!(config["settings"]["api_key"], "sk-saved-test-key");
    assert_eq!(config["settings"]["system_prompt"], "Answer concisely in 中文.");
    assert_eq!(config.as_object().unwrap().len(), 3);
    assert!(config.get("conversations").is_none());
    let sessions = sessions(&core);
    assert_eq!(sessions["schema_version"], 1);
    assert_eq!(sessions["channel"], "sessions");
    assert_eq!(sessions["active_id"], "chat-1");
    assert_eq!(sessions.as_object().unwrap().len(), 4);
    assert!(sessions.get("settings").is_none());
    assert!(!sessions.to_string().contains("sk-saved-test-key"));
    assert!(!sessions.to_string().contains("Answer concisely"));
  }

  #[test]
  fn preparing_config_changes_only_candidate_settings_and_roundtrips() {
    let core = seed();
    let before = snapshot(&core);
    let mut incoming = configuration(&WodexCore::default());
    incoming["settings"]["api_key"] = json!("sk-imported");
    incoming["settings"]["base_url"] = json!("https://example.com/v1/");
    incoming["settings"]["update_channel"] = json!("nightly");
    let prepared = core.prepare_transfer(TransferChannel::Config, &incoming.to_string()).unwrap();
    assert_eq!(snapshot(&core), before);
    assert_eq!(sessions(&prepared.core), sessions(&core));
    assert_eq!(prepared.core.next_id, core.next_id);
    assert_eq!(prepared.core.state.settings.api_key, "sk-imported");
    assert_eq!(prepared.core.state.settings.base_url, "https://example.com/v1");
    assert_eq!(prepared.core.state.settings.update_channel, UpdateChannel::Nightly);
    assert!(prepared.core.stream.is_none());
    assert!(prepared.core.load_error.is_none());
    assert!(!prepared.summary.contains("sk-imported"));
    let transferred = prepared.core.export_transfer(TransferChannel::Config).unwrap();
    let roundtrip = core.prepare_transfer(TransferChannel::Config, &transferred).unwrap();
    assert_eq!(configuration(&roundtrip.core), configuration(&prepared.core));
  }

  #[test]
  fn preparing_sessions_replaces_only_sessions_and_normalizes_interruption() {
    let core = seed();
    let before = snapshot(&core);
    let mut incoming = sessions(&core);
    incoming["conversations"][0]["id"] = json!("chat-41");
    incoming["conversations"][0]["title"] = json!("Replacement conversation");
    incoming["conversations"][0]["messages"][1]["status"] = json!("streaming");
    incoming["conversations"][0]["messages"][1]["content"] = json!("A partial answer");
    incoming["active_id"] = json!("chat-41");
    let prepared = core.prepare_transfer(TransferChannel::Sessions, &incoming.to_string()).unwrap();
    assert_eq!(snapshot(&core), before);
    assert_eq!(configuration(&prepared.core), configuration(&core));
    assert_eq!(prepared.core.state.conversations.len(), 1);
    assert_eq!(prepared.core.state.active_id, "chat-41");
    assert_eq!(prepared.core.next_id, 42);
    assert_eq!(prepared.core.state.conversations[0].messages[1].status, MessageStatus::Interrupted);
    assert!(prepared.core.state.error.as_ref().unwrap().contains("interrupted"));
    assert!(!prepared.core.state.is_streaming);
    assert!(prepared.core.stream.is_none());
    assert!(!prepared.summary.contains("A partial answer"));
    assert!(prepared.summary.contains("1 conversation and 2 messages"));
    let mut candidate = prepared.core;
    candidate.execute(Command::NewChat {}).unwrap();
    assert_eq!(candidate.state.active_id, "chat-42");
  }

  #[test]
  fn session_import_does_not_reuse_previous_ids_and_rejects_overflow() {
    let mut core = seed();
    core.next_id = 500;
    let incoming = sessions(&WodexCore::default());
    let mut prepared =
      core.prepare_transfer(TransferChannel::Sessions, &incoming.to_string()).unwrap();
    prepared.core.execute(Command::NewChat {}).unwrap();
    assert_eq!(prepared.core.state.active_id, "chat-500");
    assert_eq!(prepared.core.next_id, 501);
    let before = snapshot(&core);
    for id in ["chat-0", "chat-01", "chat-+1", "chat-18446744073709551615", "anything"] {
      let mut invalid = incoming.clone();
      invalid["conversations"][0]["id"] = json!(id);
      invalid["active_id"] = json!(id);
      assert!(core.prepare_transfer(TransferChannel::Sessions, &invalid.to_string()).is_err());
      assert_eq!(snapshot(&core), before);
      assert_eq!(core.next_id, 500);
    }
  }

  #[test]
  fn strict_document_validation_rejects_wrong_channels_versions_and_fields() {
    let core = seed();
    let config = configuration(&core);
    assert!(core.prepare_transfer(TransferChannel::Sessions, &config.to_string()).is_err());
    let original = snapshot(&core);
    for (field, value) in [
      ("schema_version", json!(0)),
      ("schema_version", json!(2)),
      ("schema_version", json!("1")),
      ("extra", json!(true)),
      ("conversations", json!([])),
      ("channel", json!("configuration")),
    ] {
      let mut invalid = config.clone();
      invalid[field] = value;
      assert!(core.prepare_transfer(TransferChannel::Config, &invalid.to_string()).is_err());
      assert_eq!(snapshot(&core), original);
    }
    for json in ["not JSON", "{}", "[]", "{\"channel\":\"config\",\"channel\":\"config\"}"] {
      assert!(core.prepare_transfer(TransferChannel::Config, json).is_err());
    }
    let duplicate_channel = config.to_string().replacen(
      "\"channel\":\"config\"",
      "\"channel\":\"config\",\"channel\":\"config\"",
      1,
    );
    assert!(core.prepare_transfer(TransferChannel::Config, &duplicate_channel).is_err());
    let duplicate_schema = config.to_string().replacen(
      "\"schema_version\":1",
      "\"schema_version\":1,\"schema_version\":1",
      1,
    );
    assert!(core.prepare_transfer(TransferChannel::Config, &duplicate_schema).is_err());
    let oversized = " ".repeat(MAX_SAVED_BYTES + 1);
    assert!(core.prepare_transfer(TransferChannel::Config, &oversized).is_err());
  }

  #[test]
  fn imported_settings_reuse_all_storage_validation_rules() {
    let core = seed();
    for (field, value) in [
      ("api_key", json!("bad key")),
      ("api_key", json!("x".repeat(4097))),
      ("model", json!("invalid\nmodel")),
      ("base_url", json!("http://remote.example")),
      ("base_url", json!("https://safe.example@other.example")),
      ("max_tokens", json!(0)),
      ("max_tokens", json!(131_073)),
      ("system_prompt", json!("x".repeat(16 * 1024 + 1))),
      ("system_prompt", json!("\0")),
      ("update_repository", json!("../repo")),
      ("update_channel", json!("unknown")),
      ("unknown_field", json!(true)),
    ] {
      let mut invalid = configuration(&core);
      invalid["settings"][field] = value;
      assert!(core.prepare_transfer(TransferChannel::Config, &invalid.to_string()).is_err());
      assert_eq!(core.state.settings.api_key, "sk-saved-test-key");
    }
  }

  #[test]
  fn imported_sessions_reuse_structure_message_and_metadata_limits() {
    let core = seed();
    let original = sessions(&core);
    let mut hostile = Vec::new();
    let mut duplicate = original.clone();
    let chat = duplicate["conversations"][0].clone();
    duplicate["conversations"].as_array_mut().unwrap().push(chat);
    hostile.push(duplicate);
    for (field, value) in [
      ("conversations", json!([])),
      ("conversations", json!(vec![original["conversations"][0].clone(); MAX_CONVERSATIONS + 1])),
      ("active_id", json!("chat-2")),
      ("settings", json!(Settings::default())),
      ("next_id", json!(42)),
    ] {
      let mut invalid = original.clone();
      invalid[field] = value;
      hostile.push(invalid);
    }
    for (field, value) in [
      ("title", json!("")),
      ("title", json!("x".repeat(257))),
      ("title", json!("line\nbreak")),
      ("messages", json!(vec![original["conversations"][0]["messages"][0].clone(); 129])),
      ("extra", json!(true)),
    ] {
      let mut invalid = original.clone();
      invalid["conversations"][0][field] = value;
      hostile.push(invalid);
    }
    for (index, field, value) in [
      (0, "content", json!("")),
      (0, "content", json!("x".repeat(MAX_USER_BYTES + 1))),
      (0, "status", json!("streaming")),
      (0, "reasoning_content", json!("reasoning from a user")),
      (1, "content", json!("x".repeat(MAX_ANSWER_BYTES + 1))),
      (1, "content", json!("\0")),
      (1, "finish_reason", json!("x".repeat(65))),
      (1, "usage", json!({"total_tokens":-1})),
      (1, "usage", json!({"unexpected_tokens":0})),
      (1, "extra", json!(true)),
    ] {
      let mut invalid = original.clone();
      invalid["conversations"][0]["messages"][index][field] = value;
      hostile.push(invalid);
    }
    for invalid in hostile {
      assert!(core.prepare_transfer(TransferChannel::Sessions, &invalid.to_string()).is_err());
      assert_eq!(sessions(&core), original);
    }
  }

  #[test]
  fn combined_serialized_save_limit_is_checked_before_returning_candidate() {
    let mut core = seed();
    // An otherwise valid save can be near 4 MiB with JSON-escaped text while
    // remaining below the independent 2 MiB limit for decoded message text.
    let text = "\u{0001}".repeat(512 * 1024);
    let mut saved = core.storage_snapshot().unwrap();
    saved.conversations[0].messages[1].content = text;
    saved.conversations[0].messages.push(Message {
      role: Role::User,
      content: "Another question".into(),
      reasoning_content: String::new(),
      status: MessageStatus::Complete,
      finish_reason: None,
      usage: None,
    });
    saved.conversations[0].messages.push(Message {
      role: Role::Assistant,
      content: String::new(),
      reasoning_content: String::new(),
      status: MessageStatus::Complete,
      finish_reason: None,
      usage: None,
    });
    let overhead = serde_json::to_vec(&saved).unwrap().len();
    let remaining = MAX_SAVED_BYTES - overhead - 8 * 1024;
    saved.conversations[0].messages[3].content =
      "\u{0001}".repeat(remaining / 6) + &"x".repeat(remaining % 6);
    core = WodexCore::from_storage(saved).unwrap();
    let original = snapshot(&core);
    let mut config = configuration(&core);
    config["settings"]["system_prompt"] = json!("x".repeat(16 * 1024));
    let error = core.prepare_transfer(TransferChannel::Config, &config.to_string()).err().unwrap();
    assert!(error.contains("Saved data is too large"));
    assert_eq!(snapshot(&core), original);

    // A save at the byte boundary can grow when "streaming" becomes
    // "interrupted". Loading must preserve the original instead of accepting
    // a state that the next save would reject.
    let mut boundary = core.storage_snapshot().unwrap();
    boundary.conversations[0].messages[3].status = MessageStatus::Streaming;
    let remaining = MAX_SAVED_BYTES - serde_json::to_vec(&boundary).unwrap().len();
    boundary.conversations[0].messages[3].content.push_str(&"x".repeat(remaining));
    let json = serde_json::to_string(&boundary).unwrap();
    assert_eq!(json.len(), MAX_SAVED_BYTES);
    let mut restored = WodexCore::new(Some(&json));
    assert!(restored.load_error.as_ref().unwrap().contains("Saved data is too large"));
    assert!(restored.execute(Command::Export {}).is_err());
  }

  #[test]
  fn import_accepts_exact_count_and_message_limits_and_checks_total_text() {
    let core = seed();
    let mut incoming = sessions(&core);
    incoming["conversations"] = Value::Array(
      (1..=MAX_CONVERSATIONS)
        .map(|id| json!({"id":format!("chat-{id}"),"title":"x".repeat(256),"messages":[]}))
        .collect(),
    );
    incoming["active_id"] = json!("chat-32");
    incoming["conversations"][0]["messages"] = Value::Array(
      (0..MAX_MESSAGES)
        .map(|index| {
          let user = index % 2 == 0;
          json!({
            "role":if user { "user" } else { "assistant" },
            "content":if user { "question" } else { "" },
            "reasoning_content":"",
            "status":"complete"
          })
        })
        .collect(),
    );
    incoming["conversations"][0]["messages"][0]["content"] = json!("x".repeat(MAX_USER_BYTES));
    incoming["conversations"][0]["messages"][1]["content"] = json!("x".repeat(MAX_ANSWER_BYTES));
    let prepared = core.prepare_transfer(TransferChannel::Sessions, &incoming.to_string()).unwrap();
    assert_eq!(prepared.core.state.conversations.len(), MAX_CONVERSATIONS);
    assert_eq!(prepared.core.state.conversations[0].messages.len(), MAX_MESSAGES);
    assert_eq!(prepared.core.state.active_id, "chat-32");
    for index in [3, 5, 7] {
      incoming["conversations"][0]["messages"][index]["content"] =
        json!("x".repeat(MAX_ANSWER_BYTES));
    }
    assert!(incoming.to_string().len() < MAX_SAVED_BYTES);
    assert!(core.prepare_transfer(TransferChannel::Sessions, &incoming.to_string()).is_err());
  }

  #[test]
  fn live_streams_and_unreadable_storage_block_both_directions() {
    let valid = WodexCore::default();
    let mut streaming = seed();
    streaming.execute(Command::Send { content: "Active response".into() }).unwrap();
    let corrupt = WodexCore::new(Some("invalid storage"));
    for guarded in [&streaming, &corrupt] {
      let before = snapshot(guarded);
      for channel in [TransferChannel::Config, TransferChannel::Sessions] {
        let document = valid.export_transfer(channel).unwrap();
        assert!(guarded.export_transfer(channel).is_err());
        assert!(guarded.prepare_transfer(channel, &document).is_err());
      }
      assert_eq!(snapshot(guarded), before);
    }
  }
}
