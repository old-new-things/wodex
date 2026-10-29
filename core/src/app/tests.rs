use super::*;
use crate::backend::home_menu::HomeMenuStatus;
use crate::backend::images::ImageResult;
use crate::backend::transfer::{TransferExports, TransferImport, TransferStatus};
use crate::provider::{Provider, TokenLimitField};
use crate::state::MessageStatus;
use alloc::{collections::VecDeque, vec};

#[derive(Default)]
struct MockHost {
  saved: Vec<u8>,
  writes: Vec<String>,
  recovery: Vec<Vec<u8>>,
  environment_key: String,
  openai_environment_key: String,
  requests: Vec<(String, String, String)>,
  transport: VecDeque<TransportEvent>,
  inputs: VecDeque<Input>,
  network_active: bool,
  update: UpdateStatus,
  update_supported: bool,
  update_checks: Vec<(String, UpdateChannel)>,
  downloads: usize,
  installs: usize,
  fail_save: bool,
  fail_load: bool,
  save_error_after_commit: bool,
  corrupt_save_on_error: bool,
  save_attempts: usize,
  fail_preserve: bool,
  fail_start: bool,
  system_keyboard: bool,
  edited: Option<String>,
  text_input: bool,
  image_requests: Vec<String>,
  image_results: VecDeque<ImageResult>,
  image_active: bool,
  image_cancellations: usize,
  home_menu: HomeMenuStatus,
  home_menu_checks: usize,
  home_menu_installs: usize,
  fail_home_menu_check: bool,
  fail_home_menu_install: bool,
  installation_writes: usize,
  transfer: TransferStatus,
  transfer_exports: Vec<TransferExports>,
  transfer_imports: VecDeque<TransferImport>,
  transfer_results: Vec<String>,
  transfer_stops: usize,
  fail_transfer_start: bool,
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
    if self.fail_load {
      return Err("Cannot read storage.".into());
    }
    Ok(self.saved.clone())
  }
  fn save(&mut self, content: &str) -> Result<(), String> {
    self.save_attempts += 1;
    if self.fail_save {
      return Err("disk full".into());
    }
    self.writes.push(content.into());
    self.saved = content.as_bytes().to_vec();
    if self.save_error_after_commit {
      if self.corrupt_save_on_error {
        self.saved = b"unverified content".to_vec();
      }
      return Err("Could not sync storage directory.".into());
    }
    Ok(())
  }
  fn preserve(&mut self, content: &[u8]) -> Result<(), String> {
    if self.fail_preserve {
      return Err("cannot preserve".into());
    }
    self.recovery.push(content.to_vec());
    Ok(())
  }
  fn environment_key(&self, provider: Provider) -> String {
    match provider {
      Provider::DeepSeek => self.environment_key.clone(),
      Provider::OpenAiCompatible => self.openai_environment_key.clone(),
    }
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
  fn request_image(&mut self, url: &str) -> bool {
    if self.image_active {
      return false;
    }
    self.image_requests.push(url.into());
    self.image_active = true;
    true
  }
  fn poll_image(&mut self) -> Option<ImageResult> {
    let result = self.image_results.pop_front();
    if result.is_some() {
      self.image_active = false;
    }
    result
  }
  fn cancel_images(&mut self) {
    // Production cancellation keeps the worker occupied until it reports back.
    self.image_cancellations += 1;
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
  fn update_check(&mut self, repository: &str, channel: UpdateChannel) -> Result<(), String> {
    self.update_checks.push((repository.into(), channel));
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
  fn home_menu_status(&self) -> HomeMenuStatus {
    self.home_menu.clone()
  }
  fn home_menu_active(&self) -> bool {
    matches!(self.home_menu.phase, HomeMenuPhase::Checking | HomeMenuPhase::Installing)
  }
  fn home_menu_check(&mut self) -> Result<(), String> {
    self.home_menu_checks += 1;
    if self.fail_home_menu_check {
      return Err("HOME Menu check failed.".into());
    }
    self.home_menu.phase = HomeMenuPhase::Checking;
    Ok(())
  }
  fn home_menu_install(&mut self) -> Result<(), String> {
    self.home_menu_installs += 1;
    self.installation_writes = self.writes.len();
    if self.fail_home_menu_install {
      return Err("HOME Menu installation failed.".into());
    }
    self.home_menu.phase = HomeMenuPhase::Installing;
    Ok(())
  }
  fn transfer_status(&self) -> TransferStatus {
    self.transfer.clone()
  }
  fn transfer_active(&self) -> bool {
    self.transfer.active
  }
  fn transfer_start(&mut self, exports: TransferExports) -> Result<(), String> {
    if self.fail_transfer_start {
      return Err("Could not start PC transfer.".into());
    }
    self.transfer_exports.push(exports);
    self.transfer.active = true;
    self.transfer.url = "http://192.0.2.1:8080/".into();
    self.transfer.code = "123456".into();
    Ok(())
  }
  fn transfer_stop(&mut self) {
    self.transfer_stops += 1;
    self.transfer.active = false;
    self.transfer_imports.clear();
  }
  fn transfer_take_import(&mut self) -> Option<TransferImport> {
    self.transfer_imports.pop_front()
  }
  fn transfer_resolve_import(&mut self, message: String, exports: Option<TransferExports>) {
    self.transfer_results.push(message);
    if let Some(exports) = exports {
      self.transfer_exports.push(exports);
    }
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

fn image_app(content: &str) -> App<MockHost> {
  let mut app = app();
  app.core.execute(Command::Send { content: "Show an image.".into() }).unwrap();
  let event = serde_json::json!({"choices": [{"delta": {"content": content}}]});
  app.core.feed(format!("data: {event}\n\n").as_bytes());
  app.core.execute(Command::Finish { http_status: Some(200), error: None }).unwrap();
  app
}

#[test]
fn image_requests_follow_visible_cards_and_completed_results_are_cached() {
  use crate::backend::images::DecodedImage;
  let url = "https://example.org/image.png";
  let source = format!("![图片]({url})");
  let mut app = image_app(&source);
  let (_backend, mut canvas) = canvas::test_backend::canvas();
  app.render(&mut canvas).unwrap();
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url]);
  app.host.image_results.push_back(ImageResult {
    url: url.into(),
    result: Ok(DecodedImage { width: 1, height: 1, rgba: vec![255, 0, 0, 255] }),
  });
  app.render(&mut canvas).unwrap();
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url]);
  assert_eq!(app.core.state().conversations[0].messages[1].content, source);
  let cancellations = app.host.image_cancellations;
  app.dispatch(Action::NewChat);
  app.render(&mut canvas).unwrap();
  assert!(app.host.image_cancellations > cancellations);
}

#[test]
fn offline_images_demo_and_update_transfers_do_not_start_image_downloads() {
  let mut app = image_app("![image](https://example.org/image.png)");
  let (_backend, mut canvas) = canvas::test_backend::canvas();
  app.set_image_loading(false);
  app.render(&mut canvas).unwrap();
  assert!(app.host.image_requests.is_empty());
  app.set_image_loading(true);
  app.demo = true;
  app.render(&mut canvas).unwrap();
  assert!(app.host.image_requests.is_empty());
  app.demo = false;
  app.host.update.phase = UpdatePhase::Downloading;
  app.render(&mut canvas).unwrap();
  assert!(app.host.image_requests.is_empty());
  app.host.update.phase = UpdatePhase::Idle;
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests.len(), 1);
  let cancellations = app.host.image_cancellations;
  app.close();
  assert!(app.host.image_cancellations > cancellations);
}

#[test]
fn failed_images_are_not_retried_each_frame_and_hidden_images_stay_idle() {
  let url = "https://example.org/image.png";
  let mut app = image_app(&format!("![image]({url})\n\n{}", "visible text\n".repeat(40)));
  let (_backend, mut canvas) = canvas::test_backend::canvas();
  app.render(&mut canvas).unwrap();
  assert!(app.host.image_requests.is_empty());
  app.ui.follow_bottom = false;
  app.ui.scroll = 0;
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url]);
  app
    .host
    .image_results
    .push_back(ImageResult { url: url.into(), result: Err("Image is unavailable.".into()) });
  app.render(&mut canvas).unwrap();
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url]);
}

#[test]
fn cancelled_image_from_another_chat_cannot_supply_pixels_for_the_same_url() {
  use crate::backend::images::DecodedImage;
  let url = "https://example.org/image.png";
  let source = format!("![image]({url})");
  let mut app = image_app(&source);
  let (backend, mut canvas) = canvas::test_backend::canvas();
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url]);
  app.dispatch(Action::NewChat);
  app.core.execute(Command::Send { content: "Show the image again.".into() }).unwrap();
  let event = serde_json::json!({"choices": [{"delta": {"content": source}}]});
  app.core.feed(format!("data: {event}\n\n").as_bytes());
  app.core.execute(Command::Finish { http_status: Some(200), error: None }).unwrap();
  app.render(&mut canvas).unwrap();
  assert_eq!(app.host.image_requests, vec![url], "the cancelled worker still owns its slot");
  // Even a success racing with cancellation must be discarded by the new chat.
  app.host.image_results.push_back(ImageResult {
    url: url.into(),
    result: Ok(DecodedImage { width: 1, height: 1, rgba: vec![255, 0, 0, 255] }),
  });
  app.render(&mut canvas).unwrap();
  assert!(backend.image_uploads.is_empty(), "stale pixels must never reach SDL");
  assert_eq!(app.host.image_requests, vec![url, url]);
  app.host.image_results.push_back(ImageResult {
    url: url.into(),
    result: Ok(DecodedImage { width: 1, height: 1, rgba: vec![0, 255, 0, 255] }),
  });
  app.render(&mut canvas).unwrap();
  assert_eq!(backend.image_uploads, vec![(1, 1)]);
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
fn resume_action_keeps_checked_source_and_requires_a_selected_download() {
  let mut app = app();
  app.host.update_supported = true;
  app.ui.page = Page::Updates;
  app.dispatch(Action::UpdateCheck);
  let (backend, mut canvas) = canvas::test_backend::canvas();
  for phase in [UpdatePhase::Cancelled, UpdatePhase::Error] {
    app.host.update.phase = phase;
    app.host.update.can_download = true;
    app.host.update.downloaded = 1024;
    app.host.update.total = 4096;
    app.render(&mut canvas).unwrap();
    assert!(app.buttons.iter().any(|button| button.action == Action::UpdateDownload));
    assert!(backend.drawn_text.iter().any(|text| text == "Resume download"));
    assert_eq!(app.update_focus(), "update-download");
    app.dispatch(Action::UpdateDownload);
  }
  assert_eq!(app.host.downloads, 2);
  app.host.update.phase = UpdatePhase::Error;
  app.host.update.can_download = false;
  app.render(&mut canvas).unwrap();
  assert!(!app.buttons.iter().any(|button| button.action == Action::UpdateDownload));
  app.dispatch(Action::UpdateDownload);
  assert_eq!(app.host.downloads, 2);
  app.host.update.can_download = true;
  set_repository(&mut app, "another/wodex");
  app.render(&mut canvas).unwrap();
  assert!(!app.buttons.iter().any(|button| button.action == Action::UpdateDownload));
  app.dispatch(Action::UpdateDownload);
  assert_eq!(app.host.downloads, 2, "a retained partial cannot bypass a fresh source check");
  drop(canvas);
}

#[test]
fn changing_version_source_invalidates_previous_download_and_install_actions() {
  let mut app = app();
  app.host.update_supported = true;
  set_repository(&mut app, "first/wodex");
  app.dispatch(Action::UpdateCheck);
  assert_eq!(app.host.update_checks, vec![("first/wodex".into(), UpdateChannel::Version)]);
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
fn changing_channel_invalidates_available_and_staged_updates_even_when_changed_back() {
  let mut app = app();
  app.host.update_supported = true;
  app.ui.page = Page::Updates;
  let (_backend, mut canvas) = canvas::test_backend::canvas();
  for phase in [UpdatePhase::Available, UpdatePhase::Ready] {
    app.dispatch(Action::UpdateCheck);
    app.host.update.phase = phase;
    app.dispatch(Action::UpdateChannel(UpdateChannel::Nightly));
    assert_eq!(app.core.state().settings.update_channel, UpdateChannel::Nightly);
    assert_eq!(last_saved(&app)["settings"]["update_channel"], "nightly");
    app.render(&mut canvas).unwrap();
    assert!(!app
      .buttons
      .iter()
      .any(|button| { matches!(button.action, Action::UpdateDownload | Action::UpdateInstall) }));
    app.dispatch(Action::UpdateDownload);
    app.dispatch(Action::UpdateInstall);
    app.dispatch(Action::UpdateChannel(UpdateChannel::Version));
    app.dispatch(Action::UpdateDownload);
    app.dispatch(Action::UpdateInstall);
    assert_eq!(app.host.downloads, 0);
    assert_eq!(app.host.installs, 0);
    assert!(app.running);
  }
  app.dispatch(Action::UpdateChannel(UpdateChannel::Nightly));
  app.dispatch(Action::UpdateCheck);
  assert_eq!(
    app.host.update_checks.last(),
    Some(&(OFFICIAL_UPDATE_REPOSITORY.into(), UpdateChannel::Nightly))
  );
  app.host.update.phase = UpdatePhase::Available;
  app.dispatch(Action::UpdateDownload);
  assert_eq!(app.host.downloads, 1);
  drop(canvas);
}

#[test]
fn official_source_can_be_enabled_without_a_keyboard_and_empty_source_never_checks() {
  let mut app = app();
  set_repository(&mut app, "");
  app.dispatch(Action::UpdateCheck);
  assert!(app.host.update_checks.is_empty());
  app.dispatch(Action::UpdateChannel(UpdateChannel::Nightly));
  app.dispatch(Action::UpdateOfficial);
  assert_eq!(app.core.state().settings.update_repository, OFFICIAL_UPDATE_REPOSITORY);
  assert_eq!(last_saved(&app)["settings"]["update_repository"], OFFICIAL_UPDATE_REPOSITORY);
  assert_eq!(app.core.state().settings.update_channel, UpdateChannel::Nightly);
  assert_eq!(app.ui.focused, "update-check");
  assert!(app.checked_source.is_none());
}

#[test]
fn active_transfer_and_demo_guard_update_settings_even_for_stale_actions() {
  let mut app = app();
  set_repository(&mut app, "custom/wodex");
  for phase in [UpdatePhase::Checking, UpdatePhase::Downloading] {
    app.host.update.phase = phase;
    app.dispatch(Action::UpdateChannel(UpdateChannel::Nightly));
    app.dispatch(Action::UpdateOfficial);
    set_repository(&mut app, "another/wodex");
    assert_eq!(app.core.state().settings.update_channel, UpdateChannel::Version);
    assert_eq!(app.core.state().settings.update_repository, "custom/wodex");
  }
  app.host.update.phase = UpdatePhase::Idle;
  app.demo = true;
  app.dispatch(Action::UpdateChannel(UpdateChannel::Nightly));
  app.dispatch(Action::UpdateOfficial);
  set_repository(&mut app, "another/wodex");
  assert_eq!(app.core.state().settings.update_channel, UpdateChannel::Version);
  assert_eq!(app.core.state().settings.update_repository, "custom/wodex");
}

#[test]
fn installation_requires_an_explicit_action_idle_chat_and_successful_save() {
  let mut app = app();
  app.host.update_supported = true;
  set_repository(&mut app, "owner/wodex");
  app.checked_source = Some(("owner/wodex".into(), UpdateChannel::Version));
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
fn home_menu_checks_on_open_and_requires_explicit_install_after_successful_save() {
  let mut app = app();
  app.dispatch(Action::HomeMenu);
  assert_eq!(app.ui.page, Page::HomeMenu);
  assert_eq!(app.host.home_menu_checks, 1);
  assert_eq!(app.host.home_menu_installs, 0);
  assert_eq!(app.ui.focused, "home-menu-working");
  app.host.home_menu.phase = HomeMenuPhase::Ready;
  app.tick();
  assert_eq!(app.ui.focused, "home-menu-install");
  assert_eq!(app.host.home_menu_installs, 0, "a successful check never installs automatically");
  app.host.fail_save = true;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 0);
  assert!(app.ui.notice.contains("Could not save"));
  app.host.fail_save = false;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 1);
  assert!(app.host.installation_writes > 0, "history must be saved before installation starts");
  assert_eq!(app.host.installation_writes, app.host.writes.len());
  assert!(app.running);
  assert_eq!(app.ui.focused, "home-menu-working");
  app.host.home_menu.phase = HomeMenuPhase::Installed;
  app.tick();
  assert_eq!(app.ui.focused, "home-menu-back");
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 1, "an installed tile is never overwritten");
  app.back();
  assert_eq!(app.ui.page, Page::Settings);
  assert_eq!(app.ui.focused, "home-menu");
}

#[test]
fn home_menu_rejects_demo_busy_unsupported_and_stale_install_actions() {
  let mut app = app();
  app.host.network_active = true;
  app.show_home_menu();
  assert_eq!(app.host.home_menu_checks, 0);
  assert!(app.ui.notice.contains("current response"));
  app.host.network_active = false;
  app.host.update.phase = UpdatePhase::Downloading;
  app.show_home_menu();
  assert_eq!(app.host.home_menu_checks, 0);
  app.host.home_menu.phase = HomeMenuPhase::Ready;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 0);
  app.host.update.phase = UpdatePhase::Idle;
  app.host.network_active = true;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 0);
  app.host.network_active = false;
  app.demo = true;
  app.show_home_menu();
  app.dispatch(Action::HomeMenuCheck);
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_checks, 0);
  assert_eq!(app.host.home_menu_installs, 0);
  assert!(app.host.writes.is_empty());
  app.demo = false;
  app.host.home_menu.phase = HomeMenuPhase::Unsupported;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 0);
  app.host.home_menu.phase = HomeMenuPhase::Ready;
  app.ui.page = Page::Chat;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 0, "an old page cannot confirm installation");
}

#[test]
fn home_menu_failures_are_visible_and_retry_does_not_lose_history() {
  let mut app = app();
  let history = serde_json::to_string(&app.core.state().conversations).unwrap();
  app.host.fail_home_menu_check = true;
  app.show_home_menu();
  assert_eq!(app.ui.notice, "HOME Menu check failed.");
  assert_eq!(app.host.home_menu_installs, 0);
  app.host.fail_home_menu_check = false;
  app.dispatch(Action::HomeMenuCheck);
  assert_eq!(app.host.home_menu_checks, 2);
  app.host.home_menu.phase = HomeMenuPhase::Ready;
  app.tick();
  app.host.fail_home_menu_install = true;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.ui.notice, "HOME Menu installation failed.");
  assert_eq!(serde_json::to_string(&app.core.state().conversations).unwrap(), history);
  assert!(app.running);
  app.host.fail_home_menu_install = false;
  app.dispatch(Action::HomeMenuInstall);
  assert_eq!(app.host.home_menu_installs, 2);
}

#[test]
fn home_menu_installation_blocks_exit_navigation_shortcuts_and_duplicate_actions() {
  let mut app = app();
  app.ui.page = Page::HomeMenu;
  app.host.home_menu.phase = HomeMenuPhase::Ready;
  app.ui.draft = "keep this draft".into();
  app.dispatch(Action::HomeMenuInstall);
  let writes = app.host.writes.len();
  let id = app.core.state().active_id.clone();
  app.dispatch(Action::HomeMenuInstall);
  app.dispatch(Action::NewChat);
  app.dispatch(Action::Updates);
  app.dispatch(Action::UpdateCheck);
  app.commit_edit(EditTarget::Setting(Setting::Model), "changed".into());
  app.input(Input::Gamepad(2));
  app.input(Input::Gamepad(4));
  app.input(Input::Gamepad(6));
  app.input(Input::Quit);
  app.input(Input::Key { key: Key::Enter, modifiers: CTRL });
  app.back();
  assert_eq!(app.host.home_menu_installs, 1);
  assert_eq!(app.ui.page, Page::HomeMenu);
  assert_eq!(app.core.state().active_id, id);
  assert_eq!(app.ui.draft, "keep this draft");
  assert_ne!(app.core.state().settings.model, "changed");
  assert_eq!(app.host.writes.len(), writes);
  assert!(app.host.requests.is_empty());
  assert!(app.host.update_checks.is_empty());
  assert!(app.running);
  assert!(app.ui.notice.contains("Keep Wodex open"));
  app.host.home_menu.phase = HomeMenuPhase::Installed;
  app.input(Input::Quit);
  assert!(!app.running);
}

#[test]
fn home_menu_check_blocks_mutation_but_allows_exiting_without_installation() {
  let mut app = app();
  app.show_home_menu();
  app.dispatch(Action::NewChat);
  app.input(Input::Gamepad(4));
  app.back();
  assert_eq!(app.ui.page, Page::HomeMenu);
  assert_eq!(app.core.state().active_id, "chat-1");
  assert_eq!(app.host.home_menu_installs, 0);
  app.input(Input::Gamepad(6));
  assert!(!app.running);
  assert_eq!(app.host.home_menu_installs, 0);
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
fn provider_selection_resets_credentials_and_exposes_only_matching_options() {
  let mut app = App::new(MockHost::default(), "0.2.1", false, false).unwrap();
  app.commit_edit(EditTarget::Setting(Setting::ApiKey), "saved-deepseek-key".into());
  app.edit_setting(Setting::Thinking);
  app.dispatch(Action::Provider(Provider::OpenAiCompatible));
  let settings = &app.core.state().settings;
  assert_eq!(settings.provider, Provider::OpenAiCompatible);
  assert!(settings.api_key.is_empty());
  assert!(settings.model.is_empty());
  assert!(!settings.thinking);
  assert_eq!(settings.base_url, "https://api.openai.com/v1");
  assert_eq!(last_saved(&app)["settings"]["api_key"], "");
  assert_eq!(last_saved(&app)["settings"]["provider"], "openai_compatible");

  let writes = app.host.writes.len();
  app.edit_setting(Setting::Thinking);
  assert_eq!(app.host.writes.len(), writes);
  app.edit_setting(Setting::TokenLimitField);
  assert_eq!(app.core.state().settings.token_limit_field, TokenLimitField::MaxCompletionTokens);
  app.commit_edit(EditTarget::Setting(Setting::Model), "provider-model".into());
  app.commit_edit(EditTarget::Setting(Setting::ApiKey), "saved-compatible-key".into());
  let writes = app.host.writes.len();
  app.dispatch(Action::Provider(Provider::OpenAiCompatible));
  assert_eq!(app.core.state().settings.api_key, "saved-compatible-key");
  assert_eq!(app.core.state().settings.model, "provider-model");
  assert_eq!(app.host.writes.len(), writes);

  app.dispatch(Action::Provider(Provider::DeepSeek));
  let settings = &app.core.state().settings;
  assert!(settings.api_key.is_empty());
  assert_eq!(settings.model, "deepseek-flash");
  assert_eq!(settings.base_url, "https://api.deepseek.com");
  assert_eq!(settings.token_limit_field, TokenLimitField::MaxTokens);
  let writes = app.host.writes.len();
  app.edit_setting(Setting::TokenLimitField);
  assert_eq!(app.host.writes.len(), writes);
  assert!(app.host.requests.is_empty());
}

#[test]
fn provider_controls_remain_reachable_with_controller_navigation() {
  let mut app = app();
  let (_backend, mut canvas) = canvas::test_backend::canvas();
  app.ui.page = Page::Settings;
  app.render(&mut canvas).unwrap();
  app.ui.focused = "provider-deepseek".into();
  app.input(Input::Gamepad(14));
  assert_eq!(app.ui.focused, "provider-openai");
  app.input(Input::Gamepad(0));
  assert_eq!(app.core.state().settings.provider, Provider::OpenAiCompatible);
  assert!(app.buttons.is_empty());
  app.render(&mut canvas).unwrap();
  app.input(Input::Gamepad(12));
  assert_eq!(app.ui.focused, "setting-api_key");
  for expected in [
    "model-setting",
    "setting-base_url",
    "setting-token_limit_field",
    "setting-system_prompt",
    "setting-max_tokens",
  ] {
    app.input(Input::Gamepad(12));
    assert_eq!(app.ui.focused, expected);
  }
  app.ui.focused = "provider-openai".into();
  app.input(Input::Gamepad(11));
  assert_eq!(app.ui.focused, "updates");
  app.input(Input::Gamepad(13));
  assert_eq!(app.ui.focused, "home-menu");
  app.input(Input::Gamepad(13));
  assert_eq!(app.ui.focused, "transfer");
}

#[test]
fn provider_change_is_blocked_during_response_and_active_transfer() {
  let mut app = app();
  app.ui.draft = "question".into();
  app.send();
  app.dispatch(Action::Provider(Provider::OpenAiCompatible));
  assert_eq!(app.core.state().settings.provider, Provider::DeepSeek);
  assert_eq!(app.host.requests.len(), 1);
  app.stop();
  app.host.network_active = false;
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  app.dispatch(Action::Provider(Provider::OpenAiCompatible));
  assert_eq!(app.core.state().settings.provider, Provider::DeepSeek);
}

#[test]
fn environment_credentials_are_isolated_when_switching_providers() {
  for openai_key in ["", "openai-environment-secret"] {
    let host = MockHost {
      environment_key: "deepseek-environment-secret".into(),
      openai_environment_key: openai_key.into(),
      ..Default::default()
    };
    let mut app = App::new(host, "0.2.1", false, false).unwrap();
    assert_eq!(app.core.state().settings.api_key, "deepseek-environment-secret");
    app.dispatch(Action::Provider(Provider::OpenAiCompatible));
    assert_eq!(app.core.state().settings.api_key, openai_key);
    assert_eq!(app.ui.environment_key, !openai_key.is_empty());
    app.commit_edit(EditTarget::Setting(Setting::Model), "provider-model".into());
    app.ui.draft = "question".into();
    app.send();
    if openai_key.is_empty() {
      assert!(app.host.requests.is_empty());
      assert!(!app.streaming());
    } else {
      assert_eq!(app.host.requests[0].1, openai_key);
      app.stop();
      app.host.network_active = false;
    }
    app.dispatch(Action::Provider(Provider::DeepSeek));
    assert_eq!(app.core.state().settings.api_key, "deepseek-environment-secret");
    assert_eq!(last_saved(&app)["settings"]["api_key"], "");
    assert!(app.host.writes.iter().all(|saved| !saved.contains("environment-secret")));
  }
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

fn queue_config_import(app: &mut App<MockHost>, key: &str, model: &str) {
  queue_provider_config_import(app, Provider::DeepSeek, key, model);
}

fn queue_provider_config_import(
  app: &mut App<MockHost>,
  provider: Provider,
  key: &str,
  model: &str,
) {
  let mut source = WodexCore::default();
  source
    .execute(Command::Settings {
      settings: SettingsPatch {
        provider: Some(provider),
        api_key: Some(key.into()),
        model: Some(model.into()),
        ..Default::default()
      },
    })
    .unwrap();
  app.host.transfer_imports.push_back(TransferImport {
    channel: TransferChannel::Config,
    data: source.export_transfer(TransferChannel::Config).unwrap(),
  });
  app.tick();
}

#[test]
fn pc_transfer_is_explicit_and_blocks_demo_busy_save_failed_and_stale_start() {
  let mut app = app();
  app.host.transfer.supported = true;
  app.dispatch(Action::TransferStart);
  assert!(app.host.transfer_exports.is_empty());
  app.show_transfer();
  assert!(app.host.transfer_exports.is_empty());
  app.demo = true;
  app.dispatch(Action::TransferStart);
  app.demo = false;
  app.host.network_active = true;
  app.dispatch(Action::TransferStart);
  app.host.network_active = false;
  app.host.update.phase = UpdatePhase::Downloading;
  app.dispatch(Action::TransferStart);
  app.host.update.phase = UpdatePhase::Idle;
  app.host.fail_save = true;
  app.dispatch(Action::TransferStart);
  assert!(app.host.transfer_exports.is_empty());
  app.host.fail_save = false;
  app.dispatch(Action::TransferStart);
  assert_eq!(app.host.transfer_exports.len(), 1);
  assert!(app.host.transfer.active);
  assert!(app.host.image_cancellations > 0);
  assert!(app.buttons.is_empty());
}

#[test]
fn pc_transfer_exports_saved_key_and_keeps_environment_key_private() {
  let mut original = WodexCore::default();
  original
    .execute(Command::Settings {
      settings: SettingsPatch { api_key: Some("saved-private-key".into()), ..Default::default() },
    })
    .unwrap();
  let saved = original.execute(Command::Export {}).unwrap()["data"].to_string().into_bytes();
  let mut app = App::new(
    MockHost { saved, environment_key: "environment-private-key".into(), ..Default::default() },
    "0.2.1",
    false,
    false,
  )
  .unwrap();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  let export = &app.host.transfer_exports[0];
  assert!(export.config.contains("saved-private-key"));
  assert!(!export.config.contains("environment-private-key"));
  assert!(!export.sessions.contains("private-key"));
  queue_config_import(&mut app, "imported-private-key", "deepseek-reasoner");
  let summary = &app.pending_transfer.as_ref().unwrap().1.summary;
  assert!(!summary.contains("private-key"));
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.core.state().settings.api_key, "environment-private-key");
  assert_eq!(app.stored_key, "imported-private-key");
  assert_eq!(last_saved(&app)["settings"]["api_key"], "imported-private-key");
  assert!(!String::from_utf8_lossy(&app.host.recovery[0]).contains("environment-private-key"));
  assert!(String::from_utf8_lossy(&app.host.recovery[0]).contains("saved-private-key"));
  assert!(app.host.transfer_exports.last().unwrap().config.contains("imported-private-key"));
  assert!(!app.host.transfer_exports.last().unwrap().config.contains("environment-private-key"));
  assert!(app.host.requests.is_empty());
}

#[test]
fn provider_import_uses_matching_environment_key_and_exports_only_saved_key() {
  for openai_key in ["", "openai-environment-secret"] {
    let host = MockHost {
      environment_key: "deepseek-environment-secret".into(),
      openai_environment_key: openai_key.into(),
      ..Default::default()
    };
    let mut app = App::new(host, "0.2.1", false, false).unwrap();
    app.show_transfer();
    app.dispatch(Action::TransferStart);
    queue_provider_config_import(
      &mut app,
      Provider::OpenAiCompatible,
      "imported-saved-key",
      "provider-model",
    );
    app.dispatch(Action::TransferConfirm);
    assert_eq!(app.core.state().settings.provider, Provider::OpenAiCompatible);
    assert_eq!(
      app.core.state().settings.api_key,
      if openai_key.is_empty() { "imported-saved-key" } else { openai_key }
    );
    assert_eq!(last_saved(&app)["settings"]["api_key"], "imported-saved-key");
    let export = &app.host.transfer_exports.last().unwrap().config;
    assert!(export.contains("imported-saved-key"));
    assert!(!export.contains("environment-secret"));
    assert!(app.host.requests.is_empty());
  }
}

#[test]
fn session_import_keeps_a_manually_edited_key_instead_of_reapplying_environment() {
  let mut app = app();
  app.commit_edit(EditTarget::Setting(Setting::ApiKey), "manually-selected-key".into());
  assert!(!app.ui.environment_key);
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  app.host.transfer_imports.push_back(TransferImport {
    channel: TransferChannel::Sessions,
    data: WodexCore::default().export_transfer(TransferChannel::Sessions).unwrap(),
  });
  app.tick();
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.core.state().settings.api_key, "manually-selected-key");
  assert!(!app.ui.environment_key);
  assert_eq!(last_saved(&app)["settings"]["api_key"], "manually-selected-key");
  assert!(!app.host.transfer_exports.last().unwrap().config.contains("test-key"));
}

#[test]
fn pc_config_import_requires_confirmation_and_preserves_sessions() {
  let mut app = image_app("A saved response.");
  let before = serde_json::to_value(&app.core.state().conversations).unwrap();
  let old_model = app.core.state().settings.model.clone();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  let writes = app.host.writes.len();
  queue_config_import(&mut app, "replacement-key", "deepseek-reasoner");
  assert_eq!(app.core.state().settings.model, old_model);
  assert_eq!(app.host.writes.len(), writes);
  assert_eq!(app.ui.focused, "transfer-reject");
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.core.state().settings.model, "deepseek-reasoner");
  assert_eq!(serde_json::to_value(&app.core.state().conversations).unwrap(), before);
  assert_eq!(app.host.writes.len(), writes + 1);
  assert_eq!(app.host.recovery.len(), 1);
  assert!(app.pending_transfer.is_none());
  assert!(app.host.requests.is_empty());
  let writes = app.host.writes.len();
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.host.writes.len(), writes);
}

#[test]
fn pc_import_rejection_and_storage_failures_leave_current_data_unchanged() {
  for failure in 0..3 {
    let mut app = app();
    app.show_transfer();
    app.dispatch(Action::TransferStart);
    let before = app.persisted_content().unwrap();
    let writes = app.host.writes.len();
    queue_config_import(&mut app, "replacement-key", "deepseek-reasoner");
    match failure {
      0 => app.dispatch(Action::TransferReject),
      1 => {
        app.host.fail_preserve = true;
        app.dispatch(Action::TransferConfirm);
      }
      _ => {
        app.host.fail_save = true;
        app.dispatch(Action::TransferConfirm);
      }
    }
    assert_eq!(app.persisted_content().unwrap(), before);
    assert_eq!(app.host.writes.len(), writes);
    assert!(app.pending_transfer.is_none());
    assert_eq!(app.host.transfer_results.len(), 1);
    assert_eq!(app.host.transfer_exports.len(), 1);
  }
}

#[test]
fn pc_sessions_import_preserves_config_and_clears_stale_draft() {
  let mut app = app();
  let before = serde_json::to_value(&app.core.state().settings).unwrap();
  let mut source = WodexCore::default();
  source.execute(Command::NewChat {}).unwrap();
  source.execute(Command::NewChat {}).unwrap();
  app.ui.draft = "Old conversation draft".into();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  app.host.transfer_imports.push_back(TransferImport {
    channel: TransferChannel::Sessions,
    data: source.export_transfer(TransferChannel::Sessions).unwrap(),
  });
  app.tick();
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.core.state().conversations.len(), 3);
  assert_eq!(serde_json::to_value(&app.core.state().settings).unwrap(), before);
  assert!(app.ui.draft.is_empty());
  assert!(app.host.requests.is_empty());
}

#[test]
fn pc_transfer_freezes_mutations_and_leaving_or_exiting_closes_listener() {
  let mut app = app();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  let before = app.persisted_content().unwrap();
  let writes = app.host.writes.len();
  for action in [
    Action::NewChat,
    Action::HomeMenu,
    Action::Updates,
    Action::UpdateCheck,
    Action::UpdateInstall,
    Action::HomeMenuInstall,
    Action::EditSetting(Setting::Model),
    Action::Retry,
    Action::OpenModel,
    Action::BackToChat,
  ] {
    app.dispatch(action);
  }
  app.keyboard(Key::X, 0);
  app.keyboard(Key::RightBracket, 0);
  app.commit_edit(EditTarget::Setting(Setting::Model), "deepseek-reasoner".into());
  assert_eq!(app.persisted_content().unwrap(), before);
  assert_eq!(app.host.writes.len(), writes);
  assert_eq!(app.ui.page, Page::Transfer);
  assert!(app.editor.is_none());
  assert!(app.host.update_checks.is_empty());
  assert_eq!(app.host.home_menu_installs, 0);
  queue_config_import(&mut app, "discarded-key", "deepseek-reasoner");
  app.back();
  assert!(!app.host.transfer.active);
  assert!(app.pending_transfer.is_none());
  assert_eq!(app.ui.page, Page::Settings);
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.host.writes.len(), writes);
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  app.close();
  assert!(!app.host.transfer.active);
  assert!(!app.running());
}

#[test]
fn pc_invalid_import_is_reported_without_showing_payload_or_writing() {
  let mut app = app();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  let writes = app.host.writes.len();
  app.host.transfer_imports.push_back(TransferImport {
    channel: TransferChannel::Config,
    data: "secret-payload-is-not-json".into(),
  });
  app.tick();
  assert!(app.pending_transfer.is_none());
  assert_eq!(app.host.writes.len(), writes);
  assert_eq!(app.host.transfer_results.len(), 1);
  assert!(!app.ui.notice.contains("secret-payload"));
}

#[test]
fn pc_transfer_start_failure_and_expiry_remove_actionable_pending_imports() {
  let mut app = app();
  app.show_transfer();
  app.host.fail_transfer_start = true;
  app.dispatch(Action::TransferStart);
  assert!(!app.host.transfer.active);
  assert!(app.ui.notice.contains("Could not start"));
  app.host.fail_transfer_start = false;
  app.dispatch(Action::TransferStart);
  queue_config_import(&mut app, "replacement-key", "deepseek-reasoner");
  let before = app.persisted_content().unwrap();
  let writes = app.host.writes.len();
  app.host.transfer.active = false;
  app.host.transfer.message = "PC transfer expired. Start a new session to continue.".into();
  app.tick();
  assert!(app.pending_transfer.is_none());
  assert!(app.ui.transfer.summary.is_none());
  assert!(app.buttons.is_empty());
  assert_eq!(app.ui.focused, "transfer-start");
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.host.writes.len(), writes);
  assert_eq!(app.persisted_content().unwrap(), before);
  assert_eq!(app.host.transfer_exports.len(), 1);
}

#[test]
fn pc_import_reconciles_committed_bytes_after_save_reports_directory_sync_failure() {
  let mut app = app();
  app.show_transfer();
  app.dispatch(Action::TransferStart);
  queue_config_import(&mut app, "committed-import-key", "deepseek-reasoner");
  app.host.save_error_after_commit = true;
  app.dispatch(Action::TransferConfirm);
  assert_eq!(app.core.state().settings.model, "deepseek-reasoner");
  assert_eq!(
    app.core.state().settings.api_key,
    "test-key",
    "runtime environment override remains active"
  );
  assert_eq!(app.stored_key, "committed-import-key");
  let persisted: Value = serde_json::from_slice(&app.host.saved).unwrap();
  assert_eq!(persisted["settings"]["api_key"], "committed-import-key");
  assert_eq!(persisted["settings"]["model"], "deepseek-reasoner");
  assert!(app.ui.notice.contains("durability could not be confirmed"));
  assert!(app.host.transfer_results.last().unwrap().contains("durability"));
  assert!(app.host.transfer_exports.last().unwrap().config.contains("committed-import-key"));
  assert_eq!(app.host.recovery.len(), 1);
  assert!(!app.ui.storage_locked);
  app.host.save_error_after_commit = false;
  app.back();
  app.close();
  let persisted: Value = serde_json::from_slice(&app.host.saved).unwrap();
  assert_eq!(persisted["settings"]["api_key"], "committed-import-key");
}

#[test]
fn pc_import_unknown_commit_state_locks_all_writes_until_restart() {
  for read_error in [false, true] {
    let mut app = app();
    app.show_transfer();
    app.dispatch(Action::TransferStart);
    let before = app.persisted_content().unwrap();
    queue_config_import(&mut app, "uncertain-import-key", "deepseek-reasoner");
    app.host.save_error_after_commit = true;
    app.host.fail_load = read_error;
    app.host.corrupt_save_on_error = !read_error;
    app.dispatch(Action::TransferConfirm);
    assert!(app.ui.storage_locked);
    assert!(!app.host.transfer.active);
    assert!(app.pending_transfer.is_none());
    assert!(app.ui.notice.contains("Storage could not be verified"));
    assert!(app.ui.notice.contains("recovery copy"));
    assert!(!app.ui.notice.contains("not applied"));
    assert_eq!(app.persisted_content().unwrap(), before);
    assert_eq!(app.host.recovery.len(), 1);
    assert_eq!(app.host.transfer_exports.len(), 1, "unknown data is never exported");
    let attempts = app.host.save_attempts;
    let unverified = app.host.saved.clone();
    app.host.fail_load = false;
    app.host.save_error_after_commit = false;
    app.host.corrupt_save_on_error = false;
    app.dispatch(Action::NewChat);
    app.dispatch(Action::TransferStart);
    app.commit_edit(EditTarget::Setting(Setting::Model), "changed-after-failure".into());
    app.input(Input::Key { key: Key::Minus, modifiers: 0 });
    app.show_home_menu();
    app.tick();
    let (_backend, mut canvas) = canvas::test_backend::canvas();
    app.render(&mut canvas).unwrap();
    assert!(app.buttons.is_empty(), "only exiting is possible while storage is uncertain");
    assert_eq!(app.persisted_content().unwrap(), before);
    assert!(!app.save());
    app.input(Input::Key { key: Key::Escape, modifiers: 0 });
    assert!(!app.running());
    assert_eq!(
      app.host.save_attempts, attempts,
      "exit and autosave cannot overwrite uncertain data"
    );
    assert_eq!(app.host.saved, unverified);
  }
}
