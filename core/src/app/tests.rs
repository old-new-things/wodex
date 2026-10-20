use super::*;
use crate::state::MessageStatus;
use alloc::{collections::VecDeque, vec};

#[derive(Default)]
struct MockHost {
  saved: Vec<u8>,
  writes: Vec<String>,
  recovery: Vec<Vec<u8>>,
  environment_key: String,
  requests: Vec<(String, String, String)>,
  transport: VecDeque<TransportEvent>,
  inputs: VecDeque<Input>,
  network_active: bool,
  update: UpdateStatus,
  update_supported: bool,
  update_checks: Vec<String>,
  downloads: usize,
  installs: usize,
  fail_save: bool,
  fail_preserve: bool,
  fail_start: bool,
  system_keyboard: bool,
  edited: Option<String>,
  text_input: bool,
}

impl Host for MockHost {
  fn now_ms(&self) -> u64 {
    0
  }
  fn running(&self) -> bool {
    true
  }
  fn is_switch(&self) -> bool {
    self.system_keyboard
  }
  fn load(&mut self) -> Result<Vec<u8>, String> {
    Ok(self.saved.clone())
  }
  fn save(&mut self, content: &str) -> Result<(), String> {
    if self.fail_save {
      return Err("disk full".into());
    }
    self.writes.push(content.into());
    Ok(())
  }
  fn preserve(&mut self, content: &[u8]) -> Result<(), String> {
    if self.fail_preserve {
      return Err("cannot preserve".into());
    }
    self.recovery.push(content.to_vec());
    Ok(())
  }
  fn environment_key(&self) -> String {
    self.environment_key.clone()
  }
  fn start(&mut self, url: &str, key: &str, body: &str) -> Result<(), String> {
    if self.fail_start {
      return Err("start failed".into());
    }
    self.requests.push((url.into(), key.into(), body.into()));
    self.network_active = true;
    Ok(())
  }
  fn cancel(&mut self) {}
  fn network_active(&self) -> bool {
    self.network_active
  }
  fn poll_transport(&mut self) -> Option<TransportEvent> {
    let event = self.transport.pop_front();
    if matches!(event, Some(TransportEvent::Finished { .. })) {
      self.network_active = false;
    }
    event
  }
  fn update_status(&self) -> UpdateStatus {
    self.update.clone()
  }
  fn update_supported(&self) -> bool {
    self.update_supported
  }
  fn update_active(&self) -> bool {
    matches!(self.update.phase, UpdatePhase::Checking | UpdatePhase::Downloading)
  }
  fn update_check(&mut self, repository: &str) -> Result<(), String> {
    self.update_checks.push(repository.into());
    self.update.phase = UpdatePhase::Checking;
    Ok(())
  }
  fn update_download(&mut self) -> Result<(), String> {
    self.downloads += 1;
    self.update.phase = UpdatePhase::Downloading;
    Ok(())
  }
  fn update_cancel(&mut self) {}
  fn prepare_install(&mut self) -> Result<bool, String> {
    self.installs += 1;
    Ok(self.update.phase == UpdatePhase::Ready)
  }
  fn begin_frame(&mut self) {}
  fn poll_input(&mut self) -> Option<Input> {
    self.inputs.pop_front()
  }
  fn text_input(&mut self, enabled: bool) {
    self.text_input = enabled;
  }
  fn clipboard(&mut self) -> String {
    "粘贴".into()
  }
  fn edit_system(&mut self, _: &str, initial: &str, _: bool) -> Result<String, String> {
    Ok(self.edited.take().unwrap_or_else(|| initial.into()))
  }
}

fn app() -> App<MockHost> {
  let host = MockHost { environment_key: "test-key".into(), ..Default::default() };
  App::new(host, "0.2.0", false, false).unwrap()
}

fn last_saved(app: &App<MockHost>) -> Value {
  serde_json::from_str(app.host.writes.last().unwrap()).unwrap()
}

fn set_repository(app: &mut App<MockHost>, repository: &str) {
  app.commit_edit(EditTarget::Setting(Setting::UpdateRepository), repository.into());
}

#[test]
fn corrupt_storage_is_preserved_byte_for_byte_before_reset() {
  let bytes = vec![b'{', 0xff, 0, b'}'];
  let host = MockHost { saved: bytes.clone(), ..Default::default() };
  let mut app = App::new(host, "0.2.0", false, false).unwrap();
  assert_eq!(app.host.recovery, vec![bytes.clone()]);
  assert!(app.ui.notice.contains("recovery copy"));
  assert!(app.save());
  assert_eq!(last_saved(&app)["version"], 1);
  let host = MockHost { saved: bytes, fail_preserve: true, ..Default::default() };
  assert!(App::new(host, "0.2.0", false, false).is_err());
}

#[test]
fn environment_key_stays_in_memory_until_the_user_edits_the_key() {
  let mut app = app();
  app.ui.draft = "你好".into();
  app.send();
  assert_eq!(app.host.requests[0].1, "test-key");
  assert_eq!(last_saved(&app)["settings"]["api_key"], "");
  assert!(app.ui.draft.is_empty());
  app.stop();
  app.host.network_active = false;
  app.commit_edit(EditTarget::Setting(Setting::ApiKey), "new-saved-key".into());
  assert!(!app.ui.environment_key);
  assert_eq!(last_saved(&app)["settings"]["api_key"], "new-saved-key");
}

#[test]
fn done_event_waits_for_http_completion_and_cancellation_keeps_partial_text() {
  let mut app = app();
  app.ui.draft = "question".into();
  app.send();
  app.host.transport.push_back(TransportEvent::Data(
    b"data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\ndata: [DONE]\n\n".to_vec(),
  ));
  app.tick();
  assert!(app.streaming());
  app.stop();
  assert!(!app.streaming());
  let message = &app.core.state().conversations[0].messages[1];
  assert_eq!(message.content, "partial");
  assert_eq!(message.status, MessageStatus::Interrupted);
  app.ui.draft = "next".into();
  app.send();
  assert_eq!(app.host.requests.len(), 1, "cancelled worker still owns the transport");
  app.host.transport.push_back(TransportEvent::Finished { http_status: 200, error: String::new() });
  app.tick();
  assert_eq!(app.core.state().conversations[0].messages[1].status, MessageStatus::Interrupted);
  app.send();
  assert_eq!(app.host.requests.len(), 2);
}

#[test]
fn completion_saves_and_retry_uses_the_same_user_turn() {
  let mut app = app();
  app.ui.draft = "question".into();
  app.send();
  app.host.transport.extend([
    TransportEvent::Data(b"data: {\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".to_vec()),
    TransportEvent::Finished { http_status: 200, error: String::new() },
  ]);
  app.tick();
  assert!(!app.streaming());
  assert_eq!(last_saved(&app)["conversations"][0]["messages"][1]["content"], "answer");
  app.dispatch(Action::Retry);
  assert!(app.streaming());
  assert_eq!(app.core.state().conversations[0].messages.len(), 2);
  let body: Value = serde_json::from_str(&app.host.requests[1].2).unwrap();
  assert_eq!(body["messages"].as_array().unwrap().len(), 2); // system + original user
}

#[test]
fn failed_request_start_preserves_draft_and_does_not_leave_streaming_state() {
  let mut app = app();
  app.host.fail_start = true;
  app.ui.draft = "keep this draft".into();
  app.send();
  assert!(!app.streaming());
  assert_eq!(app.ui.draft, "keep this draft");
  assert_eq!(app.core.state().conversations[0].messages[1].status, MessageStatus::Error);
}

#[test]
fn changing_version_source_invalidates_previous_download_and_install_actions() {
  let mut app = app();
  app.host.update_supported = true;
  set_repository(&mut app, "first/wodex");
  app.dispatch(Action::UpdateCheck);
  assert_eq!(app.host.update_checks, vec!["first/wodex"]);
  app.host.update.phase = UpdatePhase::Ready;
  set_repository(&mut app, "second/wodex");
  app.dispatch(Action::UpdateDownload);
  app.dispatch(Action::UpdateInstall);
  assert_eq!(app.host.downloads, 0);
  assert_eq!(app.host.installs, 0);
  assert!(app.running);
  set_repository(&mut app, "");
  app.dispatch(Action::UpdateDownload);
  app.dispatch(Action::UpdateInstall);
  assert_eq!(app.host.downloads, 0, "clearing the source must not unlock a stale download");
  assert_eq!(app.host.installs, 0, "clearing the source must not unlock a staged install");
}

#[test]
fn installation_requires_an_explicit_action_idle_chat_and_successful_save() {
  let mut app = app();
  app.host.update_supported = true;
  set_repository(&mut app, "owner/wodex");
  app.checked_repository = "owner/wodex".into();
  app.host.update.phase = UpdatePhase::Ready;
  app.host.fail_save = true;
  app.dispatch(Action::UpdateInstall);
  assert_eq!(app.host.installs, 0);
  assert!(app.running);
  app.host.fail_save = false;
  app.host.network_active = true;
  app.dispatch(Action::UpdateInstall);
  assert_eq!(app.host.installs, 0);
  app.host.network_active = false;
  app.dispatch(Action::UpdateInstall);
  assert_eq!(app.host.installs, 1);
  assert!(!app.running);

  let mut app = self::app();
  app.host.update.phase = UpdatePhase::Ready;
  app.close();
  assert_eq!(app.host.installs, 0, "normal exit must not install a staged update");
}

#[test]
fn desktop_editor_commit_cancel_and_clipboard_use_rust_application_actions() {
  let mut app = app();
  app.compose();
  assert!(app.host.text_input);
  app.input(Input::Text("中文".into()));
  app.input(Input::Key { key: Key::V, modifiers: CTRL });
  app.input(Input::Key { key: Key::Enter, modifiers: 0 });
  assert_eq!(app.ui.draft, "中文粘贴");
  assert!(!app.host.text_input);
  app.compose();
  app.input(Input::Text("discard".into()));
  app.input(Input::Key { key: Key::Escape, modifiers: 0 });
  assert_eq!(app.ui.draft, "中文粘贴");
  assert!(app.editor.is_none());
}

#[test]
fn switch_keyboard_result_is_committed_through_the_same_rust_actions() {
  let mut app = app();
  app.host.system_keyboard = true;
  app.host.edited = Some("Switch 中文".into());
  app.compose();
  assert!(app.editor.is_none());
  assert_eq!(app.ui.draft, "Switch 中文");
  app.host.edited = Some("deepseek-reasoner".into());
  app.edit_setting(Setting::Model);
  assert_eq!(app.core.state().settings.model, "deepseek-reasoner");
}

#[test]
fn page_change_invalidates_stale_buttons_and_delete_requires_confirmation() {
  let mut app = app();
  app.buttons.push(Button {
    id: "settings".into(),
    area: canvas::Rect { x: 1, y: 1, w: 10, h: 10 },
    action: Action::Settings,
  });
  app.ui.focused = "settings".into();
  app.activate();
  assert_eq!(app.ui.page, Page::Settings);
  assert!(app.buttons.is_empty());
  app.activate();
  assert_eq!(app.ui.page, Page::Settings);
  app.dispatch(Action::DeleteChat);
  assert_eq!(app.core.state().active_id, "chat-1");
  app.dispatch(Action::DeleteChat);
  assert_eq!(app.core.state().active_id, "chat-2");
}

#[test]
fn demo_is_offline_and_does_not_read_or_write_user_history() {
  let host = MockHost { saved: b"broken".to_vec(), ..Default::default() };
  let mut app = App::new(host, "0.2.0", true, false).unwrap();
  assert_eq!(app.core.state().conversations[0].messages.len(), 2);
  app.ui.draft = "offline".into();
  app.send();
  app.dispatch(Action::UpdateCheck);
  app.close();
  assert!(app.host.requests.is_empty());
  assert!(app.host.writes.is_empty());
  assert!(app.host.recovery.is_empty());
  assert!(app.host.update_checks.is_empty());
}
