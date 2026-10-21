use crate::provider::{self, Provider, TokenLimitField};
use crate::stream::{Event, Stream};
use crate::update::UpdateChannel;
use alloc::{format, string::String, vec, vec::Vec};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(test)]
mod provider_tests;
pub(crate) mod transfer;

pub(crate) const MAX_SAVED_BYTES: usize = 4 * 1024 * 1024;
pub(crate) const MAX_COMMAND_BYTES: usize = 256 * 1024;
pub(crate) const MAX_FEED_BYTES: usize = 1024 * 1024;
const MAX_CONVERSATIONS: usize = 32;
const MAX_MESSAGES: usize = 128;
const MAX_USER_BYTES: usize = 64 * 1024;
const MAX_ANSWER_BYTES: usize = 512 * 1024;
const MAX_TOTAL_TEXT: usize = 2 * 1024 * 1024;
const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_CHARS: usize = 512;
pub(crate) const OFFICIAL_UPDATE_REPOSITORY: &str = "old-new-things/wodex";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
  #[serde(default, skip_serializing_if = "Provider::is_deepseek")]
  pub provider: Provider,
  #[serde(default, skip_serializing_if = "TokenLimitField::is_max_tokens")]
  pub token_limit_field: TokenLimitField,
  pub api_key: String,
  pub base_url: String,
  pub model: String,
  pub thinking: bool,
  pub max_tokens: u32,
  pub system_prompt: String,
  #[serde(default)]
  pub update_repository: String,
  #[serde(default)]
  pub update_channel: UpdateChannel,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      provider: Provider::DeepSeek,
      token_limit_field: TokenLimitField::MaxTokens,
      api_key: String::new(),
      base_url: "https://api.deepseek.com".into(),
      model: "deepseek-flash".into(),
      thinking: false,
      max_tokens: 4096,
      system_prompt: "You are a helpful assistant. Give clear, accurate answers.".into(),
      update_repository: OFFICIAL_UPDATE_REPOSITORY.into(),
      update_channel: UpdateChannel::Version,
    }
  }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SettingsPatch {
  pub provider: Option<Provider>,
  pub token_limit_field: Option<TokenLimitField>,
  pub api_key: Option<String>,
  pub base_url: Option<String>,
  pub model: Option<String>,
  pub thinking: Option<bool>,
  pub max_tokens: Option<u32>,
  pub system_prompt: Option<String>,
  pub update_repository: Option<String>,
  pub update_channel: Option<UpdateChannel>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
  User,
  Assistant,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
  Complete,
  Streaming,
  Interrupted,
  Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
  pub role: Role,
  pub content: String,
  pub reasoning_content: String,
  pub status: MessageStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub finish_reason: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Conversation {
  pub id: String,
  pub title: String,
  pub messages: Vec<Message>,
}

#[derive(Clone, Debug, Serialize)]
pub struct State {
  pub settings: Settings,
  pub conversations: Vec<Conversation>,
  pub active_id: String,
  pub is_streaming: bool,
  pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Storage {
  version: u32,
  settings: Settings,
  conversations: Vec<Conversation>,
  active_id: String,
  next_id: u64,
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
  Snapshot {},
  Export {},
  Settings { settings: SettingsPatch },
  NewChat {},
  SelectChat { id: String },
  DeleteChat { id: String },
  Send { content: String },
  Retry {},
  Cancel {},
  Finish { error: Option<String>, http_status: Option<u16> },
  ResetStorage {},
}

/// Chat state and provider requests, with no filesystem or network effects.
pub struct WodexCore {
  state: State,
  next_id: u64,
  stream: Option<Stream>,
  load_error: Option<String>,
  encoded_text_bytes: usize,
}

impl Default for WodexCore {
  fn default() -> Self {
    Self {
      state: State {
        settings: Settings::default(),
        conversations: vec![Conversation {
          id: "chat-1".into(),
          title: "New conversation".into(),
          messages: Vec::new(),
        }],
        active_id: "chat-1".into(),
        is_streaming: false,
        error: None,
      },
      next_id: 2,
      stream: None,
      load_error: None,
      encoded_text_bytes: 0,
    }
  }
}

impl WodexCore {
  /// Borrow validated state directly; native clients keep using the stable C API.
  pub(crate) fn state(&self) -> &State {
    &self.state
  }

  pub fn new(saved_json: Option<&str>) -> Self {
    let mut core = Self::default();
    if let Some(saved_json) = saved_json {
      if let Err(error) = core.load(saved_json) {
        let error = format!("Saved data could not be loaded: {error} Original data was preserved.");
        core.state.error = Some(error.clone());
        core.load_error = Some(error);
      }
    }
    core
  }

  pub fn call(&mut self, command_json: &str) -> Value {
    let result = if command_json.len() > MAX_COMMAND_BYTES {
      Err(String::from("The command is too large."))
    } else {
      serde_json::from_str::<Command>(command_json)
        .map_err(|_| String::from("Invalid command JSON or unsupported command fields."))
        .and_then(|command| self.execute(command))
    };
    match result {
      Ok(value) => value,
      Err(error) => self.failure(&error),
    }
  }

  pub fn feed(&mut self, bytes: &[u8]) -> Value {
    if !self.state.is_streaming {
      return self.failure("No response is streaming.");
    }
    let stream = self.stream.as_mut().expect("streaming state has a parser");
    if bytes.len() > MAX_FEED_BYTES || stream.total_bytes + bytes.len() > MAX_STREAM_BYTES {
      return self.fail_stream("The response exceeded the download limit.");
    }
    stream.total_bytes += bytes.len();
    let events = stream.feed(bytes);
    if let Err(error) = self.apply_events(events) {
      return self.fail_stream(&error);
    }
    json!({"ok": true, "done": self.stream.as_ref().is_some_and(|stream| stream.done)})
  }

  pub(crate) fn execute(&mut self, command: Command) -> Result<Value, String> {
    match command {
      Command::Snapshot {} => Ok(json!({"ok": true, "state": self.state})),
      Command::Export {} => {
        let storage = self.storage_snapshot()?;
        let data = serde_json::to_value(storage).map_err(|_| "Could not serialize saved data.")?;
        if serde_json::to_vec(&data).map_err(|_| "Could not serialize saved data.")?.len()
          > MAX_SAVED_BYTES
        {
          return Err("Saved data is too large. Delete an older conversation.".into());
        }
        Ok(json!({"ok": true, "data": data}))
      }
      Command::Cancel {} => {
        if self.state.is_streaming {
          self.end_stream(MessageStatus::Interrupted);
          self.state.error = None;
        }
        Ok(json!({"ok": true}))
      }
      Command::Finish { error, http_status } => self.finish(error, http_status),
      command => {
        if self.state.is_streaming {
          return Err("Wait for the current response or cancel it first.".into());
        }
        self.execute_idle(command)
      }
    }
  }

  fn execute_idle(&mut self, command: Command) -> Result<Value, String> {
    match command {
      Command::Settings { settings: patch } => {
        let mut settings = self.state.settings.clone();
        if let Some(provider) = patch.provider.filter(|provider| *provider != settings.provider) {
          // Switching protocols must not silently send the previous provider's
          // credential to a new service. A complete imported/edited document
          // may explicitly supply its own credential and endpoint afterwards.
          settings.provider = provider;
          settings.api_key.clear();
          settings.base_url = provider.default_base_url().into();
          settings.model = provider.default_model().into();
          settings.thinking = false;
          settings.token_limit_field = TokenLimitField::MaxTokens;
        }
        macro_rules! apply {
          ($($field:ident),+ $(,)?) => {$({
            if let Some(value) = patch.$field {
              settings.$field = value;
            }
          })+};
        }
        apply!(
          token_limit_field,
          api_key,
          base_url,
          model,
          thinking,
          max_tokens,
          system_prompt,
          update_repository,
          update_channel
        );
        settings.base_url = settings.base_url.trim_end_matches('/').into();
        validate_settings(&settings)?;
        self.state.settings = settings;
      }
      Command::NewChat {} => {
        if self.state.conversations.len() >= MAX_CONVERSATIONS {
          return Err("The conversation limit is 32. Delete an older conversation first.".into());
        }
        if !self.storage_fits(self.encoded_text_bytes, self.message_count(), 1) {
          return Err("Conversation storage is full. Delete an older conversation first.".into());
        }
        self.new_chat()?;
      }
      Command::SelectChat { id } => {
        if !self.state.conversations.iter().any(|conversation| conversation.id == id) {
          return Err("Conversation not found.".into());
        }
        self.state.active_id = id;
      }
      Command::DeleteChat { id } => {
        let index = self
          .state
          .conversations
          .iter()
          .position(|conversation| conversation.id == id)
          .ok_or("Conversation not found.")?;
        // Deleting the final chat creates its replacement. Verify that this
        // can succeed before removing the sole active conversation.
        if self.state.conversations.len() == 1 {
          self.next_id.checked_add(1).ok_or("Conversation IDs are exhausted.")?;
        }
        let removed = self.state.conversations.remove(index);
        for message in removed.messages {
          self.encoded_text_bytes -=
            json_text_bytes(&message.content) + json_text_bytes(&message.reasoning_content);
        }
        if self.state.conversations.is_empty() {
          self.new_chat()?;
        } else if self.state.active_id == id {
          self.state.active_id = self.state.conversations[0].id.clone();
        }
      }
      Command::Send { content } => return self.send(content),
      Command::Retry {} => return self.retry(),
      Command::ResetStorage {} => {
        *self = Self::default();
      }
      _ => return Err("Unsupported operation in this state.".into()),
    }
    self.state.error = None;
    Ok(json!({"ok": true}))
  }

  fn new_chat(&mut self) -> Result<(), String> {
    let next_id = self.next_id.checked_add(1).ok_or("Conversation IDs are exhausted.")?;
    let id = format!("chat-{}", self.next_id);
    self.next_id = next_id;
    self.state.active_id = id.clone();
    self
      .state
      .conversations
      .insert(0, Conversation { id, title: "New conversation".into(), messages: Vec::new() });
    Ok(())
  }

  fn active_index(&self) -> usize {
    self
      .state
      .conversations
      .iter()
      .position(|conversation| conversation.id == self.state.active_id)
      .expect("validated state always has an active conversation")
  }

  fn send(&mut self, content: String) -> Result<Value, String> {
    self.check_ready()?;
    let content = content.trim();
    if content.is_empty() {
      return Err("Enter a message first.".into());
    }
    if content.len() > MAX_USER_BYTES || content.contains('\0') {
      return Err("A message must be at most 64 KiB and cannot contain NUL characters.".into());
    }
    if self.total_text() + content.len() > MAX_TOTAL_TEXT {
      return Err("Conversation storage is full. Delete an older conversation first.".into());
    }
    let encoded_text_bytes = self.encoded_text_bytes + json_text_bytes(content);
    if !self.storage_fits(encoded_text_bytes, self.message_count() + 2, 0) {
      return Err("Conversation storage is full. Delete an older conversation first.".into());
    }
    let index = self.active_index();
    let conversation = &mut self.state.conversations[index];
    if conversation.messages.len() + 2 > MAX_MESSAGES {
      return Err("This conversation is full. Start a new conversation.".into());
    }
    if conversation.messages.is_empty() {
      conversation.title = make_title(content);
    }
    conversation.messages.push(Message {
      role: Role::User,
      content: content.into(),
      reasoning_content: String::new(),
      status: MessageStatus::Complete,
      finish_reason: None,
      usage: None,
    });
    self.encoded_text_bytes = encoded_text_bytes;
    Ok(self.start_request())
  }

  fn retry(&mut self) -> Result<Value, String> {
    self.check_ready()?;
    let index = self.active_index();
    let last =
      self.state.conversations[index].messages.last().ok_or("There is no message to retry.")?;
    let removed_bytes = if last.role == Role::Assistant {
      json_text_bytes(&last.content) + json_text_bytes(&last.reasoning_content)
    } else {
      0
    };
    let added_messages = usize::from(last.role == Role::User);
    if !self.storage_fits(
      self.encoded_text_bytes - removed_bytes,
      self.message_count() + added_messages,
      0,
    ) {
      return Err("Conversation storage is full. Delete an older conversation first.".into());
    }
    let conversation = &mut self.state.conversations[index];
    if conversation.messages.last().is_some_and(|message| message.role == Role::Assistant) {
      conversation.messages.pop();
    }
    if !conversation.messages.last().is_some_and(|message| message.role == Role::User) {
      return Err("There is no user message to retry.".into());
    }
    if conversation.messages.len() >= MAX_MESSAGES {
      return Err("This conversation is full. Start a new conversation.".into());
    }
    self.encoded_text_bytes -= removed_bytes;
    Ok(self.start_request())
  }

  fn check_ready(&self) -> Result<(), String> {
    validate_settings(&self.state.settings)?;
    if self.state.settings.api_key.trim().is_empty() {
      return Err(format!(
        "Add your {} API key in Settings first.",
        self.state.settings.provider.label()
      ));
    }
    if self.state.settings.model.is_empty() {
      return Err("Enter your provider's model name in Settings first.".into());
    }
    Ok(())
  }

  fn start_request(&mut self) -> Value {
    let index = self.active_index();
    let body =
      provider::chat_request(&self.state.settings, &self.state.conversations[index].messages);
    let request = json!({
      "url": format!("{}/chat/completions", self.state.settings.base_url),
      "api_key": self.state.settings.api_key,
      "body": body
    });
    self.state.conversations[index].messages.push(Message {
      role: Role::Assistant,
      content: String::new(),
      reasoning_content: String::new(),
      status: MessageStatus::Streaming,
      finish_reason: None,
      usage: None,
    });
    self.state.is_streaming = true;
    self.state.error = None;
    self.stream = Some(Stream::default());
    json!({"ok": true, "request": request})
  }

  fn apply_events(&mut self, events: Vec<Event>) -> Result<(), String> {
    let mut total_text = self.total_text();
    let metadata_bytes = self.storage_metadata_bytes(self.message_count(), 0);
    let index = self.active_index();
    let message = self.state.conversations[index]
      .messages
      .last_mut()
      .ok_or("The streaming message is missing.")?;
    for event in events {
      match event {
        Event::Delta { content, reasoning } => {
          if content.contains('\0') || reasoning.contains('\0') {
            return Err("The server sent a NUL character in its response.".into());
          }
          let added = content.len() + reasoning.len();
          if message.content.len() + message.reasoning_content.len() + added > MAX_ANSWER_BYTES {
            return Err("The response exceeded the 512 KiB display limit.".into());
          }
          if total_text + added > MAX_TOTAL_TEXT {
            return Err("Conversation storage is full. Delete an older conversation first.".into());
          }
          let encoded_added = json_text_bytes(&content) + json_text_bytes(&reasoning);
          if self.encoded_text_bytes + encoded_added + metadata_bytes > MAX_SAVED_BYTES {
            return Err("Conversation storage is full. Delete an older conversation first.".into());
          }
          message.content.push_str(&content);
          message.reasoning_content.push_str(&reasoning);
          total_text += added;
          self.encoded_text_bytes += encoded_added;
        }
        Event::Error(error) => return Err(error),
      }
    }
    Ok(())
  }

  fn finish(&mut self, error: Option<String>, http_status: Option<u16>) -> Result<Value, String> {
    if !self.state.is_streaming {
      // curl can report completion after a parser error or cancellation. Keep
      // the original cause and the terminal message status in that case.
      return Ok(json!({"ok": true}));
    }
    if let Some(error) = error.filter(|error| !error.trim().is_empty()) {
      return Ok(self.fail_stream(&error));
    }
    if let Some(status) = http_status.filter(|status| !(200..300).contains(status)) {
      return Ok(self.fail_stream(&format!(
        "{} returned HTTP {status}. Check your key, balance, and endpoint.",
        self.state.settings.provider.label()
      )));
    }
    let events = self.stream.as_mut().expect("active stream").finish();
    if let Err(error) = self.apply_events(events) {
      return Ok(self.fail_stream(&error));
    }
    let stream = self.stream.as_ref().expect("active stream");
    if !stream.done && stream.finish_reason.is_none() {
      return Ok(
        self.fail_stream("The connection ended before the response finished. You can retry."),
      );
    }
    let finish_reason = stream.finish_reason.clone();
    if finish_reason.as_deref() == Some("insufficient_system_resource") {
      return Ok(self.fail_stream(
        "The API could not finish because its resources were exhausted. You can retry.",
      ));
    }
    let warning = match finish_reason.as_deref() {
      Some("length") => {
        Some("The response reached the token limit. Increase max_tokens in Settings.")
      }
      Some("content_filter") => {
        Some("The API stopped this response because of its content filter.")
      }
      _ => None,
    };
    self.end_stream(MessageStatus::Complete);
    self.state.error = warning.map(String::from);
    Ok(json!({"ok": true}))
  }

  fn end_stream(&mut self, status: MessageStatus) {
    let index = self.active_index();
    if let Some(message) = self.state.conversations[index].messages.last_mut() {
      message.status = status;
      if let Some(stream) = &self.stream {
        message.finish_reason = stream.finish_reason.clone();
        message.usage = stream.usage.clone();
      }
    }
    self.stream = None;
    self.state.is_streaming = false;
  }

  fn fail_stream(&mut self, error: &str) -> Value {
    self.end_stream(MessageStatus::Error);
    self.failure(error)
  }

  fn failure(&mut self, error: &str) -> Value {
    let error = self.clean_error(error);
    self.state.error = Some(error.clone());
    json!({"ok": false, "error": error})
  }

  fn clean_error(&self, error: &str) -> String {
    let key = &self.state.settings.api_key;
    let text = if key.is_empty() { String::from(error) } else { error.replace(key, "[redacted]") };
    text
      .chars()
      .filter(|character| !character.is_control() || *character == '\n')
      .take(MAX_ERROR_CHARS)
      .collect()
  }

  fn total_text(&self) -> usize {
    self
      .state
      .conversations
      .iter()
      .flat_map(|conversation| &conversation.messages)
      .map(|message| message.content.len() + message.reasoning_content.len())
      .sum()
  }

  fn message_count(&self) -> usize {
    self.state.conversations.iter().map(|conversation| conversation.messages.len()).sum()
  }

  // Reserve worst-case bounded metadata separately from message text. JSON can
  // expand control bytes sixfold, so a plain UTF-8 byte limit alone cannot
  // guarantee that every accepted response can still be persisted.
  fn storage_metadata_bytes(&self, messages: usize, added_chats: usize) -> usize {
    128 * 1024 + messages * 512 + (self.state.conversations.len() + added_chats) * 1024
  }

  fn storage_fits(&self, encoded_text: usize, messages: usize, added_chats: usize) -> bool {
    encoded_text + self.storage_metadata_bytes(messages, added_chats) <= MAX_SAVED_BYTES
  }

  fn load(&mut self, saved_json: &str) -> Result<(), String> {
    if saved_json.len() > MAX_SAVED_BYTES {
      return Err("The save exceeds 4 MiB.".into());
    }
    let saved: Storage = serde_json::from_str(saved_json)
      .map_err(|_| String::from("Invalid storage JSON or fields."))?;
    *self = Self::from_storage(saved)?;
    Ok(())
  }

  fn storage_snapshot(&self) -> Result<Storage, String> {
    if let Some(error) = &self.load_error {
      return Err(format!("Export is disabled to preserve unreadable saved data. {error}"));
    }
    let mut conversations = self.state.conversations.clone();
    normalize_interrupted(&mut conversations);
    Ok(Storage {
      version: 1,
      settings: self.state.settings.clone(),
      conversations,
      active_id: self.state.active_id.clone(),
      next_id: self.next_id,
    })
  }

  fn from_storage(mut saved: Storage) -> Result<Self, String> {
    if saved.version != 1 {
      return Err("Unsupported storage version.".into());
    }
    saved.settings.base_url = saved.settings.base_url.trim_end_matches('/').into();
    validate_settings(&saved.settings)?;
    if saved.conversations.is_empty() || saved.conversations.len() > MAX_CONVERSATIONS {
      return Err("Invalid conversation count.".into());
    }
    let mut highest_id = 0;
    let mut total_text = 0;
    let mut encoded_text_bytes = 0;
    for (index, conversation) in saved.conversations.iter().enumerate() {
      let id = conversation_id(&conversation.id)?;
      if saved.conversations[..index].iter().any(|previous| previous.id == conversation.id) {
        return Err("Duplicate or invalid conversation ID.".into());
      }
      highest_id = highest_id.max(id);
      if conversation.title.is_empty()
        || conversation.title.len() > 256
        || conversation.title.chars().any(char::is_control)
        || conversation.messages.len() > MAX_MESSAGES
      {
        return Err("Invalid conversation title or message count.".into());
      }
      for (index, message) in conversation.messages.iter().enumerate() {
        if message.content.contains('\0') || message.reasoning_content.contains('\0') {
          return Err("Invalid NUL in saved message.".into());
        }
        match message.role {
          Role::User
            if message.status != MessageStatus::Complete
              || !message.reasoning_content.is_empty()
              || message.content.trim().is_empty()
              || message.content.len() > MAX_USER_BYTES =>
          {
            return Err("Invalid user message.".into());
          }
          Role::Assistant
            if index == 0
              || conversation.messages[index - 1].role != Role::User
              || message.content.len() + message.reasoning_content.len() > MAX_ANSWER_BYTES =>
          {
            return Err("Invalid assistant message.".into());
          }
          _ => {}
        }
        if message.finish_reason.as_ref().is_some_and(|reason| {
          reason.is_empty() || reason.len() > 64 || reason.chars().any(char::is_control)
        }) {
          return Err("Invalid saved finish reason.".into());
        }
        if let Some(usage) = &message.usage {
          let object = usage.as_object().ok_or("Invalid saved usage.")?;
          if object.len() > 3
            || object.iter().any(|(key, value)| {
              !matches!(key.as_str(), "prompt_tokens" | "completion_tokens" | "total_tokens")
                || value.as_u64().is_none()
            })
          {
            return Err("Invalid saved usage.".into());
          }
        }
        total_text += message.content.len() + message.reasoning_content.len();
        encoded_text_bytes +=
          json_text_bytes(&message.content) + json_text_bytes(&message.reasoning_content);
      }
    }
    if total_text > MAX_TOTAL_TEXT
      || saved.next_id <= highest_id
      || !saved.conversations.iter().any(|conversation| conversation.id == saved.active_id)
    {
      return Err("Invalid storage size, active conversation, or next ID.".into());
    }
    let interrupted = normalize_interrupted(&mut saved.conversations);
    if serde_json::to_vec(&saved).map_err(|_| "Could not serialize saved data.")?.len()
      > MAX_SAVED_BYTES
    {
      return Err("Saved data is too large. Delete an older conversation.".into());
    }
    Ok(Self {
      state: State {
        settings: saved.settings,
        conversations: saved.conversations,
        active_id: saved.active_id,
        is_streaming: false,
        error: interrupted
          .then(|| String::from("The previous response was interrupted. You can retry.")),
      },
      next_id: saved.next_id,
      stream: None,
      load_error: None,
      encoded_text_bytes,
    })
  }
}

fn conversation_id(value: &str) -> Result<u64, String> {
  let id = value
    .strip_prefix("chat-")
    .and_then(|id| id.parse::<u64>().ok())
    .filter(|&id| id > 0)
    .ok_or("Invalid conversation ID.")?;
  if value != format!("chat-{id}") {
    return Err("Invalid conversation ID.".into());
  }
  Ok(id)
}

// A saved or transferred response cannot retain a live stream after restoration.
fn normalize_interrupted(conversations: &mut [Conversation]) -> bool {
  let mut interrupted = false;
  for conversation in conversations {
    for message in &mut conversation.messages {
      if message.status == MessageStatus::Streaming {
        message.status = MessageStatus::Interrupted;
        interrupted = true;
      }
    }
  }
  interrupted
}

fn json_text_bytes(text: &str) -> usize {
  text
    .bytes()
    .map(|byte| match byte {
      b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0c => 2,
      0..=0x1f => 6,
      _ => 1,
    })
    .sum()
}

fn make_title(content: &str) -> String {
  let title: String =
    content.chars().filter(|character| !character.is_control()).take(48).collect();
  if content.chars().count() > 48 {
    format!("{title}…")
  } else {
    title
  }
}

fn validate_settings(settings: &Settings) -> Result<(), String> {
  if settings.provider == Provider::OpenAiCompatible && settings.thinking {
    return Err("Thinking controls are available only for DeepSeek.".into());
  }
  if settings.provider == Provider::DeepSeek
    && settings.token_limit_field != TokenLimitField::MaxTokens
  {
    return Err("DeepSeek uses the max_tokens response limit.".into());
  }
  if !settings.update_repository.is_empty()
    && !crate::update::valid_repository(&settings.update_repository)
  {
    return Err(
      "Enter a public GitHub repository as owner/repo, or leave the update source empty.".into(),
    );
  }
  if settings.api_key.len() > 4096
    || settings.api_key.chars().any(|c| c.is_whitespace() || c.is_control())
  {
    return Err(
      "The API key must contain no spaces or control characters and be at most 4096 bytes.".into(),
    );
  }
  if (settings.model.is_empty() && settings.provider == Provider::DeepSeek)
    || settings.model.len() > 128
    || !settings.model.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/".contains(&byte))
  {
    return Err("Enter a valid model name of at most 128 ASCII characters.".into());
  }
  if !(1..=131_072).contains(&settings.max_tokens) {
    return Err("max_tokens must be between 1 and 131072.".into());
  }
  if settings.system_prompt.len() > 16 * 1024 || settings.system_prompt.contains('\0') {
    return Err(
      "The system prompt must be at most 16 KiB and cannot contain NUL characters.".into(),
    );
  }
  validate_base_url(&settings.base_url)
}

fn validate_base_url(url: &str) -> Result<(), String> {
  let fail = || {
    String::from("Use an HTTPS base URL without credentials, query, or fragment. HTTP is allowed only on localhost.")
  };
  if url.len() > 2048
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
  if !secure && !matches!(host, "localhost" | "127.0.0.1" | "[::1]") {
    return Err(fail());
  }
  if remainder.contains("//") || remainder.split('/').skip(1).any(|part| matches!(part, "." | ".."))
  {
    return Err(fail());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn command(core: &mut WodexCore, value: Value) -> Value {
    core.call(&value.to_string())
  }

  fn ready() -> WodexCore {
    let mut core = WodexCore::default();
    assert_eq!(
      command(&mut core, json!({"op":"settings","settings":{"api_key":"sk-test"}}))["ok"],
      true
    );
    core
  }

  fn delta(core: &mut WodexCore, content: &str, reasoning: &str) -> Value {
    core.feed(
      format!(
        "data: {}\n\n",
        json!({"choices":[{"index":0,"delta":{
          "content":content,"reasoning_content":reasoning
        }}]})
      )
      .as_bytes(),
    )
  }

  fn finish_successfully(core: &mut WodexCore) {
    assert_eq!(core.feed(b"data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"total_tokens\":8}}\n\ndata: [DONE]\n\n")["ok"], true);
    assert_eq!(command(core, json!({"op":"finish","http_status":200}))["ok"], true);
  }

  #[test]
  fn requests_have_correct_payload_and_context() {
    let mut core = ready();
    command(
      &mut core,
      json!({"op":"settings","settings":{
        "base_url":"https://api.deepseek.com/v1/","thinking":true,"max_tokens":8192,"system_prompt":"Be concise"
      }}),
    );
    let response = command(&mut core, json!({"op":"send","content":"  Hello  "}));
    assert_eq!(response["request"]["url"], "https://api.deepseek.com/v1/chat/completions");
    assert_eq!(response["request"]["api_key"], "sk-test");
    let body = &response["request"]["body"];
    assert_eq!(body["model"], "deepseek-flash");
    assert_eq!(body["stream"], true);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["max_tokens"], 8192);
    assert_eq!(
      body["messages"],
      json!([
        {"role":"system","content":"Be concise"},{"role":"user","content":"Hello"}
      ])
    );
    delta(&mut core, "Hi!", "A private chain of thought");
    finish_successfully(&mut core);
    let response = command(&mut core, json!({"op":"send","content":"Next"}));
    assert_eq!(
      response["request"]["body"]["messages"],
      json!([
        {"role":"system","content":"Be concise"},
        {"role":"user","content":"Hello"},
        {"role":"assistant","content":"Hi!"},
        {"role":"user","content":"Next"}
      ])
    );
    assert!(!response["request"]["body"].to_string().contains("private chain"));
  }

  #[test]
  fn cancellation_preserves_partial_and_retry_does_not_duplicate_user() {
    let mut core = ready();
    command(&mut core, json!({"op":"send","content":"Explain it"}));
    delta(&mut core, "Partial", "Thinking");
    let before = core.state.conversations.clone();
    for blocked in [
      json!({"op":"new_chat"}),
      json!({"op":"select_chat","id":"chat-1"}),
      json!({"op":"delete_chat","id":"chat-1"}),
      json!({"op":"send","content":"No"}),
      json!({"op":"retry"}),
      json!({"op":"settings","settings":{"thinking":true}}),
      json!({"op":"reset_storage"}),
    ] {
      assert_eq!(command(&mut core, blocked)["ok"], false);
    }
    assert_eq!(
      serde_json::to_value(&core.state.conversations).unwrap(),
      serde_json::to_value(before).unwrap()
    );
    assert_eq!(command(&mut core, json!({"op":"cancel"}))["ok"], true);
    assert!(!core.state.is_streaming);
    assert_eq!(core.state.conversations[0].messages[1].content, "Partial");
    assert_eq!(core.state.conversations[0].messages[1].status, MessageStatus::Interrupted);
    let retry = command(&mut core, json!({"op":"retry"}));
    assert_eq!(retry["ok"], true);
    assert_eq!(retry["request"]["body"]["messages"].as_array().unwrap().len(), 2);
    assert_eq!(core.state.conversations[0].messages.len(), 2);
    delta(&mut core, "Finished", "");
    finish_successfully(&mut core);
    let message = &core.state.conversations[0].messages[1];
    assert_eq!(message.content, "Finished");
    assert_eq!(message.status, MessageStatus::Complete);
    assert_eq!(message.usage, Some(json!({"total_tokens":8})));
    assert_eq!(message.finish_reason.as_deref(), Some("stop"));
  }

  #[test]
  fn later_turn_omits_cancelled_assistant() {
    let mut core = ready();
    command(&mut core, json!({"op":"send","content":"First"}));
    delta(&mut core, "Unfinished", "Do not repeat this");
    command(&mut core, json!({"op":"cancel"}));
    let response = command(&mut core, json!({"op":"send","content":"Second"}));
    let messages = response["request"]["body"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["content"], "First");
    assert_eq!(messages[2]["content"], "Second");
    assert!(!response.to_string().contains("Unfinished"));
  }

  #[test]
  fn truncation_and_api_errors_are_terminal_with_partial_content_preserved() {
    let mut core = ready();
    command(&mut core, json!({"op":"send","content":"Hello"}));
    delta(&mut core, "Partial", "");
    let response = command(&mut core, json!({"op":"finish","http_status":200}));
    assert_eq!(response["ok"], false);
    assert!(response["error"].as_str().unwrap().contains("before the response finished"));
    assert_eq!(core.state.conversations[0].messages[1].status, MessageStatus::Error);
    assert_eq!(core.state.conversations[0].messages[1].content, "Partial");
    assert!(!core.state.is_streaming);

    command(&mut core, json!({"op":"retry"}));
    let response = core.feed(b"data: {\"choices\":[{\"delta\":{\"content\":\"Before error\"}}]}\n\ndata: {\"error\":{\"message\":\"Invalid sk-test key\\u0001\"}}\n\n");
    assert_eq!(response["ok"], false);
    assert_eq!(response["error"], "Invalid [redacted] key");
    assert_eq!(core.state.conversations[0].messages[1].content, "Before error");
    assert!(!core.state.is_streaming);
    command(&mut core, json!({"op":"finish","error":"Canceled by callback"}));
    assert_eq!(core.state.error.as_deref(), Some("Invalid [redacted] key"));
  }

  #[test]
  fn final_reason_without_done_is_accepted_and_length_is_visible() {
    let mut core = ready();
    command(&mut core, json!({"op":"send","content":"Hello"}));
    core.feed(
      b"data: {\"choices\":[{\"delta\":{\"content\":\"Tail\"},\"finish_reason\":\"length\"}]}",
    );
    assert_eq!(command(&mut core, json!({"op":"finish","http_status":200}))["ok"], true);
    assert_eq!(core.state.conversations[0].messages[1].status, MessageStatus::Complete);
    assert_eq!(core.state.conversations[0].messages[1].content, "Tail");
    assert!(core.state.error.as_ref().unwrap().contains("token limit"));
  }

  #[test]
  fn settings_are_transactional_and_reject_hostile_values() {
    let mut core = ready();
    let before = serde_json::to_value(&core.state.settings).unwrap();
    for settings in [
      json!({"base_url":"http://evil.example"}),
      json!({"base_url":"https://good.example@evil.example"}),
      json!({"base_url":"https://good.example?key=secret"}),
      json!({"base_url":"https://good.example/#part"}),
      json!({"base_url":"https://good.example\\evil"}),
      json!({"base_url":"https://good.example:65536"}),
      json!({"base_url":"https://good.example/../private"}),
      json!({"api_key":"sk-test\r\nInjected: value"}),
      json!({"api_key":"bad key"}),
      json!({"model":"model\n"}),
      json!({"max_tokens":0}),
      json!({"max_tokens":131073}),
      json!({"system_prompt":"\0"}),
      json!({"unrecognized":true}),
    ] {
      assert_eq!(command(&mut core, json!({"op":"settings","settings":settings}))["ok"], false);
      assert_eq!(serde_json::to_value(&core.state.settings).unwrap(), before);
    }
    for base_url in [
      "http://127.0.0.1:8080/v1",
      "http://localhost:8080",
      "http://[::1]:8080",
      "https://example.com/proxy/v1",
    ] {
      assert_eq!(
        command(&mut core, json!({"op":"settings","settings":{"base_url":base_url}}))["ok"],
        true
      );
    }
    assert_eq!(core.call("{\"op\":\"snapshot\",\"extra\":true}")["ok"], false);
    assert_eq!(core.call("[]")["ok"], false);
    assert_eq!(command(&mut core, json!({"op":"send","content":" \n\t "}))["ok"], false);
    assert_eq!(command(&mut core, json!({"op":"send","content":"bad\0text"}))["ok"], false);
  }

  #[test]
  fn persistence_roundtrip_recovers_interruption_and_protects_corruption() {
    let mut core = ready();
    command(&mut core, json!({"op":"new_chat"}));
    command(&mut core, json!({"op":"send","content":"你好 👋"}));
    delta(&mut core, "A partial answer", "Reasoning");
    let exported = command(&mut core, json!({"op":"export"}));
    assert_eq!(exported["ok"], true);
    assert_eq!(exported["data"]["conversations"][0]["messages"][1]["status"], "interrupted");
    let mut restored = WodexCore::new(Some(&exported["data"].to_string()));
    assert!(!restored.state.is_streaming);
    assert_eq!(restored.state.active_id, "chat-2");
    assert_eq!(restored.state.conversations[0].messages[1].status, MessageStatus::Interrupted);
    assert_eq!(command(&mut restored, json!({"op":"retry"}))["ok"], true);

    let mut saved = exported["data"].clone();
    saved["conversations"][0]["messages"][1]["status"] = json!("streaming");
    let recovered = WodexCore::new(Some(&saved.to_string()));
    assert!(recovered.state.error.as_ref().unwrap().contains("interrupted"));

    let mut hostile_saves = vec![String::from("not json"), json!({"version":99}).to_string()];
    for (field, value) in
      [("version", json!(999)), ("active_id", json!("missing")), ("next_id", json!(1))]
    {
      let mut bad = exported["data"].clone();
      bad[field] = value;
      hostile_saves.push(bad.to_string());
    }
    let mut duplicate = exported["data"].clone();
    duplicate["conversations"][1]["id"] = duplicate["conversations"][0]["id"].clone();
    hostile_saves.push(duplicate.to_string());
    for saved in hostile_saves {
      let mut invalid = WodexCore::new(Some(&saved));
      assert!(invalid.state.error.as_ref().unwrap().contains("could not be loaded"));
      assert_eq!(command(&mut invalid, json!({"op":"export"}))["ok"], false);
      assert_eq!(command(&mut invalid, json!({"op":"reset_storage"}))["ok"], true);
      assert_eq!(command(&mut invalid, json!({"op":"export"}))["ok"], true);
    }
  }

  #[test]
  fn conversation_lifecycle_and_limits() {
    let mut core = ready();
    for _ in 1..MAX_CONVERSATIONS {
      assert_eq!(command(&mut core, json!({"op":"new_chat"}))["ok"], true);
    }
    assert_eq!(command(&mut core, json!({"op":"new_chat"}))["ok"], false);
    assert_eq!(command(&mut core, json!({"op":"select_chat","id":"chat-1"}))["ok"], true);
    assert_eq!(command(&mut core, json!({"op":"delete_chat","id":"chat-1"}))["ok"], true);
    assert_ne!(core.state.active_id, "chat-1");
    let ids: Vec<_> =
      core.state.conversations.iter().map(|conversation| conversation.id.clone()).collect();
    for id in ids {
      assert_eq!(command(&mut core, json!({"op":"delete_chat","id":id}))["ok"], true);
    }
    assert_eq!(core.state.conversations.len(), 1);
    assert_eq!(core.state.conversations[0].id, "chat-33");
    assert_eq!(
      command(&mut core, json!({"op":"send","content":"x".repeat(MAX_USER_BYTES + 1)}))["ok"],
      false
    );
    command(&mut core, json!({"op":"send","content":"Small"}));
    let delta_content = "x".repeat(2048);
    for _ in 0..MAX_ANSWER_BYTES / 2048 {
      assert_eq!(delta(&mut core, &delta_content, "")["ok"], true);
    }
    assert_eq!(delta(&mut core, "overflow", "")["ok"], false);
    assert!(!core.state.is_streaming);
    assert_eq!(core.state.conversations[0].messages[1].content.len(), MAX_ANSWER_BYTES);
  }

  #[test]
  fn nul_in_streamed_text_cannot_break_rendering_or_saved_history() {
    for (content, reasoning) in [("bad\0answer", ""), ("", "bad\0reasoning")] {
      let mut core = ready();
      command(&mut core, json!({"op":"send","content":"Hello"}));
      assert_eq!(delta(&mut core, "Safe partial answer", "")["ok"], true);
      assert_eq!(delta(&mut core, content, reasoning)["ok"], false);
      assert!(!core.state.is_streaming);
      assert_eq!(core.state.conversations[0].messages[1].content, "Safe partial answer");
      let saved = command(&mut core, json!({"op":"export"}))["data"].to_string();
      let mut restored = WodexCore::new(Some(&saved));
      assert_eq!(command(&mut restored, json!({"op":"export"}))["ok"], true);
    }
  }

  #[test]
  fn control_heavy_responses_remain_persistable_at_the_storage_limit() {
    let mut core = ready();
    command(&mut core, json!({"op":"send","content":"First"}));
    let control_text = "\u{0001}".repeat(4096);
    for _ in 0..MAX_ANSWER_BYTES / 4096 {
      assert_eq!(delta(&mut core, &control_text, "")["ok"], true);
    }
    finish_successfully(&mut core);
    assert_eq!(command(&mut core, json!({"op":"send","content":"Second"}))["ok"], true);
    let mut limited = false;
    for _ in 0..MAX_ANSWER_BYTES / 4096 {
      let response = delta(&mut core, &control_text, "");
      if response["ok"] == false {
        assert!(response["error"].as_str().unwrap().contains("storage is full"));
        limited = true;
        break;
      }
    }
    assert!(limited);
    assert!(!core.state.is_streaming);
    let exported = command(&mut core, json!({"op":"export"}));
    assert_eq!(exported["ok"], true);
    let saved = exported["data"].to_string();
    assert!(saved.len() <= MAX_SAVED_BYTES);
    let mut restored = WodexCore::new(Some(&saved));
    assert_eq!(command(&mut restored, json!({"op":"export"}))["ok"], true);
    let active_id = restored.state.active_id.clone();
    assert_eq!(command(&mut restored, json!({"op":"delete_chat","id":active_id}))["ok"], true);
    assert_eq!(restored.encoded_text_bytes, 0);
    assert_eq!(command(&mut restored, json!({"op":"send","content":"Recovered"}))["ok"], true);
  }

  #[test]
  fn control_heavy_user_input_is_rejected_before_it_could_break_saving() {
    let mut core = ready();
    let content = "\u{0001}".repeat(32 * 1024);
    let mut limited = false;
    for _ in 0..64 {
      let response = command(&mut core, json!({"op":"send","content":content}));
      if response["ok"] == false {
        assert!(response["error"].as_str().unwrap().contains("storage is full"));
        limited = true;
        break;
      }
      command(&mut core, json!({"op":"cancel"}));
    }
    assert!(limited);
    assert_eq!(command(&mut core, json!({"op":"export"}))["ok"], true);
    let cache_before_retry = core.encoded_text_bytes;
    assert_eq!(command(&mut core, json!({"op":"retry"}))["ok"], true);
    assert_eq!(core.encoded_text_bytes, cache_before_retry);
  }

  #[test]
  fn exhausted_saved_ids_cannot_remove_the_last_chat_or_break_reload() {
    let mut core = ready();
    let mut saved = command(&mut core, json!({"op":"export"}))["data"].clone();
    saved["next_id"] = json!(u64::MAX - 1);
    saved["settings"]["base_url"] = json!("https://api.deepseek.com/");
    let mut restored = WodexCore::new(Some(&saved.to_string()));
    assert_eq!(command(&mut restored, json!({"op":"new_chat"}))["ok"], true);
    assert_eq!(restored.next_id, u64::MAX);
    let exported = command(&mut restored, json!({"op":"export"}));
    assert_eq!(exported["ok"], true);
    let mut exhausted = WodexCore::new(Some(&exported["data"].to_string()));
    assert!(exhausted.load_error.is_none());
    assert_eq!(command(&mut exhausted, json!({"op":"new_chat"}))["ok"], false);
    let active_id = exhausted.state.active_id.clone();
    assert_eq!(command(&mut exhausted, json!({"op":"delete_chat","id":"chat-1"}))["ok"], true);
    assert_eq!(command(&mut exhausted, json!({"op":"delete_chat","id":active_id}))["ok"], false);
    assert_eq!(exhausted.state.conversations.len(), 1);
    assert_eq!(command(&mut exhausted, json!({"op":"snapshot"}))["ok"], true);
    let response = command(&mut exhausted, json!({"op":"send","content":"Still usable"}));
    assert_eq!(response["request"]["url"], "https://api.deepseek.com/chat/completions");
  }

  #[test]
  fn legacy_saves_default_to_no_update_source_and_source_settings_are_validated() {
    let mut core = ready();
    assert_eq!(core.state.settings.update_repository, OFFICIAL_UPDATE_REPOSITORY);
    assert_eq!(core.state.settings.update_channel, UpdateChannel::Version);
    let mut saved = command(&mut core, json!({"op":"export"}))["data"].clone();
    saved["settings"].as_object_mut().unwrap().remove("update_repository");
    saved["settings"].as_object_mut().unwrap().remove("update_channel");
    let mut restored = WodexCore::new(Some(&saved.to_string()));
    assert!(restored.load_error.is_none());
    assert!(restored.state.settings.update_repository.is_empty());
    assert_eq!(restored.state.settings.update_channel, UpdateChannel::Version);
    assert_eq!(
      command(
        &mut restored,
        json!({"op":"settings","settings":{"update_repository":"example/wodex"}})
      )["ok"],
      true
    );
    for invalid in [
      "../repo",
      "https://github.com/example/wodex",
      "example/repo/extra",
      "example/..",
      "example/repo?token=x",
    ] {
      assert_eq!(
        command(&mut restored, json!({"op":"settings","settings":{"update_repository":invalid}}))
          ["ok"],
        false
      );
      assert_eq!(restored.state.settings.update_repository, "example/wodex");
    }
    let exported = command(&mut restored, json!({"op":"export"}));
    let restored_again = WodexCore::new(Some(&exported["data"].to_string()));
    assert_eq!(restored_again.state.settings.update_repository, "example/wodex");
    assert_eq!(
      command(&mut restored, json!({"op":"settings","settings":{"update_repository":""}}))["ok"],
      true
    );
    let exported = command(&mut restored, json!({"op":"export"}));
    let disabled = WodexCore::new(Some(&exported["data"].to_string()));
    assert!(disabled.load_error.is_none());
    assert!(disabled.state.settings.update_repository.is_empty());
  }

  #[test]
  fn update_channel_is_explicit_validated_and_persisted() {
    let mut core = ready();
    assert_eq!(
      command(&mut core, json!({"op":"settings","settings":{"update_channel":"nightly"}}))["ok"],
      true
    );
    assert_eq!(core.state.settings.update_channel, UpdateChannel::Nightly);
    for invalid in [json!("stable"), json!("Nightly"), json!(""), json!(42), json!({})] {
      assert_eq!(
        command(&mut core, json!({"op":"settings","settings":{"update_channel":invalid}}))["ok"],
        false
      );
      assert_eq!(core.state.settings.update_channel, UpdateChannel::Nightly);
    }
    let exported = command(&mut core, json!({"op":"export"}));
    assert_eq!(exported["data"]["settings"]["update_channel"], "nightly");
    let restored = WodexCore::new(Some(&exported["data"].to_string()));
    assert!(restored.load_error.is_none());
    assert_eq!(restored.state.settings.update_channel, UpdateChannel::Nightly);
    assert_eq!(
      command(&mut core, json!({"op":"settings","settings":{"update_channel":"version"}}))["ok"],
      true
    );
    assert_eq!(core.state.settings.update_channel, UpdateChannel::Version);
  }
}
